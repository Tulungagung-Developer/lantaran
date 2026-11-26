//! Drives a running engine over real sockets, with local stand-ins for the
//! upstream server so the tests need no network.

use flate2::{Compression, write::GzEncoder};
use lantaran_engine::{
    CapturedBody, Direction, Engine, EventStream, FlowEvent, HostPattern, MAX_DECODED_SIZE,
    MapLocal, MapRemote, MapRules, RootCa, SslProxyingList, UrlPattern,
};
use std::{
    collections::VecDeque, future::pending, io::Write, net::SocketAddr, sync::Arc, time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::timeout,
};
use tokio_rustls::{
    TlsAcceptor, TlsConnector,
    rustls::{
        ClientConfig, RootCertStore, ServerConfig,
        crypto::aws_lc_rs,
        pki_types::{CertificateDer, PrivateKeyDer, ServerName, pem::PemObject},
    },
};

const WAIT: Duration = Duration::from_secs(5);

/// An upstream server that hands every connection to `serve`.
async fn start_origin<F, Fut>(serve: F) -> SocketAddr
where
    F: Fn(TcpStream) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            tokio::spawn(serve(stream));
        }
    });

    addr
}

/// An upstream server that reads one request and sends `response` as is.
async fn start_fixed_origin(response: Vec<u8>) -> SocketAddr {
    let response = Arc::new(response);
    start_origin(move |mut stream| {
        let response = Arc::clone(&response);
        async move {
            read_request(&mut stream).await;
            stream.write_all(&response).await.unwrap();
        }
    })
    .await
}

struct Proxy {
    addr: SocketAddr,
    events: Events,
    engine: Arc<Engine>,
    /// The engine's root certificate, for clients that need to trust it.
    ca_pem: String,
}

/// Hands out events one at a time from the engine's batches.
struct Events {
    stream: EventStream,
    pending: VecDeque<FlowEvent>,
}

impl Events {
    fn try_recv(&mut self) -> Option<FlowEvent> {
        if let Some(event) = self.pending.pop_front() {
            return Some(event);
        }
        self.pending.extend(self.stream.try_next_batch()?.events);
        self.pending.pop_front()
    }
}

async fn start_proxy(ssl_proxying: SslProxyingList) -> Proxy {
    let dir = tempfile::tempdir().unwrap();
    let ca = RootCa::load_or_create(dir.path()).unwrap();
    let ca_pem = ca.cert_pem().to_owned();
    let (engine, events) = Engine::new(ca, ssl_proxying);
    let engine = Arc::new(engine);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let serving = Arc::clone(&engine);
    tokio::spawn(async move { serving.serve(listener, pending()).await });

    Proxy {
        addr,
        events: Events {
            stream: events,
            pending: VecDeque::new(),
        },
        engine,
        ca_pem,
    }
}

/// Reads up to the blank line that ends an HTTP head.
async fn read_head(stream: &mut (impl AsyncRead + Unpin)) -> String {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        let n = timeout(WAIT, stream.read(&mut byte))
            .await
            .unwrap()
            .unwrap();
        assert!(n > 0, "connection closed before the head was complete");
        head.push(byte[0]);
    }
    String::from_utf8(head).unwrap()
}

/// Reads a request head and, if it announces one, its body.
async fn read_request(stream: &mut (impl AsyncRead + Unpin)) -> (String, Vec<u8>) {
    let head = read_head(stream).await;
    let length = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().unwrap())
        })
        .unwrap_or(0);

    let mut body = vec![0; length];
    timeout(WAIT, stream.read_exact(&mut body))
        .await
        .unwrap()
        .unwrap();
    (head, body)
}

async fn read_to_end(stream: &mut (impl AsyncRead + Unpin)) -> Vec<u8> {
    let mut out = Vec::new();
    timeout(WAIT, stream.read_to_end(&mut out))
        .await
        .unwrap()
        .unwrap();
    out
}

