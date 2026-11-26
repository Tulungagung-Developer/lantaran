//! The Lantaran proxy engine.
//!
//! The engine owns the root certificate, accepts proxy connections and reports
//! what it sees as a stream of [`FlowEvent`]s. Captured bodies are fetched by
//! flow id with [`Engine::body`]. Events arrive in batches from an
//! [`EventStream`], which drops events rather than slow the proxy down. It has no user interface: the command line
//! tool and the desktop apps all drive this same crate.

mod ca;
mod capture;
mod engine;
mod error;
mod events;
mod flow;
mod guard;
mod upstream;

pub use ca::RootCa;
pub use capture::MAX_DECODED_SIZE;
pub use engine::{DEFAULT_BODY_BUDGET, DEFAULT_BODY_LIMIT, Engine};
pub use error::Error;
pub use events::{BATCH_WINDOW, DEFAULT_EVENT_CAPACITY, EventBatch, EventStream, MAX_BATCH};
pub use flow::{CapturedBody, Direction, FlowEvent, FlowId, Headers};
pub use lantaran_rules::{HostPattern, SslProxyingList};
