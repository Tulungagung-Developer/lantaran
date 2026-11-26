use clap::Parser;
use lantaran_engine::{Engine, FlowEvent, RootCa, SslProxyingList};
use std::{
    error::Error,
    net::{IpAddr, SocketAddr, UdpSocket},
    path::PathBuf,
    process::ExitCode,
};
use tokio::{net::TcpListener, sync::mpsc};

/// Capture HTTP and HTTPS traffic from iOS and Android devices.
#[derive(Parser)]
#[command(name = "lantaran", version)]
struct Args {
    /// Address to listen on. Devices on the same network use it as their HTTP proxy.
    #[arg(long, default_value = "0.0.0.0:9090", value_name = "ADDR")]
    listen: SocketAddr,

    /// Host to decrypt, such as api.example.com or *.example.com. Repeat for
    /// more hosts. HTTPS to any other host passes through unread.
    #[arg(long = "ssl", value_name = "HOST")]
    ssl: Vec<String>,

    /// Directory that holds the root certificate and its key.
    #[arg(long, value_name = "DIR")]
    data_dir: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> ExitCode {
    match run(Args::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("lantaran: {err}");
            ExitCode::FAILURE
        }
    }
}

async fn run(args: Args) -> Result<(), Box<dyn Error>> {
    let data_dir = match args.data_dir {
        Some(dir) => dir,
        None => dirs::data_dir()
            .ok_or("could not find a data directory for this user; pass --data-dir")?
            .join("Lantaran"),
    };

    let ca = RootCa::load_or_create(&data_dir)?;
    let ssl_proxying = args.ssl.iter().fold(SslProxyingList::new(), |list, host| {
        list.include(host.as_str())
    });

    let listener = TcpListener::bind(args.listen)
        .await
        .map_err(|e| format!("could not listen on {}: {e}", args.listen))?;
    let bound = listener.local_addr()?;

    println!("Listening on {}", advertised_addr(bound));
    println!("Root certificate: {}", ca.cert_path().display());
    if args.ssl.is_empty() {
        println!("Decrypting no hosts. Pass --ssl <HOST> to read HTTPS traffic.");
    } else {
        println!("Decrypting: {}", args.ssl.join(", "));
    }

    let (engine, events) = Engine::new(ca, ssl_proxying);
    tokio::spawn(print_events(events));

    engine
        .serve(listener, async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

async fn print_events(mut events: mpsc::UnboundedReceiver<FlowEvent>) {
    while let Some(event) = events.recv().await {
        match event {
            FlowEvent::Request {
                id,
                client,
                method,
                url,
            } => println!("#{id} {method} {url}  (from {})", client.ip()),
            FlowEvent::Response {
                id,
                status,
                elapsed,
                content_length,
            } => {
                let size = content_length.map_or(String::new(), |n| format!(", {n} bytes"));
                println!("#{id} {status} in {} ms{size}", elapsed.as_millis());
            }
            FlowEvent::Failed { id, error } => println!("#{id} failed: {error}"),
            FlowEvent::Tunnel {
                id,
                client,
                authority,
            } => println!(
                "#{id} CONNECT {authority}  (from {}, not decrypted)",
                client.ip()
            ),
        }
    }
}

/// The address to type into a phone. A wildcard listen address is replaced by
/// this machine's address on the local network.
fn advertised_addr(bound: SocketAddr) -> SocketAddr {
    if !bound.ip().is_unspecified() {
        return bound;
    }
    match local_network_ip() {
        Some(ip) => SocketAddr::new(ip, bound.port()),
        None => bound,
    }
}

/// Finds the address of the interface used for outbound traffic. Connecting a
/// UDP socket only selects a route; no packet is sent.
fn local_network_ip() -> Option<IpAddr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    Some(socket.local_addr().ok()?.ip())
}
