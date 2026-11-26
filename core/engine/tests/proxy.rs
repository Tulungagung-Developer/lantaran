//! Drives a running engine over real sockets, with a local stand-in for the
//! upstream server so the tests need no network.

use lantaran_engine::{Engine, FlowEvent, RootCa, SslProxyingList};
use std::{future::pending, net::SocketAddr, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::mpsc::UnboundedReceiver,
    time::timeout,
};

const WAIT: Duration = Duration::from_secs(5);

/// An upstream server that answers every request with `hello`.
async fn start_origin() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                read_head(&mut stream).await;
                stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello",
                    )
                    .await
                    .unwrap();
            });
        }
    });

    addr
}

async fn start_proxy(ssl_proxying: SslProxyingList) -> (SocketAddr, UnboundedReceiver<FlowEvent>) {
    let dir = tempfile::tempdir().unwrap();
    let ca = RootCa::load_or_create(dir.path()).unwrap();
    let (engine, events) = Engine::new(ca, ssl_proxying);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { engine.serve(listener, pending()).await });

    (addr, events)
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

async fn read_to_end(stream: &mut TcpStream) -> String {
    let mut out = String::new();
    timeout(WAIT, stream.read_to_string(&mut out))
        .await
        .unwrap()
        .unwrap();
    out
}

async fn next_event(events: &mut UnboundedReceiver<FlowEvent>) -> FlowEvent {
    timeout(WAIT, events.recv()).await.unwrap().unwrap()
}

#[tokio::test]
async fn plain_http_request_is_forwarded_and_reported() {
    let origin = start_origin().await;
    let (proxy, mut events) = start_proxy(SslProxyingList::new()).await;

    let mut client = TcpStream::connect(proxy).await.unwrap();
    client
        .write_all(
            format!(
                "GET http://{origin}/hello HTTP/1.1\r\nHost: {origin}\r\nConnection: close\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let reply = read_to_end(&mut client).await;

    assert!(
        reply.starts_with("HTTP/1.1 200"),
        "unexpected reply: {reply}"
    );
    assert!(reply.ends_with("hello"), "unexpected reply: {reply}");

    let FlowEvent::Request {
        id, method, url, ..
    } = next_event(&mut events).await
    else {
        panic!("expected a request event first");
    };
    assert_eq!(method, "GET");
    assert_eq!(url, format!("http://{origin}/hello"));

    let FlowEvent::Response {
        id: response_id,
        status,
        content_length,
        ..
    } = next_event(&mut events).await
    else {
        panic!("expected a response event second");
    };
    assert_eq!(response_id, id);
    assert_eq!(status, 200);
    assert_eq!(content_length, Some(5));
}

#[tokio::test]
async fn connect_to_an_unlisted_host_is_tunnelled_unread() {
    let origin = start_origin().await;
    let (proxy, mut events) = start_proxy(SslProxyingList::new()).await;

    let mut client = TcpStream::connect(proxy).await.unwrap();
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
    let reply = read_to_end(&mut client).await;
    assert!(reply.ends_with("hello"), "unexpected reply: {reply}");

    let event = next_event(&mut events).await;
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
    let (proxy, mut events) = start_proxy(SslProxyingList::new()).await;

    let mut client = TcpStream::connect(proxy).await.unwrap();
    client
        .write_all(
            format!("GET http://{closed}/ HTTP/1.1\r\nHost: {closed}\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .await
        .unwrap();
    let reply = read_to_end(&mut client).await;
    assert!(
        reply.starts_with("HTTP/1.1 502"),
        "unexpected reply: {reply}"
    );

    let FlowEvent::Request { id, .. } = next_event(&mut events).await else {
        panic!("expected a request event first");
    };
    let event = next_event(&mut events).await;
    let FlowEvent::Failed { id: failed_id, .. } = event else {
        panic!("expected a failed event, got {event:?}");
    };
    assert_eq!(failed_id, id);
}

#[tokio::test]
async fn request_aimed_at_the_proxy_itself_gets_a_notice() {
    let (proxy, mut events) = start_proxy(SslProxyingList::new()).await;

    let mut client = TcpStream::connect(proxy).await.unwrap();
    client
        .write_all(
            format!("GET / HTTP/1.1\r\nHost: {proxy}\r\nConnection: close\r\n\r\n").as_bytes(),
        )
        .await
        .unwrap();
    let reply = read_to_end(&mut client).await;

    assert!(
        reply.starts_with("HTTP/1.1 200"),
        "unexpected reply: {reply}"
    );
    assert!(
        reply.contains("Lantaran is running"),
        "unexpected reply: {reply}"
    );
    assert!(
        events.try_recv().is_err(),
        "a request to the proxy itself is not a captured flow"
    );
}
