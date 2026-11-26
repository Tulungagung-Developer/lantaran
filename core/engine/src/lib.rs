//! The Lantaran proxy engine.
//!
//! The engine owns the root certificate, accepts proxy connections and reports
//! what it sees as a stream of [`FlowEvent`]s. Captured bodies are fetched by
//! flow id with [`Engine::body`]. It has no user interface: the command line
//! tool and the desktop apps all drive this same crate.

mod ca;
mod capture;
mod engine;
mod error;
mod flow;
mod guard;

pub use ca::RootCa;
pub use engine::{DEFAULT_BODY_LIMIT, Engine};
pub use error::Error;
pub use flow::{CapturedBody, Direction, FlowEvent, FlowId, Headers};
pub use lantaran_rules::{HostPattern, SslProxyingList};
