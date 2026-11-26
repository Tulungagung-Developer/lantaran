use bytes::Bytes;
use std::{
    net::SocketAddr,
    time::{Duration, SystemTime},
};

/// Identifies one request and everything that later happens to it.
pub type FlowId = u64;

/// Header names and values in the order they were received. A value that is
/// not valid UTF-8 is converted lossily.
pub type Headers = Vec<(String, String)>;

/// Something the proxy observed. Events for one flow share its [`FlowId`].
///
/// A readable flow produces `Request`, then `Response`, then `Completed`.
/// `Failed` can arrive in place of `Response` or `Completed`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlowEvent {
    /// A readable request arrived: plain HTTP, or HTTPS for a decrypted host.
    Request {
        id: FlowId,
        client: SocketAddr,
        started_at: SystemTime,
        method: String,
        url: String,
        /// The HTTP version the client used, such as `HTTP/1.1` or `HTTP/2.0`.
        version: String,
        headers: Headers,
    },

    /// The upstream server sent the response headers for the request with
    /// this id. The body may still be arriving.
    Response {
        id: FlowId,
        status: u16,
        version: String,
        headers: Headers,
        /// Time from the request arriving to the response headers arriving.
        elapsed: Duration,
        /// The server's host is exempt from certificate checks, so its
        /// certificate may have been accepted without being verified.
        unverified_upstream: bool,
    },

    /// The response body was delivered to the client in full.
    Completed {
        id: FlowId,
        /// Body bytes as they travelled, before any decoding.
        request_size: u64,
        response_size: u64,
        /// Time from the request arriving to the last response byte.
        duration: Duration,
    },

    /// The flow with this id ended without a complete response.
    Failed { id: FlowId, error: String },

    /// A rule changed the flow with this id: Map Local answered it from a
    /// file, or Map Remote sent it somewhere else. Arrives right after
    /// `Request`, whose URL is the one the client asked for.
    Rewritten { id: FlowId, description: String },

    /// An HTTPS connection to a host that is not being decrypted. Its content
    /// passes through unread.
    Tunnel {
        id: FlowId,
        client: SocketAddr,
        authority: String,
    },
}

/// Which side of a flow a body belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    Request,
    Response,
}

/// A request or response body as it was captured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedBody {
    /// The bytes as they travelled, up to the capture limit. Still compressed
    /// if the message was; see [`CapturedBody::decoded`].
    pub bytes: Bytes,
    /// The full size of the body, including any part that was not kept.
    pub size: u64,
    /// Whether the capture limit cut the body short.
    pub truncated: bool,
    /// Whether the body ended before the message said it would, for example
    /// because a connection was closed.
    pub incomplete: bool,
    /// The message's `Content-Encoding`, if any.
    pub content_encoding: Option<String>,
}
