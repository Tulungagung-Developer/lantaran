//! Keeps other devices from using the proxy to reach services that only this
//! machine is meant to see.
//!
//! The proxy listens on the local network so phones can connect. Without this
//! check, any device on that network could ask it for `http://127.0.0.1:5432`
//! and reach whatever listens on loopback here. Clients on this machine, such
//! as the iOS simulator, are not restricted: they can reach loopback anyway.
//!
//! The destination is resolved here and again when the connection is made, so
//! a DNS name that changes its answer in between can still get through.

use hudsucker::hyper::http::uri::Authority;
use std::net::IpAddr;

/// Whether a request from `client` to `destination` must be refused.
pub(crate) async fn refuses(client: IpAddr, destination: &Authority) -> bool {
    if client.to_canonical().is_loopback() {
        return false;
    }

    let host = destination
        .host()
        .trim_start_matches('[')
        .trim_end_matches(']');
    if let Ok(ip) = host.parse::<IpAddr>() {
        return is_private_to_this_machine(ip);
    }

    // The port does not affect resolution; lookup_host only requires one.
    match tokio::net::lookup_host((host, 0)).await {
        Ok(mut addrs) => addrs.any(|addr| is_private_to_this_machine(addr.ip())),
        // A name that does not resolve reaches nothing; let the normal
        // upstream error report it.
        Err(_) => false,
    }
}

/// Loopback, unspecified and link-local addresses. Link-local covers the
/// `169.254.169.254` metadata address used by cloud machines.
fn is_private_to_this_machine(ip: IpAddr) -> bool {
    match ip.to_canonical() {
        IpAddr::V4(ip) => ip.is_loopback() || ip.is_unspecified() || ip.is_link_local(),
        IpAddr::V6(ip) => ip.is_loopback() || ip.is_unspecified() || ip.is_unicast_link_local(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PHONE: &str = "192.168.1.20";

    async fn refused(client: &str, destination: &str) -> bool {
        refuses(
            client.parse().unwrap(),
            &Authority::try_from(destination).unwrap(),
        )
        .await
    }

    #[tokio::test]
    async fn another_device_cannot_reach_loopback() {
        assert!(refused(PHONE, "127.0.0.1:5432").await);
        assert!(refused(PHONE, "127.8.8.8").await);
        assert!(refused(PHONE, "[::1]:8080").await);
        assert!(refused(PHONE, "[::ffff:127.0.0.1]:8080").await);
    }

    #[tokio::test]
    async fn another_device_cannot_reach_loopback_by_name() {
        assert!(refused(PHONE, "localhost:3000").await);
    }

    #[tokio::test]
    async fn another_device_cannot_reach_unspecified_or_link_local() {
        assert!(refused(PHONE, "0.0.0.0:80").await);
        assert!(refused(PHONE, "[::]:80").await);
        assert!(refused(PHONE, "169.254.169.254").await);
        assert!(refused(PHONE, "[fe80::1]:80").await);
    }

    #[tokio::test]
    async fn another_device_can_reach_the_local_network_and_the_internet() {
        assert!(!refused(PHONE, "192.168.1.3:3000").await);
        assert!(!refused(PHONE, "10.0.2.2:8080").await);
        assert!(!refused(PHONE, "93.184.215.14:443").await);
    }

    #[tokio::test]
    async fn a_client_on_this_machine_is_not_restricted() {
        assert!(!refused("127.0.0.1", "127.0.0.1:5432").await);
        assert!(!refused("::1", "localhost:3000").await);
        assert!(!refused("::ffff:127.0.0.1", "127.0.0.1:5432").await);
    }

    #[tokio::test]
    async fn an_unresolvable_name_is_left_to_fail_upstream() {
        assert!(!refused(PHONE, "no-such-host.invalid:80").await);
    }
}
