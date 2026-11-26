use std::net::{IpAddr, UdpSocket};

/// This machine's address on the network it uses to reach the internet: the
/// address a phone on the same Wi-Fi should use as its proxy.
///
/// Connecting a UDP socket only picks a route; no packet is sent. With a VPN
/// or several network interfaces the result may not be the one the phone can
/// reach, so front ends should let the user override it.
pub fn local_network_address() -> Option<IpAddr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    Some(socket.local_addr().ok()?.ip())
}
