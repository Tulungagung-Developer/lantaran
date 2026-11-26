mod cert;

use clap::{Parser, Subcommand};
use lantaran_engine::{
    Direction, Engine, EventStream, FlowEvent, FlowId, Headers, HostPattern, MapLocal, MapRemote,
    MapRules, ONBOARDING_HOST, RootCa, SslProxyingList, UrlPattern, local_network_address,
};
use std::{
    error::Error,
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    process::ExitCode,
    sync::Arc,
};
use tokio::net::TcpListener;

/// Capture HTTP and HTTPS traffic from iOS and Android devices.
///
/// Without a command, runs the proxy.
#[derive(Parser)]
#[command(name = "lantaran", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    #[command(flatten)]
    proxy: Args,

    /// Directory that holds the root certificate and its key.
    #[arg(long, value_name = "DIR", global = true)]
    data_dir: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Command {
    /// Export, import or reset the root certificate.
    #[command(subcommand)]
    Cert(cert::CertCommand),
}

#[derive(clap::Args)]
struct Args {
    /// Address to listen on. The default accepts only this machine. Pass
    /// 0.0.0.0:9090 to accept phones on the same network; any device on that
    /// network can then use the proxy while it runs.
    #[arg(long, default_value = DEFAULT_LISTEN, value_name = "ADDR")]
    listen: SocketAddr,

    /// Host to decrypt, such as api.example.com or *.example.com. Repeat for
    /// more hosts. HTTPS to any other host passes through unread.
    #[arg(long = "ssl", value_name = "HOST")]
    ssl: Vec<String>,

    /// Host whose certificate is accepted without being checked, for a
    /// development server with a self-signed certificate. Takes the same
    /// patterns as --ssl and can be repeated.
    #[arg(long = "insecure-host", value_name = "HOST")]
    insecure_hosts: Vec<String>,

    /// Answer matching requests from a file: PATTERN=FILE, for example
    /// 'https://api.example.com/v1/users*=./users.json'. Can be repeated.
    #[arg(long = "map-local", value_name = "PATTERN=FILE")]
    map_local: Vec<String>,

    /// Send matching requests elsewhere: PATTERN=URL, for example
    /// 'https://api.example.com/*=http://localhost:3000'. The URL's scheme,
    /// host, port and path replace the request's; a path of "/" keeps the
    /// original path. Can be repeated.
    #[arg(long = "map-remote", value_name = "PATTERN=URL")]
    map_remote: Vec<String>,

    /// The address to show for the certificate page, when the one Lantaran
    /// picks is wrong, for example with a VPN or several network interfaces.
    #[arg(long, value_name = "IP")]
    advertise: Option<IpAddr>,

    /// Also print headers and the start of each body.
    #[arg(short, long)]
    verbose: bool,
}

impl Args {
    /// Whether any of the proxy's options were given.
    fn is_set(&self) -> bool {
        self.listen != DEFAULT_LISTEN.parse::<SocketAddr>().unwrap_or(self.listen)
            || !self.ssl.is_empty()
            || !self.insecure_hosts.is_empty()
            || !self.map_local.is_empty()
            || !self.map_remote.is_empty()
            || self.advertise.is_some()
            || self.verbose
    }
}

const DEFAULT_LISTEN: &str = "127.0.0.1:9090";

/// How much of a body `--verbose` prints.
const BODY_PREVIEW: usize = 400;

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match data_dir(cli.data_dir) {
        Ok(data_dir) => match cli.command {
            // clap cannot keep the proxy's options away from the commands
            // while sharing --data-dir with them, so check here.
            Some(_) if cli.proxy.is_set() => {
                Err("the proxy's options cannot be used with a command".into())
            }
            Some(Command::Cert(command)) => cert::run(command, &data_dir),
            None => run(cli.proxy, &data_dir).await,
        },
        Err(err) => Err(err),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("lantaran: {err}");
            ExitCode::FAILURE
        }
    }
}

fn data_dir(given: Option<PathBuf>) -> Result<PathBuf, Box<dyn Error>> {
    match given {
        Some(dir) => Ok(dir),
        None => Ok(dirs::data_dir()
            .ok_or("could not find a data directory for this user; pass --data-dir")?
            .join("Lantaran")),
    }
}