async fn send(proxy: SocketAddr, request: String) -> String {
    let mut client = TcpStream::connect(proxy).await.unwrap();
    client.write_all(request.as_bytes()).await.unwrap();
    String::from_utf8_lossy(&read_to_end(&mut client).await).into_owned()
}

async fn next_event(events: &mut Events) -> FlowEvent {
    while events.pending.is_empty() {
        let batch = timeout(WAIT, events.stream.next_batch())
            .await
            .expect("no event arrived")
            .expect("the event stream ended");
        events.pending.extend(batch.events);
    }
    events.pending.pop_front().unwrap()
}

fn get(origin: SocketAddr, path: &str) -> String {
    format!("GET http://{origin}{path} HTTP/1.1\r\nHost: {origin}\r\nConnection: close\r\n\r\n")
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

fn gzip(data: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data).unwrap();
    encoder.finish().unwrap()
}

#[tokio::test]
async fn plain_http_request_is_forwarded_and_reported() {
    let origin = start_fixed_origin(
        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nX-Origin: yes\r\nConnection: close\r\n\r\nhello"
            .to_vec(),
    )
    .await;
    let mut proxy = start_proxy(SslProxyingList::new()).await;

    let reply = send(proxy.addr, get(origin, "/hello")).await;
    assert!(
        reply.starts_with("HTTP/1.1 200"),
        "unexpected reply: {reply}"
    );
    assert!(reply.ends_with("hello"), "unexpected reply: {reply}");

    let FlowEvent::Request {
        id,
        method,
        url,
        version,
        headers,
        ..
    } = next_event(&mut proxy.events).await
    else {
        panic!("expected a request event first");
    };
    assert_eq!(method, "GET");
    assert_eq!(url, format!("http://{origin}/hello"));
    assert_eq!(version, "HTTP/1.1");
    assert_eq!(header(&headers, "host"), Some(origin.to_string().as_str()));

    let FlowEvent::Response {
        id: response_id,
        status,
        headers,
        ..
    } = next_event(&mut proxy.events).await
    else {
        panic!("expected a response event second");
    };
    assert_eq!(response_id, id);
    assert_eq!(status, 200);
    assert_eq!(header(&headers, "x-origin"), Some("yes"));

    let FlowEvent::Completed {
        id: completed_id,
        request_size,
        response_size,
        ..
    } = next_event(&mut proxy.events).await
    else {
        panic!("expected a completed event third");
    };
    assert_eq!(completed_id, id);
    assert_eq!(request_size, 0);
    assert_eq!(response_size, 5);

    let body = proxy.engine.body(id, Direction::Response).unwrap();
    assert_eq!(&body.bytes[..], b"hello");
    assert!(!body.truncated && !body.incomplete);
}

#[tokio::test]
async fn request_body_is_forwarded_and_captured() {
    let origin = start_origin(|mut stream| async move {
        let (_, body) = read_request(&mut stream).await;
        let reply = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(reply.as_bytes()).await.unwrap();
        stream.write_all(&body).await.unwrap();
    })
    .await;
    let mut proxy = start_proxy(SslProxyingList::new()).await;

    let reply = send(
        proxy.addr,
        format!(
            "POST http://{origin}/echo HTTP/1.1\r\nHost: {origin}\r\nContent-Length: 11\r\n\
             Connection: close\r\n\r\nname=lantar"
        ),
    )
    .await;
    assert!(reply.ends_with("name=lantar"), "unexpected reply: {reply}");

    let FlowEvent::Request { id, .. } = next_event(&mut proxy.events).await else {
        panic!("expected a request event first");
    };
    next_event(&mut proxy.events).await;
    let FlowEvent::Completed { request_size, .. } = next_event(&mut proxy.events).await else {
        panic!("expected a completed event");
    };
    assert_eq!(request_size, 11);

    let body = proxy.engine.body(id, Direction::Request).unwrap();
    assert_eq!(&body.bytes[..], b"name=lantar");
}

