//! Drives a running engine over real sockets, with local stand-ins for the
//! upstream server so the tests need no network.

use flate2::{Compression, write::GzEncoder};
use lantaran_engine::{Direction, Engine, FlowEvent, RootCa, SslProxyingList};
use std::{future::pending, io::Write, net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::mpsc::UnboundedReceiver,
    time::timeout,
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
    events: UnboundedReceiver<FlowEvent>,
    engine: Arc<Engine>,
}

async fn start_proxy(ssl_proxying: SslProxyingList) -> Proxy {
    let dir = tempfile::tempdir().unwrap();
    let ca = RootCa::load_or_create(dir.path()).unwrap();
    let (engine, events) = Engine::new(ca, ssl_proxying);
    let engine = Arc::new(engine);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let serving = Arc::clone(&engine);
    tokio::spawn(async move { serving.serve(listener, pending()).await });

    Proxy {
        addr,
        events,
        engine,
    }
}

/// Reads up to the blank line that ends an HTTP head.
async fn read_head(stream: &mut TcpStream) -> String {
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
async fn read_request(stream: &mut TcpStream) -> (String, Vec<u8>) {
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

async fn read_to_end(stream: &mut TcpStream) -> Vec<u8> {
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

async fn next_event(events: &mut UnboundedReceiver<FlowEvent>) -> FlowEvent {
    timeout(WAIT, events.recv()).await.unwrap().unwrap()
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
async fn request_aimed_at_the_proxy_itself_gets_a_notice() {
    let mut proxy = start_proxy(SslProxyingList::new()).await;

    let reply = send(
        proxy.addr,
        format!(
            "GET / HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
            proxy.addr
        ),
    )
    .await;

    assert!(
        reply.starts_with("HTTP/1.1 200"),
        "unexpected reply: {reply}"
    );
    assert!(
        reply.contains("Lantaran is running"),
        "unexpected reply: {reply}"
    );
    assert!(
        proxy.events.try_recv().is_err(),
        "a request to the proxy itself is not a captured flow"
    );
}
