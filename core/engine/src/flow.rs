use std::{net::SocketAddr, time::Duration};

/// Identifies one request and everything that later happens to it.
pub type FlowId = u64;

/// Something the proxy observed. Events for one flow share its [`FlowId`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlowEvent {
    /// A readable request arrived: plain HTTP, or HTTPS for a decrypted host.
    Request {
        id: FlowId,
        client: SocketAddr,
        method: String,
        url: String,
    },

    /// The upstream server answered the request with this id.
    Response {
        id: FlowId,
        status: u16,
        elapsed: Duration,
        content_length: Option<u64>,
    },

    /// The request with this id never got an answer from upstream.
    Failed { id: FlowId, error: String },

    /// An HTTPS connection to a host that is not being decrypted. Its content
    /// passes through unread.
    Tunnel {
        id: FlowId,
        client: SocketAddr,
        authority: String,
    },
}