#[tokio::test]
async fn chunked_response_is_captured_whole() {
    let origin = start_fixed_origin(
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n\
          4\r\nwiki\r\n5\r\npedia\r\n0\r\n\r\n"
            .to_vec(),
    )
    .await;
    let mut proxy = start_proxy(SslProxyingList::new()).await;

    let reply = send(proxy.addr, get(origin, "/")).await;
    assert!(
        reply.starts_with("HTTP/1.1 200"),
        "unexpected reply: {reply}"
    );

    let FlowEvent::Request { id, .. } = next_event(&mut proxy.events).await else {
        panic!("expected a request event first");
    };
    next_event(&mut proxy.events).await;
    let FlowEvent::Completed { response_size, .. } = next_event(&mut proxy.events).await else {
        panic!("expected a completed event");
    };
    assert_eq!(response_size, 9);
    assert_eq!(
        &proxy.engine.body(id, Direction::Response).unwrap().bytes[..],
        b"wikipedia"
    );
}

#[tokio::test]
async fn compressed_response_reaches_the_client_compressed_and_decodes_for_display() {
    let text = "a body that compresses well, well, well, well, well".repeat(20);
    let compressed = gzip(text.as_bytes());
    let mut response = format!(
        "HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        compressed.len()
    )
    .into_bytes();
    response.extend_from_slice(&compressed);
    let origin = start_fixed_origin(response).await;
    let mut proxy = start_proxy(SslProxyingList::new()).await;

    let mut client = TcpStream::connect(proxy.addr).await.unwrap();
    client.write_all(get(origin, "/").as_bytes()).await.unwrap();
    let reply = read_to_end(&mut client).await;
    assert!(
        reply.ends_with(&compressed),
        "the client must receive the original encoding"
    );

    let FlowEvent::Request { id, .. } = next_event(&mut proxy.events).await else {
        panic!("expected a request event first");
    };
    next_event(&mut proxy.events).await;
    next_event(&mut proxy.events).await;

    let body = proxy.engine.body(id, Direction::Response).unwrap();
    assert_eq!(&body.bytes[..], &compressed[..]);
    assert_eq!(body.content_encoding.as_deref(), Some("gzip"));
    assert_eq!(&body.decoded().await.unwrap()[..], text.as_bytes());
}

#[tokio::test]
async fn decoding_stops_at_the_size_limit() {
    // A few kilobytes of gzip that would inflate past the limit.
    let bomb = gzip(&vec![0; MAX_DECODED_SIZE + 1]);
    let body = CapturedBody {
        size: bomb.len() as u64,
        bytes: bomb.into(),
        truncated: false,
        incomplete: false,
        content_encoding: Some("gzip".to_owned()),
    };

    assert!(body.decoded().await.is_err());
}

#[tokio::test]
async fn body_beyond_the_limit_passes_through_but_is_not_kept() {
    let payload = vec![b'x'; 100_000];
    let mut response = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    )
    .into_bytes();
    response.extend_from_slice(&payload);
    let origin = start_fixed_origin(response).await;
    let mut proxy = start_proxy(SslProxyingList::new()).await;
    proxy.engine.set_body_limit(1_000);

    let mut client = TcpStream::connect(proxy.addr).await.unwrap();
    client
        .write_all(get(origin, "/big").as_bytes())
        .await
        .unwrap();
    let reply = read_to_end(&mut client).await;
    assert!(reply.ends_with(&payload), "the client must get every byte");

    let FlowEvent::Request { id, .. } = next_event(&mut proxy.events).await else {
        panic!("expected a request event first");
    };
    next_event(&mut proxy.events).await;
    let FlowEvent::Completed { response_size, .. } = next_event(&mut proxy.events).await else {
        panic!("expected a completed event");
    };
    assert_eq!(response_size, 100_000);

    let body = proxy.engine.body(id, Direction::Response).unwrap();
    assert_eq!(body.bytes.len(), 1_000);
    assert_eq!(body.size, 100_000);
    assert!(body.truncated);
}

