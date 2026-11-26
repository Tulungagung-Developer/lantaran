use crate::{Error, FlowEvent, FlowId, RootCa, guard};
use hudsucker::{
    Body, HttpContext, HttpHandler, Proxy, RequestOrResponse,
    certificate_authority::RcgenAuthority,
    hyper::{Method, Request, Response, StatusCode, header::CONTENT_LENGTH},
    hyper_util::client::legacy::Error as UpstreamError,
    rustls::crypto::aws_lc_rs,
};
use lantaran_rules::SslProxyingList;
use std::{
    sync::{
        Arc, RwLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};
use tokio::{net::TcpListener, sync::mpsc};

/// How many generated per-host certificates are kept ready for reuse.
const LEAF_CERT_CACHE: u64 = 1_000;

const DIRECT_REQUEST_NOTICE: &str = "Lantaran is running.\n\
Set this address as the HTTP proxy in your device's Wi-Fi settings to capture its traffic.\n";

const REFUSED_NOTICE: &str = "Lantaran refused this request: other devices may not reach \
addresses that are private to the machine running the proxy.\n";
const REFUSED_REASON: &str = "refused: the address is private to the machine running the proxy";

/// A proxy that reports the traffic passing through it.
pub struct Engine {
    ca: RootCa,
    shared: Arc<Shared>,
}

struct Shared {
    ssl_proxying: RwLock<SslProxyingList>,
    events: mpsc::UnboundedSender<FlowEvent>,
    next_id: AtomicU64,
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

    fn emit(&self, event: FlowEvent) {
        // Nobody listening is not an error: the proxy keeps forwarding traffic.
        let _ = self.events.send(event);
    }

    fn should_decrypt(&self, host: &str) -> bool {
        self.ssl_proxying
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .should_decrypt(host)
    }
}

/// hudsucker clones the handler for each request and calls `handle_request`
/// and then `handle_response` or `handle_error` on that same clone, so
/// `pending` links a response to its request.
#[derive(Clone)]
struct Handler {
    shared: Arc<Shared>,
    pending: Option<(FlowId, Instant)>,
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
        if guard::refuses(ctx.client_addr.ip(), destination).await {
            let id = self.shared.next_id();
            self.shared.emit(FlowEvent::Request {
                id,
                client: ctx.client_addr,
                method: req.method().to_string(),
                url: req.uri().to_string(),
            });
            self.shared.emit(FlowEvent::Failed {
                id,
                error: REFUSED_REASON.to_owned(),
            });

            let mut res = Response::new(Body::from(REFUSED_NOTICE));
            *res.status_mut() = StatusCode::FORBIDDEN;
            return res.into();
        }

        // CONNECT only opens a connection. What it carries is reported later,
        // either as decrypted requests or as a tunnel.
        if req.method() == Method::CONNECT {
            return req.into();
        }

        let id = self.shared.next_id();
        self.pending = Some((id, Instant::now()));
        self.shared.emit(FlowEvent::Request {
            id,
            client: ctx.client_addr,
            method: req.method().to_string(),
            url: req.uri().to_string(),
        });

        req.into()
    }

    async fn handle_response(&mut self, _ctx: &HttpContext, res: Response<Body>) -> Response<Body> {
        if let Some((id, started)) = self.pending.take() {
            let content_length = res
                .headers()
                .get(CONTENT_LENGTH)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse().ok());

            self.shared.emit(FlowEvent::Response {
                id,
                status: res.status().as_u16(),
                elapsed: started.elapsed(),
                content_length,
            });
        }

        res
    }

    async fn handle_error(&mut self, _ctx: &HttpContext, err: UpstreamError) -> Response<Body> {
        if let Some((id, _)) = self.pending.take() {
            self.shared.emit(FlowEvent::Failed {
                id,
                error: describe(&err),
            });
        }

        let mut res = Response::new(Body::from(format!(
            "Lantaran could not reach the server: {}\n",
            describe(&err)
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

/// Joins an error with its causes, since the top-level message of an upstream
/// failure is only "client error (Connect)".
fn describe(err: &(dyn std::error::Error + 'static)) -> String {
    let mut text = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}
