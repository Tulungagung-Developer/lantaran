//! Bindings that let the desktop apps drive the engine.
//!
//! The interface is message-shaped on purpose: the app calls methods on one
//! [`LantaranEngine`] object and receives flow events in batches through one
//! [`EventListener`]. Nothing mutable is shared across the boundary, so the
//! same interface works for Swift today and C# later.

use lantaran_engine::{
    CapturedBody, Direction, Engine, EventStream, FlowEvent, HostPattern, MapLocal, MapRemote,
    MapRules, RootCa, SslProxyingList, UrlPattern,
};
use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, UNIX_EPOCH},
};
use tokio::{net::TcpListener, runtime::Runtime, sync::oneshot};

uniffi::setup_scaffolding!();

#[derive(Debug, thiserror::Error, uniffi::Error)]
#[uniffi(flat_error)]
pub enum LantaranError {
    #[error("{0}")]
    Failed(String),
}

impl LantaranError {
    fn from(error: impl std::fmt::Display) -> Self {
        Self::Failed(error.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Header {
    pub name: String,
    pub value: String,
}

/// A [`FlowEvent`] in a form every language can read. Times are in
/// milliseconds; `started_at` is seconds since 1970.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum FlowEventRecord {
    Request {
        id: u64,
        client: String,
        /// The client's source port, which tells connections from the same
        /// machine apart.
        client_port: u16,
        started_at: f64,
        method: String,
        url: String,
        version: String,
        headers: Vec<Header>,
    },
    Response {
        id: u64,
        status: u16,
        version: String,
        headers: Vec<Header>,
        elapsed_ms: f64,
        unverified_upstream: bool,
    },
    Completed {
        id: u64,
        request_size: u64,
        response_size: u64,
        duration_ms: f64,
    },
    Failed {
        id: u64,
        error: String,
    },
    Rewritten {
        id: u64,
        description: String,
    },
    Tunnel {
        id: u64,
        client: String,
        client_port: u16,
        authority: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct BodyRecord {
    /// As it travelled, possibly compressed, up to the capture limit.
    pub bytes: Vec<u8>,
    pub size: u64,
    pub truncated: bool,
    pub incomplete: bool,
    pub content_encoding: Option<String>,
}

/// A Map Local rule: answer matching requests from `file`.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MapLocalRule {
    pub pattern: String,
    pub method: Option<String>,
    pub file: String,
    pub status: u16,
    pub content_type: Option<String>,
}

/// A Map Remote rule: each part that is set replaces that part of the URL.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MapRemoteRule {
    pub pattern: String,
    pub method: Option<String>,
    pub scheme: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub path: Option<String>,
}

/// Receives events. Called on a background thread; implementations must hand
/// the work to their UI thread themselves.
#[uniffi::export(with_foreign)]
pub trait EventListener: Send + Sync {
    fn on_events(&self, events: Vec<FlowEventRecord>, dropped: u64);
}

#[derive(uniffi::Object)]
pub struct LantaranEngine {
    runtime: Runtime,
    engine: Arc<Engine>,
    data_dir: PathBuf,
    events: Mutex<Option<EventStream>>,
    stop: Mutex<Option<oneshot::Sender<()>>>,
}

#[uniffi::export]
impl LantaranEngine {
    /// Loads or creates the root certificate in `data_dir`.
    #[uniffi::constructor]
    pub fn new(data_dir: String) -> Result<Arc<Self>, LantaranError> {
        let data_dir = PathBuf::from(data_dir);
        let ca = RootCa::load_or_create(&data_dir).map_err(LantaranError::from)?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .thread_name("lantaran-engine")
            .enable_all()
            .build()
            .map_err(LantaranError::from)?;
        let (engine, events) = Engine::new(ca, SslProxyingList::new());

        Ok(Arc::new(Self {
            runtime,
            engine: Arc::new(engine),
            data_dir,
            events: Mutex::new(Some(events)),
            stop: Mutex::new(None),
        }))
    }

    /// Starts delivering events to `listener`. Only the first call has an
    /// effect: there is one event stream per engine.
    pub fn set_listener(&self, listener: Arc<dyn EventListener>) {
        let Some(mut events) = lock(&self.events).take() else {
            return;
        };
        self.runtime.spawn(async move {
            while let Some(batch) = events.next_batch().await {
                let records = batch
                    .events
                    .into_iter()
                    .map(FlowEventRecord::from)
                    .collect();
                listener.on_events(records, batch.dropped);
            }
        });
    }

    /// Starts accepting proxy connections on `address`, such as
    /// `0.0.0.0:9090`. Returns the address actually bound. Stops a previous
    /// listener first.
    pub fn start(&self, address: String) -> Result<String, LantaranError> {
        self.stop();

        let address: SocketAddr = address
            .parse()
            .map_err(|_| LantaranError::Failed(format!("{address} is not an address and port")))?;
        let listener = self
            .runtime
            .block_on(TcpListener::bind(address))
            .map_err(|e| LantaranError::Failed(format!("could not listen on {address}: {e}")))?;
        let bound = listener.local_addr().map_err(LantaranError::from)?;

        let (stop, stopped) = oneshot::channel::<()>();
        *lock(&self.stop) = Some(stop);
        let engine = Arc::clone(&self.engine);
        self.runtime.spawn(async move {
            let _ = engine
                .serve(listener, async {
                    let _ = stopped.await;
                })
                .await;
        });

        Ok(bound.to_string())
    }

    /// Stops accepting connections. Connections already open finish.
    pub fn stop(&self) {
        if let Some(stop) = lock(&self.stop).take() {
            let _ = stop.send(());
        }
    }

    pub fn is_running(&self) -> bool {
        lock(&self.stop)
            .as_ref()
            .is_some_and(|stop| !stop.is_closed())
    }

    /// Replaces the hosts to decrypt. Applies to new connections.
    pub fn set_ssl_proxying(&self, include: Vec<String>, exclude: Vec<String>) {
        let list = include
            .iter()
            .fold(SslProxyingList::new(), |list, pattern| {
                list.include(pattern.as_str())
            });
        let list = exclude
            .iter()
            .fold(list, |list, pattern| list.exclude(pattern.as_str()));
        self.engine.set_ssl_proxying(list);
    }

    /// Replaces the hosts whose server certificates are not checked.
    pub fn set_exempt_hosts(&self, hosts: Vec<String>) {
        self.engine
            .set_exempt_hosts(hosts.iter().map(|h| HostPattern::new(h.as_str())).collect());
    }

    /// Replaces the Map Local and Map Remote rules, checked in order.
    pub fn set_map_rules(&self, local: Vec<MapLocalRule>, remote: Vec<MapRemoteRule>) {
        let blank = |value: Option<String>| value.filter(|v| !v.trim().is_empty());
        self.engine.set_map_rules(MapRules {
            local: local
                .into_iter()
                .map(|rule| MapLocal {
                    pattern: UrlPattern::new(rule.pattern),
                    method: blank(rule.method),
                    file: rule.file,
                    status: rule.status,
                    content_type: blank(rule.content_type),
                })
                .collect(),
            remote: remote
                .into_iter()
                .map(|rule| MapRemote {
                    pattern: UrlPattern::new(rule.pattern),
                    method: blank(rule.method),
                    scheme: blank(rule.scheme),
                    host: blank(rule.host),
                    port: rule.port,
                    path: blank(rule.path),
                })
                .collect(),
        });
    }

    /// One side of a flow as captured, once that body has ended.
    pub fn body(&self, id: u64, response: bool) -> Option<BodyRecord> {
        self.engine
            .body(id, direction(response))
            .map(BodyRecord::from)
    }

    /// One side of a flow with its compression undone, for display. `None`
    /// when there is no body or it cannot be decoded.
    pub fn decoded_body(&self, id: u64, response: bool) -> Option<Vec<u8>> {
        let body = self.engine.body(id, direction(response))?;
        self.runtime
            .block_on(body.decoded())
            .ok()
            .map(|bytes| bytes.to_vec())
    }

    pub fn certificate_pem(&self) -> String {
        self.engine.root_ca().cert_pem().to_owned()
    }

    /// The certificate in DER form, for a `.crt` file.
    pub fn certificate_der(&self) -> Vec<u8> {
        self.engine.root_ca().cert_der()
    }

    pub fn certificate_path(&self) -> String {
        self.engine.root_ca().cert_path().display().to_string()
    }

    /// The name devices show for the certificate.
    pub fn certificate_name(&self) -> String {
        self.engine.root_ca().name()
    }

    /// The certificate and its private key as a PKCS#12 file protected by
    /// `password`, for moving this setup to another machine.
    pub fn export_pkcs12(&self, password: String) -> Result<Vec<u8>, LantaranError> {
        self.engine
            .root_ca()
            .to_pkcs12(&password)
            .map_err(LantaranError::from)
    }

    /// Replaces the root certificate and key with the ones in a PKCS#12
    /// file. Restart a running proxy afterwards so it signs with them.
    pub fn import_pkcs12(&self, data: Vec<u8>, password: String) -> Result<(), LantaranError> {
        let ca =
            RootCa::import_pkcs12(&self.data_dir, &data, &password).map_err(LantaranError::from)?;
        self.engine.set_root_ca(ca);
        Ok(())
    }

    /// Creates a new root certificate and key. Every device that trusted the
    /// old one has to be set up again. Restart a running proxy afterwards.
    pub fn reset_certificate(&self) -> Result<(), LantaranError> {
        let ca = RootCa::reset(&self.data_dir).map_err(LantaranError::from)?;
        self.engine.set_root_ca(ca);
        Ok(())
    }
}

impl Drop for LantaranEngine {
    fn drop(&mut self) {
        self.stop();
    }
}

/// This machine's address for phones on the same network, if it has one.
#[uniffi::export]
pub fn local_network_address() -> Option<String> {
    lantaran_engine::local_network_address().map(|ip| ip.to_string())
}

/// The host name that opens the certificate page through the proxy.
#[uniffi::export]
pub fn onboarding_host() -> String {
    lantaran_engine::ONBOARDING_HOST.to_owned()
}

fn direction(response: bool) -> Direction {
    if response {
        Direction::Response
    } else {
        Direction::Request
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

fn headers(headers: Vec<(String, String)>) -> Vec<Header> {
    headers
        .into_iter()
        .map(|(name, value)| Header { name, value })
        .collect()
}

impl From<FlowEvent> for FlowEventRecord {
    fn from(event: FlowEvent) -> Self {
        match event {
            FlowEvent::Request {
                id,
                client,
                started_at,
                method,
                url,
                version,
                headers: h,
            } => Self::Request {
                id,
                client: client.ip().to_string(),
                client_port: client.port(),
                started_at: started_at
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs_f64(),
                method,
                url,
                version,
                headers: headers(h),
            },
            FlowEvent::Response {
                id,
                status,
                version,
                headers: h,
                elapsed,
                unverified_upstream,
            } => Self::Response {
                id,
                status,
                version,
                headers: headers(h),
                elapsed_ms: millis(elapsed),
                unverified_upstream,
            },
            FlowEvent::Completed {
                id,
                request_size,
                response_size,
                duration,
            } => Self::Completed {
                id,
                request_size,
                response_size,
                duration_ms: millis(duration),
            },
            FlowEvent::Failed { id, error } => Self::Failed { id, error },
            FlowEvent::Rewritten { id, description } => Self::Rewritten { id, description },
            FlowEvent::Tunnel {
                id,
                client,
                authority,
            } => Self::Tunnel {
                id,
                client: client.ip().to_string(),
                client_port: client.port(),
                authority,
            },
        }
    }
}

impl From<CapturedBody> for BodyRecord {
    fn from(body: CapturedBody) -> Self {
        Self {
            bytes: body.bytes.to_vec(),
            size: body.size,
            truncated: body.truncated,
            incomplete: body.incomplete,
            content_encoding: body.content_encoding,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpStream,
        sync::mpsc,
    };

    struct Collect(Mutex<mpsc::Sender<Vec<FlowEventRecord>>>);

    impl EventListener for Collect {
        fn on_events(&self, events: Vec<FlowEventRecord>, _dropped: u64) {
            let _ = lock(&self.0).send(events);
        }
    }

    #[test]
    fn start_reports_the_bound_address_and_can_restart() {
        let dir = tempfile::tempdir().unwrap();
        let engine = LantaranEngine::new(dir.path().display().to_string()).unwrap();

        let first = engine.start("127.0.0.1:0".to_owned()).unwrap();
        assert!(engine.is_running());
        engine.stop();
        assert!(!engine.is_running());

        let second = engine.start("127.0.0.1:0".to_owned()).unwrap();
        assert!(second.starts_with("127.0.0.1:"));
        assert_ne!(first, "");
        engine.stop();
    }

    #[test]
    fn a_bad_address_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let engine = LantaranEngine::new(dir.path().display().to_string()).unwrap();

        assert!(engine.start("not an address".to_owned()).is_err());
    }

    #[test]
    fn events_reach_the_listener_as_records() {
        let dir = tempfile::tempdir().unwrap();
        let engine = LantaranEngine::new(dir.path().display().to_string()).unwrap();
        let (sender, receiver) = mpsc::channel();
        engine.set_listener(Arc::new(Collect(Mutex::new(sender))));
        let address = engine.start("127.0.0.1:0".to_owned()).unwrap();

        // A request to an address that refuses connections fails without
        // needing a server.
        let closed = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let mut client = TcpStream::connect(&address).unwrap();
        write!(
            client,
            "GET http://{closed}/ HTTP/1.1\r\nHost: {closed}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut reply = String::new();
        client.read_to_string(&mut reply).unwrap();

        let mut seen = Vec::new();
        while seen.len() < 2 {
            seen.extend(receiver.recv_timeout(Duration::from_secs(5)).unwrap());
        }
        assert!(
            matches!(&seen[0], FlowEventRecord::Request { method, client_port, .. } if method == "GET" && *client_port != 0)
        );
        assert!(matches!(&seen[1], FlowEventRecord::Failed { .. }));
        engine.stop();
    }

    #[test]
    fn the_certificate_is_available() {
        let dir = tempfile::tempdir().unwrap();
        let engine = LantaranEngine::new(dir.path().display().to_string()).unwrap();

        assert!(
            engine
                .certificate_pem()
                .starts_with("-----BEGIN CERTIFICATE-----")
        );
        assert!(engine.certificate_path().ends_with("lantaran-ca.pem"));
        assert_eq!(engine.certificate_der().first(), Some(&0x30));
        assert!(engine.certificate_name().starts_with("Lantaran Root CA"));
    }

    #[test]
    fn reset_and_import_replace_the_certificate() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let a = LantaranEngine::new(first.path().display().to_string()).unwrap();
        let b = LantaranEngine::new(second.path().display().to_string()).unwrap();
        let original = a.certificate_pem();

        let file = a.export_pkcs12("pw".to_owned()).unwrap();
        assert!(b.import_pkcs12(file.clone(), "nope".to_owned()).is_err());
        b.import_pkcs12(file, "pw".to_owned()).unwrap();
        assert_eq!(b.certificate_pem(), original);

        a.reset_certificate().unwrap();
        assert_ne!(a.certificate_pem(), original);
    }
}