#[tokio::test]
async fn response_cut_off_mid_body_is_reported_as_failed() {
    let origin = start_fixed_origin(
        b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\nonly ten b".to_vec(),
    )
    .await;
    let mut proxy = start_proxy(SslProxyingList::new()).await;

    send(proxy.addr, get(origin, "/")).await;

    let FlowEvent::Request { id, .. } = next_event(&mut proxy.events).await else {
        panic!("expected a request event first");
    };
    let FlowEvent::Response { .. } = next_event(&mut proxy.events).await else {
        panic!("expected a response event second");
    };
    let event = next_event(&mut proxy.events).await;
    let FlowEvent::Failed { id: failed_id, .. } = event else {
        panic!("expected a failed event, got {event:?}");
    };
    assert_eq!(failed_id, id);
    assert!(
        proxy
            .engine
            .body(id, Direction::Response)
            .unwrap()
            .incomplete
    );
}

#[tokio::test]
async fn body_is_forwarded_as_it_arrives() {
    let (release, released) = tokio::sync::oneshot::channel::<()>();
    let released = std::sync::Mutex::new(Some(released));
    let origin = start_origin(move |mut stream| {
        let released = released.lock().unwrap().take();
        async move {
            read_request(&mut stream).await;
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n\
                      5\r\nfirst\r\n",
                )
                .await
                .unwrap();
            if let Some(released) = released {
                let _ = released.await;
            }
            stream.write_all(b"4\r\nlast\r\n0\r\n\r\n").await.unwrap();
        }
    })
    .await;
    let proxy = start_proxy(SslProxyingList::new()).await;

    let mut client = TcpStream::connect(proxy.addr).await.unwrap();
    client
        .write_all(get(origin, "/stream").as_bytes())
        .await
        .unwrap();

    // The first chunk has to arrive while the origin is still holding back
    // the rest of the body.
    let mut seen = Vec::new();
    let mut buf = [0u8; 1024];
    while !String::from_utf8_lossy(&seen).contains("first") {
        let n = timeout(WAIT, client.read(&mut buf))
            .await
            .expect("the first chunk was held back until the body ended")
            .unwrap();
        assert!(n > 0, "connection closed early");
        seen.extend_from_slice(&buf[..n]);
    }

    release.send(()).unwrap();
    let rest = read_to_end(&mut client).await;
    assert!(String::from_utf8_lossy(&rest).contains("last"));
}

#[tokio::test]
async fn old_bodies_are_dropped_to_stay_within_the_budget() {
    let origin = start_origin(|mut stream| async move {
        read_request(&mut stream).await;
        let body = vec![b'x'; 1_000];
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        stream.write_all(&body).await.unwrap();
    })
    .await;
    let mut proxy = start_proxy(SslProxyingList::new()).await;
    proxy.engine.set_body_budget(2_500);

    let mut ids = Vec::new();
    for _ in 0..4 {
        send(proxy.addr, get(origin, "/")).await;
        let FlowEvent::Request { id, .. } = next_event(&mut proxy.events).await else {
            panic!("expected a request event");
        };
        next_event(&mut proxy.events).await;
        next_event(&mut proxy.events).await;
        ids.push(id);
    }

    assert!(proxy.engine.body(ids[0], Direction::Response).is_none());
    assert!(proxy.engine.body(ids[1], Direction::Response).is_none());
    assert!(proxy.engine.body(ids[2], Direction::Response).is_some());
    assert!(proxy.engine.body(ids[3], Direction::Response).is_some());
}

