use crate::{
    CapturedBody, Direction, Error, FlowEvent, FlowId, Headers, RootCa,
    capture::{self, BodyStore, FlowTracker, Tap, TapSink},
    guard::{self, Verdict},
};
use hudsucker::{
    Body, HttpContext, HttpHandler, Proxy, RequestOrResponse,
    certificate_authority::RcgenAuthority,
    hyper::{HeaderMap, Method, Request, Response, StatusCode},
    hyper_util::client::legacy::Error as UpstreamError,
    rustls::crypto::aws_lc_rs,
};
use lantaran_rules::SslProxyingList;
use std::{
    sync::{
        Arc, RwLock,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::SystemTime,
};
use tokio::{net::TcpListener, sync::mpsc};

/// How many generated per-host certificates are kept ready for reuse.
const LEAF_CERT_CACHE: u64 = 1_000;

/// How much of each body is kept by default. The rest still passes through.
pub const DEFAULT_BODY_LIMIT: usize = 5 * 1024 * 1024;

const DIRECT_REQUEST_NOTICE: &str = "Lantaran is running.\n\
Set this address as the HTTP proxy in your device's Wi-Fi settings to capture its traffic.\n";

const PRIVATE_REASON: &str =
    "refused the request: the address is private to the machine running the proxy";
const UNRESOLVED_REASON: &str = "refused the request: the host name could not be resolved";

/// A proxy that reports the traffic passing through it.
pub struct Engine {
    ca: RootCa,
    shared: Arc<Shared>,
}

struct Shared {
    ssl_proxying: RwLock<SslProxyingList>,
    events: mpsc::UnboundedSender<FlowEvent>,
    next_id: AtomicU64,
    bodies: BodyStore,
    body_limit: AtomicUsize,
}

impl Engine {
    /// Creates an engine and the receiving end of its event stream.
    pub fn new(
        ca: RootCa,
        ssl_proxying: SslProxyingList,
    ) -> (Self, mpsc::UnboundedReceiver<FlowEvent>) {
        let (events, receiver) = mpsc::unbounded_channel();
        let shared = Arc::new(Shared {
            ssl_proxying: RwLock::new(ssl_proxying),
            events,
            next_id: AtomicU64::new(1),
            bodies: BodyStore::default(),
            body_limit: AtomicUsize::new(DEFAULT_BODY_LIMIT),
        });

        (Self { ca, shared }, receiver)
    }

    /// Replaces the list of hosts to decrypt. Applies to new connections.
    pub fn set_ssl_proxying(&self, list: SslProxyingList) {
        *self
            .shared
            .ssl_proxying
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = list;
    }

    /// Sets how many bytes of each body are kept. Applies to new flows.
    pub fn set_body_limit(&self, bytes: usize) {
        self.shared.body_limit.store(bytes, Ordering::Relaxed);
    }

    /// The captured body of one side of a flow, once that body has ended.
    pub fn body(&self, id: FlowId, direction: Direction) -> Option<CapturedBody> {
        self.shared.bodies.get(id, direction)
    }

    /// Accepts proxy connections on `listener` until `shutdown` completes.
    pub async fn serve(
        &self,
        listener: TcpListener,
        shutdown: impl Future<Output = ()> + Send + 'static,
    ) -> Result<(), Error> {
        let provider = aws_lc_rs::default_provider();
        let authority = RcgenAuthority::new(self.ca.issuer()?, LEAF_CERT_CACHE, provider.clone());

        let proxy = Proxy::builder()
            .with_listener(listener)
            .with_ca(authority)
            .with_rustls_connector(provider)
            .with_http_handler(Handler {
                shared: Arc::clone(&self.shared),
                pending: None,
            })
            .with_graceful_shutdown(shutdown)
            .build()?;

        Ok(proxy.start().await?)
    }
}

impl Shared {
    fn next_id(&self) -> FlowId {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    fn should_decrypt(&self, host: &str) -> bool {
        self.ssl_proxying
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .should_decrypt(host)
    }

    fn body_limit(&self) -> usize {
        self.body_limit.load(Ordering::Relaxed)
    }
}

impl TapSink for Shared {
    fn store(&self) -> &BodyStore {
        &self.bodies
    }

    fn emit(&self, event: FlowEvent) {
        // Nobody listening is not an error: the proxy keeps forwarding traffic.
        let _ = self.events.send(event);
    }
}

/// hudsucker clones the handler for each request and calls `handle_request`
/// and then `handle_response` or `handle_error` on that same clone, so
/// `pending` links a response to its request.
#[derive(Clone)]
struct Handler {
    shared: Arc<Shared>,
    pending: Option<Arc<FlowTracker>>,
}

impl HttpHandler for Handler {
    async fn handle_request(&mut self, ctx: &HttpContext, req: Request<Body>) -> RequestOrResponse {
        // No host in the URL means the client is talking to the proxy itself,
        // for example a browser opened on the proxy address.
        let Some(destination) = req.uri().authority() else {
            return Response::new(Body::from(DIRECT_REQUEST_NOTICE)).into();
        };

        // Checked on every request, not only on CONNECT: an HTTP/2 request
        // inside a decrypted connection names its own destination.
        let refusal = match guard::check(ctx.client_addr.ip(), destination).await {
            Verdict::Allow => None,
            Verdict::Private => Some((StatusCode::FORBIDDEN, PRIVATE_REASON)),
            Verdict::Unresolved => Some((StatusCode::BAD_GATEWAY, UNRESOLVED_REASON)),
        };
        if let Some((status, reason)) = refusal {
            let id = self.shared.next_id();
            self.shared.emit(request_event(id, ctx, &req));
            self.shared.emit(FlowEvent::Failed {
                id,
                error: reason.to_owned(),
            });

            let mut res = Response::new(Body::from(format!("Lantaran {reason}\n")));
            *res.status_mut() = status;
            return res.into();
        }

        // CONNECT only opens a connection. What it carries is reported later,
        // either as decrypted requests or as a tunnel.
        if req.method() == Method::CONNECT {
            return req.into();
        }

        let flow = FlowTracker::new(self.shared.next_id());
        self.shared.emit(request_event(flow.id, ctx, &req));

        let (parts, body) = req.into_parts();
        let body = Tap::wrap(
            body,
            &parts.headers,
            Arc::clone(&self.shared),
            Arc::clone(&flow),
            Direction::Request,
            self.shared.body_limit(),
        );
        self.pending = Some(flow);

        Request::from_parts(parts, body).into()
    }

    async fn handle_response(&mut self, _ctx: &HttpContext, res: Response<Body>) -> Response<Body> {
        let Some(flow) = self.pending.take() else {
            return res;
        };

        self.shared.emit(FlowEvent::Response {
            id: flow.id,
            status: res.status().as_u16(),
            version: format!("{:?}", res.version()),
            headers: headers(res.headers()),
            elapsed: flow.started.elapsed(),
        });

        let (parts, body) = res.into_parts();
        let body = Tap::wrap(
            body,
            &parts.headers,
            Arc::clone(&self.shared),
            flow,
            Direction::Response,
            self.shared.body_limit(),
        );
        Response::from_parts(parts, body)
    }

    async fn handle_error(&mut self, _ctx: &HttpContext, err: UpstreamError) -> Response<Body> {
        let reason = capture::describe(&err);
        if let Some(flow) = self.pending.take() {
            self.shared.emit(FlowEvent::Failed {
                id: flow.id,
                error: reason.clone(),
            });
        }

        let mut res = Response::new(Body::from(format!(
            "Lantaran could not reach the server: {reason}\n"
        )));
        *res.status_mut() = StatusCode::BAD_GATEWAY;
        res
    }

    async fn should_intercept_connect(&mut self, ctx: &HttpContext, req: &Request<Body>) -> bool {
        let Some(authority) = req.uri().authority() else {
            return false;
        };

        if self.shared.should_decrypt(authority.host()) {
            return true;
        }

        self.shared.emit(FlowEvent::Tunnel {
            id: self.shared.next_id(),
            client: ctx.client_addr,
            authority: authority.to_string(),
        });
        false
    }
}

fn request_event(id: FlowId, ctx: &HttpContext, req: &Request<Body>) -> FlowEvent {
    FlowEvent::Request {
        id,
        client: ctx.client_addr,
        started_at: SystemTime::now(),
        method: req.method().to_string(),
        url: req.uri().to_string(),
        version: format!("{:?}", req.version()),
        headers: headers(req.headers()),
    }
}

fn headers(map: &HeaderMap) -> Headers {
    map.iter()
        .map(|(name, value)| {
            (
                name.as_str().to_owned(),
                String::from_utf8_lossy(value.as_bytes()).into_owned(),
            )
        })
        .collect()
}