async fn run(args: Args, data_dir: &Path) -> Result<(), Box<dyn Error>> {
    let ca = RootCa::load_or_create(data_dir)?;
    let ssl_proxying = args.ssl.iter().fold(SslProxyingList::new(), |list, host| {
        list.include(host.as_str())
    });

    let listener = TcpListener::bind(args.listen)
        .await
        .map_err(|e| format!("could not listen on {}: {e}", args.listen))?;
    let bound = listener.local_addr()?;

    let shown = match args.advertise {
        Some(ip) => SocketAddr::new(ip, bound.port()),
        None => advertised_addr(bound),
    };
    println!("Listening on {shown}");
    if bound.ip().is_loopback() {
        println!("Only this machine can connect. Pass --listen 0.0.0.0:9090 to accept phones.");
        println!(
            "Certificate page: http://{shown}/ here, or http://{ONBOARDING_HOST}/ from a simulator or emulator"
        );
    } else {
        println!("Any device on this network can use this proxy while it runs.");
        let page = format!("http://{shown}/");
        println!("Certificate page: {page}  (or http://{ONBOARDING_HOST}/ once the proxy is set)");
        print_qr(&page);
    }
    println!(
        "Root certificate: {} ({})",
        ca.name(),
        ca.cert_path().display()
    );
    if args.ssl.is_empty() {
        println!("Decrypting no hosts. Pass --ssl <HOST> to read HTTPS traffic.");
    } else {
        println!("Decrypting: {}", args.ssl.join(", "));
    }

    if !args.insecure_hosts.is_empty() {
        println!(
            "Not checking certificates of: {}",
            args.insecure_hosts.join(", ")
        );
    }

    let map_rules = map_rules(&args.map_local, &args.map_remote)?;
    for rule in &map_rules.local {
        println!("Map Local: {} -> {}", rule.pattern.as_str(), rule.file);
    }
    for rule in &args.map_remote {
        println!("Map Remote: {}", rule.replacen('=', " -> ", 1));
    }

    let (engine, events) = Engine::new(ca, ssl_proxying);
    engine.set_map_rules(map_rules);
    engine.set_exempt_hosts(
        args.insecure_hosts
            .iter()
            .map(|host| HostPattern::new(host.as_str()))
            .collect(),
    );
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
                unverified_upstream,
            } => {
                let note = if unverified_upstream {
                    "  (server certificate not checked)"
                } else {
                    ""
                };
                println!("#{id} {status} after {} ms{note}", elapsed.as_millis());
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
            FlowEvent::Rewritten { id, description } => {
                println!("#{id} {}", printable(&description))
            }
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

/// Parses the --map-local and --map-remote arguments.
fn map_rules(local: &[String], remote: &[String]) -> Result<MapRules, String> {
    let split = |arg: &String, flag: &str| {
        arg.split_once('=')
            .map(|(pattern, target)| (pattern.trim().to_owned(), target.trim().to_owned()))
            .filter(|(pattern, target)| !pattern.is_empty() && !target.is_empty())
            .ok_or_else(|| format!("{flag} expects PATTERN=TARGET, got {arg}"))
    };

    let local = local
        .iter()
        .map(|arg| {
            let (pattern, file) = split(arg, "--map-local")?;
            Ok(MapLocal {
                pattern: UrlPattern::new(pattern),
                method: None,
                file,
                status: 200,
                content_type: None,
            })
        })
        .collect::<Result<_, String>>()?;

    let remote = remote
        .iter()
        .map(|arg| {
            let (pattern, target) = split(arg, "--map-remote")?;
            let (scheme, rest) = target
                .split_once("://")
                .ok_or_else(|| format!("--map-remote target must be a URL, got {target}"))?;
            let (authority, path) = match rest.find('/') {
                Some(slash) => (&rest[..slash], &rest[slash..]),
                None => (rest, "/"),
            };
            let (host, port) = match authority.rsplit_once(':') {
                Some((host, port)) if !host.ends_with(']') || authority.starts_with('[') => {
                    match port.parse::<u16>() {
                        Ok(port) => (host, Some(port)),
                        Err(_) => (authority, None),
                    }
                }
                _ => (authority, None),
            };
            Ok(MapRemote {
                pattern: UrlPattern::new(pattern),
                method: None,
                scheme: Some(scheme.to_owned()),
                host: Some(
                    host.trim_start_matches('[')
                        .trim_end_matches(']')
                        .to_owned(),
                ),
                port,
                path: (path != "/").then(|| path.to_owned()),
            })
        })
        .collect::<Result<_, String>>()?;

    Ok(MapRules { local, remote })
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

/// Prints a QR code a phone camera can open, using half-height blocks so it
/// stays small.
fn print_qr(text: &str) {
    use qrcode::{QrCode, render::unicode::Dense1x2};

    if let Ok(code) = QrCode::new(text) {
        let image = code
            .render::<Dense1x2>()
            .dark_color(Dense1x2::Light)
            .light_color(Dense1x2::Dark)
            .build();
        println!("{image}");
    }
}

/// The address to type into a phone. A wildcard listen address is replaced by
/// this machine's address on the local network.
fn advertised_addr(bound: SocketAddr) -> SocketAddr {
    if !bound.ip().is_unspecified() {
        return bound;
    }
    match local_network_address() {
        Some(ip) => SocketAddr::new(ip, bound.port()),
        None => bound,
    }
}

#[cfg(test)]
mod tests {
    use super::printable;

    #[test]
    fn map_arguments_are_parsed() {
        let rules = super::map_rules(
            &["https://api.test/users*=./users.json".to_owned()],
            &["https://api.test/*=http://localhost:3000".to_owned()],
        )
        .unwrap();
        assert_eq!(rules.local[0].file, "./users.json");
        assert_eq!(rules.remote[0].host.as_deref(), Some("localhost"));
        assert_eq!(rules.remote[0].port, Some(3000));
        assert_eq!(rules.remote[0].path, None);

        assert!(super::map_rules(&["no-equals".to_owned()], &[]).is_err());
        assert!(super::map_rules(&[], &["x=not-a-url".to_owned()]).is_err());
    }

    #[test]
    fn control_characters_are_escaped() {
        assert_eq!(printable("ok\u{1b}[2Jgone"), "ok\\u{1b}[2Jgone");
        assert_eq!(printable("bell\u{7}"), "bell\\u{7}");
        assert_eq!(printable("tab\tstays"), "tab\tstays");
        assert_eq!(printable("plain text"), "plain text");
    }
}
