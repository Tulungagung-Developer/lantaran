//! Matching rules for the Lantaran proxy.
//!
//! This crate has no I/O and no dependencies, so every rule can be unit tested
//! on its own and reused by any front end.

mod glob;
mod host_pattern;
mod map;
mod ssl_proxying;
mod url_pattern;

pub use host_pattern::HostPattern;
pub use map::{MapLocal, MapRemote, MapRules, content_type_for};
pub use ssl_proxying::SslProxyingList;
pub use url_pattern::UrlPattern;