/// An HTTPS server on `localhost` with a self-signed certificate, like a
/// development server. Answers every request with `secure hello`.
async fn start_self_signed_origin() -> u16 {
    let key = hudsucker::rcgen::KeyPair::generate().unwrap();
    let cert = hudsucker::rcgen::CertificateParams::new(vec!["localhost".to_owned()])
        .unwrap()
        .self_signed(&key)
        .unwrap();
    let config = ServerConfig::builder_with_provider(Arc::new(aws_lc_rs::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.der().clone()],
            PrivateKeyDer::from_pem_slice(key.serialize_pem().as_bytes()).unwrap(),
        )
        .unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(config));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.unwrap();
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                // A rejected handshake is the point of some tests.
                let Ok(mut tls) = acceptor.accept(stream).await else {
                    return;
                };
                read_request(&mut tls).await;
                tls.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\nConnection: close\r\n\r\nsecure hello",
                )
                .await
                .unwrap();
                let _ = tls.shutdown().await;
            });
        }
    });
    port
}

/// Opens a decrypted connection to `host:port` through the proxy, trusting
/// the proxy's root certificate the way a set-up device would.
async fn connect_through(
    proxy: &Proxy,
    host: &str,
    port: u16,
) -> impl AsyncRead + AsyncWrite + Unpin + use<> {
    let mut tcp = TcpStream::connect(proxy.addr).await.unwrap();
    tcp.write_all(
        format!("CONNECT {host}:{port} HTTP/1.1\r\nHost: {host}:{port}\r\n\r\n").as_bytes(),
    )
    .await
    .unwrap();
    read_head(&mut tcp).await;

    let mut roots = RootCertStore::empty();
    roots
        .add(CertificateDer::from_pem_slice(proxy.ca_pem.as_bytes()).unwrap())
        .unwrap();
    let config = ClientConfig::builder_with_provider(Arc::new(aws_lc_rs::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    TlsConnector::from(Arc::new(config))
        .connect(ServerName::try_from(host.to_owned()).unwrap(), tcp)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_self_signed_server_is_rejected_with_a_reason() {
    let port = start_self_signed_origin().await;
    let mut proxy = start_proxy(SslProxyingList::new().include("localhost")).await;

    let mut tls = connect_through(&proxy, "localhost", port).await;
    tls.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let reply = String::from_utf8_lossy(&read_to_end(&mut tls).await).into_owned();
    assert!(
        reply.starts_with("HTTP/1.1 502"),
        "unexpected reply: {reply}"
    );

    let FlowEvent::Request { id, .. } = next_event(&mut proxy.events).await else {
        panic!("expected a request event first");
    };
    let event = next_event(&mut proxy.events).await;
    let FlowEvent::Failed {
        id: failed_id,
        error,
    } = event
    else {
        panic!("expected a failed event, got {event:?}");
    };
    assert_eq!(failed_id, id);
    assert!(
        error.contains("certificate was rejected"),
        "unexpected reason: {error}"
    );
}

#[tokio::test]
async fn an_exempt_self_signed_server_is_reached_and_marked() {
    let port = start_self_signed_origin().await;
    let mut proxy = start_proxy(SslProxyingList::new().include("localhost")).await;
    proxy
        .engine
        .set_exempt_hosts(vec![HostPattern::new("localhost")]);

    let mut tls = connect_through(&proxy, "localhost", port).await;
    tls.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let reply = String::from_utf8_lossy(&read_to_end(&mut tls).await).into_owned();
    assert!(reply.ends_with("secure hello"), "unexpected reply: {reply}");

    next_event(&mut proxy.events).await;
    let event = next_event(&mut proxy.events).await;
    let FlowEvent::Response {
        status,
        unverified_upstream,
        ..
    } = event
    else {
        panic!("expected a response event, got {event:?}");
    };
    assert_eq!(status, 200);
    assert!(unverified_upstream);
}

/// An address nothing listens on, so a request that reaches it fails.
async fn closed_address() -> SocketAddr {
    TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap()
}

fn map_local(pattern: String, file: &std::path::Path) -> MapRules {
    MapRules {
        local: vec![MapLocal {
            pattern: UrlPattern::new(pattern),
            method: None,
            file: file.display().to_string(),
            status: 200,
            content_type: None,
        }],
        remote: vec![],
    }
}

#[tokio::test]
async fn map_local_answers_from_a_file_without_reaching_the_server() {
    let server = closed_address().await;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("users.json");
    std::fs::write(&file, r#"{"users":["mapped"]}"#).unwrap();
    let mut proxy = start_proxy(SslProxyingList::new()).await;
    proxy
        .engine
        .set_map_rules(map_local(format!("http://{server}/api/*"), &file));

    let reply = send(proxy.addr, get(server, "/api/users?page=1")).await;
    assert!(
        reply.starts_with("HTTP/1.1 200"),
        "unexpected reply: {reply}"
    );
    assert!(
        reply
            .to_lowercase()
            .contains("content-type: application/json")
    );
    assert!(
        reply.ends_with(r#"{"users":["mapped"]}"#),
        "unexpected reply: {reply}"
    );

    let FlowEvent::Request { id, url, .. } = next_event(&mut proxy.events).await else {
        panic!("expected a request event first");
    };
    assert_eq!(url, format!("http://{server}/api/users?page=1"));
    let event = next_event(&mut proxy.events).await;
    let FlowEvent::Rewritten { description, .. } = event else {
        panic!("expected a rewritten event, got {event:?}");
    };
    assert!(description.starts_with("Map Local: "), "{description}");
    let FlowEvent::Response { status, .. } = next_event(&mut proxy.events).await else {
        panic!("expected a response event");
    };
    assert_eq!(status, 200);
    let FlowEvent::Completed { response_size, .. } = next_event(&mut proxy.events).await else {
        panic!("expected a completed event");
    };
    assert_eq!(response_size, 20);
    assert_eq!(
        &proxy.engine.body(id, Direction::Response).unwrap().bytes[..],
        br#"{"users":["mapped"]}"#
    );
}

#[tokio::test]
async fn map_local_reads_the_file_on_every_request() {
    let server = closed_address().await;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("body.txt");
    let proxy = start_proxy(SslProxyingList::new()).await;
    proxy
        .engine
        .set_map_rules(map_local(format!("http://{server}/*"), &file));

    std::fs::write(&file, "first").unwrap();
    assert!(send(proxy.addr, get(server, "/")).await.ends_with("first"));
    std::fs::write(&file, "second").unwrap();
    assert!(send(proxy.addr, get(server, "/")).await.ends_with("second"));
}

#[tokio::test]
async fn map_local_with_a_missing_file_says_so() {
    let server = closed_address().await;
    let dir = tempfile::tempdir().unwrap();
    let mut proxy = start_proxy(SslProxyingList::new()).await;
    proxy.engine.set_map_rules(map_local(
        format!("http://{server}/*"),
        &dir.path().join("missing.json"),
    ));

    let reply = send(proxy.addr, get(server, "/")).await;
    assert!(
        reply.starts_with("HTTP/1.1 500"),
        "unexpected reply: {reply}"
    );
    assert!(
        !reply.contains("missing.json"),
        "the device must not learn paths on this machine: {reply}"
    );

    // The path and the reason are reported on this machine instead.
    next_event(&mut proxy.events).await;
    let event = next_event(&mut proxy.events).await;
    let FlowEvent::Rewritten { description, .. } = event else {
        panic!("expected a rewritten event, got {event:?}");
    };
    assert!(
        description.contains("could not read") && description.contains("missing.json"),
        "{description}"
    );
}

#[tokio::test]
async fn map_remote_sends_the_request_to_another_server() {
    let asked_for = closed_address().await;
    let (heads, mut seen_heads) = tokio::sync::mpsc::unbounded_channel();
    let actual = start_origin(move |mut stream| {
        let heads = heads.clone();
        async move {
            let (head, _) = read_request(&mut stream).await;
            heads.send(head).unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nstaged",
                )
                .await
                .unwrap();
        }
    })
    .await;
    let mut proxy = start_proxy(SslProxyingList::new()).await;
    proxy.engine.set_map_rules(MapRules {
        local: vec![],
        remote: vec![MapRemote {
            pattern: UrlPattern::new(format!("http://{asked_for}/*")),
            method: None,
            scheme: None,
            host: Some("127.0.0.1".to_owned()),
            port: Some(actual.port()),
            path: None,
        }],
    });

    let reply = send(proxy.addr, get(asked_for, "/v1/items?x=1")).await;
    assert!(reply.ends_with("staged"), "unexpected reply: {reply}");

    let head = timeout(WAIT, seen_heads.recv()).await.unwrap().unwrap();
    assert!(
        head.starts_with("GET /v1/items?x=1 "),
        "unexpected head: {head}"
    );
    assert!(
        head.to_lowercase().contains(&format!("host: {actual}")),
        "the Host header must follow the new server: {head}"
    );

    let FlowEvent::Request { url, .. } = next_event(&mut proxy.events).await else {
        panic!("expected a request event first");
    };
    assert_eq!(url, format!("http://{asked_for}/v1/items?x=1"));
    let event = next_event(&mut proxy.events).await;
    let FlowEvent::Rewritten { description, .. } = event else {
        panic!("expected a rewritten event, got {event:?}");
    };
    assert_eq!(
        description,
        format!("Map Remote: http://{actual}/v1/items?x=1")
    );
}

#[tokio::test]
async fn connect_to_an_unlisted_host_is_tunnelled_unread() {
    let origin = start_fixed_origin(
        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello".to_vec(),
    )
    .await;
    let mut proxy = start_proxy(SslProxyingList::new()).await;

    let mut client = TcpStream::connect(proxy.addr).await.unwrap();
    client
        .write_all(format!("CONNECT {origin} HTTP/1.1\r\nHost: {origin}\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let established = read_head(&mut client).await;
    assert!(
        established.starts_with("HTTP/1.1 200"),
        "unexpected reply: {established}"
    );

    // Whatever goes through the tunnel must reach the origin byte for byte.
    client
        .write_all(b"POST /through-tunnel HTTP/1.1\r\nHost: origin\r\n\r\n")
        .await
        .unwrap();
    let reply = String::from_utf8(read_to_end(&mut client).await).unwrap();
    assert!(reply.ends_with("hello"), "unexpected reply: {reply}");

    let event = next_event(&mut proxy.events).await;
    let FlowEvent::Tunnel { authority, .. } = event else {
        panic!("expected a tunnel event, got {event:?}");
    };
    assert_eq!(authority, origin.to_string());
}

#[tokio::test]
async fn unreachable_upstream_is_reported_as_failed() {
    // Bind then drop, so the port is known to refuse connections.
    let closed = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap();
    let mut proxy = start_proxy(SslProxyingList::new()).await;

    let reply = send(proxy.addr, get(closed, "/")).await;
    assert!(
        reply.starts_with("HTTP/1.1 502"),
        "unexpected reply: {reply}"
    );

    let FlowEvent::Request { id, .. } = next_event(&mut proxy.events).await else {
        panic!("expected a request event first");
    };
    let event = next_event(&mut proxy.events).await;
    let FlowEvent::Failed { id: failed_id, .. } = event else {
        panic!("expected a failed event, got {event:?}");
    };
    assert_eq!(failed_id, id);
}

#[tokio::test]
async fn the_proxy_address_serves_the_certificate_page() {
    let mut proxy = start_proxy(SslProxyingList::new()).await;

    let reply = send(
        proxy.addr,
        format!(
            "GET / HTTP/1.1\r\nHost: {}\r\nUser-Agent: Mozilla/5.0 (iPhone; CPU iPhone OS 18_1)\r\n\
             Connection: close\r\n\r\n",
            proxy.addr
        ),
    )
    .await;

    assert!(
        reply.starts_with("HTTP/1.1 200"),
        "unexpected reply: {reply}"
    );
    assert!(reply.contains("text/html"), "unexpected reply: {reply}");
    assert!(
        reply.contains("Certificate Trust Settings"),
        "iOS steps expected: {reply}"
    );
    assert!(
        reply.contains("href=\"/lantaran.mobileconfig\""),
        "an iPhone gets the profile: {reply}"
    );
    assert!(
        proxy.events.try_recv().is_none(),
        "the certificate page is not a captured flow"
    );
}

#[tokio::test]
async fn the_certificate_downloads_in_both_forms() {
    let mut proxy = start_proxy(SslProxyingList::new()).await;

    let mut client = TcpStream::connect(proxy.addr).await.unwrap();
    client
        .write_all(b"GET /lantaran-ca.crt HTTP/1.1\r\nHost: proxy\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let reply = read_to_end(&mut client).await;
    let text = String::from_utf8_lossy(&reply).to_lowercase();
    assert!(text.contains("content-type: application/x-x509-ca-cert"));
    assert!(text.contains("filename=\"lantaran-ca.crt\""));
    // The body is DER: binary, starting with an ASN.1 SEQUENCE.
    let body_start = reply.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    assert_eq!(reply[body_start], 0x30);

    let pem = send(
        proxy.addr,
        "GET /lantaran-ca.pem HTTP/1.1\r\nHost: proxy\r\nConnection: close\r\n\r\n".to_owned(),
    )
    .await;
    assert!(
        pem.ends_with(&proxy.ca_pem),
        "the PEM download must be the root certificate"
    );
    assert!(proxy.events.try_recv().is_none());
}

#[tokio::test]
async fn an_iphone_downloads_a_configuration_profile() {
    let proxy = start_proxy(SslProxyingList::new()).await;

    let reply = send(
        proxy.addr,
        "GET /lantaran.mobileconfig HTTP/1.1\r\nHost: proxy\r\nConnection: close\r\n\r\n"
            .to_owned(),
    )
    .await;

    assert!(
        reply.starts_with("HTTP/1.1 200"),
        "unexpected reply: {reply}"
    );
    assert!(
        reply
            .to_lowercase()
            .contains("content-type: application/x-apple-aspen-config"),
        "unexpected reply: {reply}"
    );
    assert!(reply.contains("<string>com.apple.security.root</string>"));
    // The profile embeds the same certificate the PEM download offers.
    let pem_body: String = proxy
        .ca_pem
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect();
    assert!(
        reply.contains(&format!("<data>{pem_body}</data>")),
        "the profile must carry the root certificate"
    );
}

#[tokio::test]
async fn the_certificate_page_is_reachable_through_the_proxy_by_name() {
    let mut proxy = start_proxy(SslProxyingList::new()).await;

    let reply = send(
        proxy.addr,
        "GET http://lantaran.ssl/ HTTP/1.1\r\nHost: lantaran.ssl\r\n\
         User-Agent: Mozilla/5.0 (Linux; Android 15; Pixel 8)\r\nConnection: close\r\n\r\n"
            .to_owned(),
    )
    .await;

    assert!(
        reply.starts_with("HTTP/1.1 200"),
        "unexpected reply: {reply}"
    );
    assert!(
        reply.contains("CA certificate"),
        "Android steps expected: {reply}"
    );
    assert!(proxy.events.try_recv().is_none());
}

#[tokio::test]
async fn an_unknown_path_on_the_certificate_page_is_not_found() {
    let proxy = start_proxy(SslProxyingList::new()).await;

    let reply = send(
        proxy.addr,
        "GET /etc/passwd HTTP/1.1\r\nHost: proxy\r\nConnection: close\r\n\r\n".to_owned(),
    )
    .await;
    assert!(
        reply.starts_with("HTTP/1.1 404"),
        "unexpected reply: {reply}"
    );
}
