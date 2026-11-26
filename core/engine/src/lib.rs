//! The Lantaran proxy engine.
//!
//! The engine owns the root certificate, accepts proxy connections and reports
//! what it sees as a stream of [`FlowEvent`]s. It has no user interface: the
//! command line tool and the desktop apps all drive this same crate.

mod ca;
mod engine;
mod error;
mod flow;

pub use ca::RootCa;
pub use engine::Engine;
pub use error::Error;
pub use flow::{FlowEvent, FlowId};
pub use lantaran_rules::{HostPattern, SslProxyingList};
