use std::{io, path::PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not access {}: {source}", path.display())]
    Io { path: PathBuf, source: io::Error },

    #[error("the root certificate or its key is not usable: {0}")]
    Certificate(#[from] hudsucker::rcgen::Error),

    #[error("the proxy stopped: {0}")]
    Proxy(#[from] hudsucker::Error),

    #[error("the body could not be decoded")]
    Decode,

    #[error("could not set up TLS for connections to servers: {0}")]
    Tls(String),
}
