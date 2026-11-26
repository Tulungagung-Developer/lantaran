//! Matching rules for the Lantaran proxy.
//!
//! This crate has no I/O and no dependencies, so every rule can be unit tested
//! on its own and reused by any front end.

mod host_pattern;
mod ssl_proxying;

pub use host_pattern::HostPattern;
pub use ssl_proxying::SslProxyingList;
