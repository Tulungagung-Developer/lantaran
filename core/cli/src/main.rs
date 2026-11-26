use clap::Parser;
use lantaran_engine::{
    Direction, Engine, EventStream, FlowEvent, FlowId, Headers, RootCa, SslProxyingList,
};
use std::{
    error::Error,
    net::{IpAddr, SocketAddr, UdpSocket},
    path::PathBuf,
    process::ExitCode,
    sync::Arc,
};
use tokio::net::TcpListener;

/// Capture HTTP and HTTPS traffic from iOS and Android devices.
#[derive(Parser)]
#[command(name = "lantaran", version)]
struct Args {
    /// Address to listen on. The default accepts only this machine. Pass
    /// 0.0.0.0:9090 to accept phones on the same network; any device on that
    /// network can then use the proxy while it runs.
    #[arg(long, default_value = "127.0.0.1:9090", value_name = "ADDR")]
    listen: SocketAddr,

    /// Host to decrypt, such as api.example.com or *.example.com. Repeat for
    /// more hosts. HTTPS to any other host passes through unread.
    #[arg(long = "ssl", value_name = "HOST")]
    ssl: Vec<String>,

    /// Directory that holds the root certificate and its key.
    #[arg(long, value_name = "DIR")]
    data_dir: Option<PathBuf>,

    /// Also print headers and the start of each body.
    #[arg(short, long)]
    verbose: bool,
}

/// How much of a body `--verbose` prints.
const BODY_PREVIEW: usize = 400;

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
    if bound.ip().is_loopback() {
        println!("Only this machine can connect. Pass --listen 0.0.0.0:9090 to accept phones.");
    } else {
        println!("Any device on this network can use this proxy while it runs.");
    }
    println!("Root certificate: {}", ca.cert_path().display());
    if args.ssl.is_empty() {
        println!("Decrypting no hosts. Pass --ssl <HOST> to read HTTPS traffic.");
    } else {
        println!("Decrypting: {}", args.ssl.join(", "));
    }

    let (engine, events) = Engine::new(ca, ssl_proxying);
    let engine = Arc::new(engine);
    tokio::spawn(print_events(Arc::clone(&engine), events, args.verbose));

    engine
        .serve(listener, async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

async fn print_events(engine: Arc<Engine>, mut events: EventStream, verbose: bool) {
    while let Some(batch) = events.next_batch().await {
        if batch.dropped > 0 {
            println!(
                "({} events were dropped because the terminal could not keep up)",
                batch.dropped
            );
        }
        for event in batch.events {
            print_event(&engine, event, verbose).await;
        }
    }
}

async fn print_event(engine: &Engine, event: FlowEvent, verbose: bool) {
    {
        match event {
            FlowEvent::Request {
                id,
                client,
                method,
                url,
                version,
                headers,
                ..
            } => {
                println!(
                    "#{id} {} {}  (from {})",
                    printable(&method),
                    printable(&url),
                    client.ip()
                );
                if verbose {
                    print_headers(&version, &headers);
                }
            }
            FlowEvent::Response {
                id,
                status,
                version,
                headers,
                elapsed,
            } => {
                println!("#{id} {status} after {} ms", elapsed.as_millis());
                if verbose {
                    print_headers(&version, &headers);
                }
            }
            FlowEvent::Completed {
                id,
                request_size,
                response_size,
                duration,
            } => {
                println!(
                    "#{id} done in {} ms: {request_size} bytes up, {response_size} bytes down",
                    duration.as_millis()
                );
                if verbose {
                    print_body(engine, id, Direction::Request).await;
                    print_body(engine, id, Direction::Response).await;
                }
            }
            FlowEvent::Failed { id, error } => println!("#{id} failed: {}", printable(&error)),
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

fn print_headers(version: &str, headers: &Headers) {
    println!("    {version}");
    for (name, value) in headers {
        println!("    {}: {}", printable(name), printable(value));
    }
}

async fn print_body(engine: &Engine, id: FlowId, direction: Direction) {
    let Some(body) = engine.body(id, direction) else {
        return;
    };
    if body.size == 0 {
        return;
    }

    let label = match direction {
        Direction::Request => "request body",
        Direction::Response => "response body",
    };
    let bytes = match body.decoded().await {
        Ok(bytes) => bytes,
        Err(_) => {
            println!("    {label}: {} bytes, could not be decoded", body.size);
            return;
        }
    };

    match std::str::from_utf8(&bytes) {
        Ok(text) => {
            let preview: String = text.chars().take(BODY_PREVIEW).collect();
            let more = if preview.len() < text.len() || body.truncated {
                " ..."
            } else {
                ""
            };
            let lines: Vec<String> = preview.lines().map(printable).collect();
            println!("    {label}: {}{more}", lines.join("\n    "));
        }
        Err(_) => println!("    {label}: {} bytes of binary data", body.size),
    }
}

/// Text from a device, made safe for a terminal: control characters such as
/// escape sequences are shown escaped instead of being acted on.
fn printable(text: &str) -> String {
    text.chars()
        .flat_map(|c| {
            let keep = !c.is_control() || c == '\t';
            let shown: Vec<char> = if keep {
                vec![c]
            } else {
                c.escape_default().collect()
            };
            shown
        })
        .collect()
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

#[cfg(test)]
mod tests {
    use super::printable;

    #[test]
    fn control_characters_are_escaped() {
        assert_eq!(printable("ok\u{1b}[2Jgone"), "ok\\u{1b}[2Jgone");
        assert_eq!(printable("bell\u{7}"), "bell\\u{7}");
        assert_eq!(printable("tab\tstays"), "tab\tstays");
        assert_eq!(printable("plain text"), "plain text");
    }
}
