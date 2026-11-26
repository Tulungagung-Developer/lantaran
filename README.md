# Lantaran

An open-source HTTP debugging proxy for iOS and Android development.

Point a phone, simulator or emulator at Lantaran and it shows the HTTP and HTTPS
requests the device makes. It is built as one proxy engine written in Rust, with
a native app for each desktop OS on top: macOS first, then Windows, then Linux.

## Status

Early. The engine, a command line tool and a first macOS app work.

| Works today | Planned next |
| --- | --- |
| HTTP proxy, HTTP/1.1 and HTTP/2 | WebSocket frames |
| HTTPS decryption for the hosts you list | Guided Android setup |
| Untouched tunnelling for every other host | Breakpoints |
| macOS app: traffic table, inspector, SSL proxying list | |
| Map Local and Map Remote | |
| iPhone setup with a configuration profile; iOS Simulator support | |
| Headers and bodies, decoded for display (`--verbose`) | |
| Certificate download page and QR code | |
| Root certificate: export, move to another machine, reset | |

Each release is a [milestone](https://github.com/Tulungagung-Developer/lantaran/milestones), and the
[roadmap](https://github.com/Tulungagung-Developer/lantaran/issues/34) lists everything planned, including
work that has no release yet.

## macOS app

You need Xcode (Swift 6) and [Rust](https://rustup.rs) 1.86 or newer.

```sh
apps/macos/scripts/build-app.sh
open apps/macos/build/Lantaran.app
```

The first build of the engine takes a few minutes. The app listens on port 9090,
the same as Proxyman; quit Proxyman first or pick another port in Settings (⌘,).
Use **Connect Device** in the toolbar to set up a phone; the first time, it asks
to accept connections from other devices on the network, which is off until then.
For the **iOS Simulator**, the same sheet installs the certificate into running
simulators and routes this Mac's traffic through Lantaran (the simulator has no
proxy settings of its own). Lantaran puts the Mac's previous proxy settings back
when you turn that off, stop the proxy or quit, and on the next launch if it
ever quits unexpectedly.
**SSL Proxying** chooses which hosts to decrypt. The app and the command line tool share the root
certificate, so a device set up with one works with the other.

To work on the app in Xcode, run `apps/macos/scripts/build-core.sh` once, then
open `apps/macos/Package.swift`. Run `build-core.sh` again after changing the
Rust code.

## Try it from the command line

You need [Rust](https://rustup.rs) 1.86 or newer.

```sh
cd core
cargo run -p lantaran-cli -- --listen 0.0.0.0:9090 --ssl api.example.com --ssl '*.example.org'
```

```
Listening on 192.168.1.3:9090
Any device on this network can use this proxy while it runs.
Root certificate: Lantaran Root CA (your-mac, 2026-11-27) (/Users/you/Library/Application Support/Lantaran/lantaran-ca.pem)
Decrypting: api.example.com, *.example.org
#1 GET https://api.example.com/v1/me  (from 192.168.1.20)
#1 200 in 212 ms, 512 bytes
#2 CONNECT gateway.icloud.com:443  (from 192.168.1.20, not decrypted)
```

To check it from the same machine:

```sh
curl -x http://127.0.0.1:9090 --cacert "<root certificate path>" https://api.example.com/
```

To capture a phone, start Lantaran with `--listen 0.0.0.0:9090`, scan the QR
code it prints (or open the printed address) on the phone, and follow the steps
on that page to install and trust the certificate. Then set the same address as
the manual HTTP proxy in the phone's Wi-Fi settings. From a simulator or
emulator, or once the proxy is set, the page is also at `http://lantaran.ssl/`.

Without `--listen`, Lantaran accepts connections from this machine only, which
is enough for `curl` and the iOS simulator. Listening on `0.0.0.0` lets every
device on the network use the proxy, so do that on networks you trust. Requests
from other devices to this machine's loopback addresses are always refused.

Map Local answers matching requests from a file, read fresh on every request;
Map Remote sends them to another server. Both work on readable traffic (plain
HTTP, or HTTPS for hosts passed with `--ssl`):

```sh
lantaran --ssl api.example.com \
  --map-local 'https://api.example.com/v1/users*=./users.json' \
  --map-remote 'https://api.example.com/*=http://localhost:3000'
```

In the app they are under **Tools**, or right-click a request.

Only hosts passed with `--ssl` are decrypted. When decrypting, Lantaran checks the
server's certificate against the public roots and every certificate authority
installed on this machine, so `mkcert` and company CAs work. For a development
server with a self-signed certificate, add `--insecure-host <HOST>`.

### The root certificate

Lantaran creates its root certificate on first run and keeps it, so a device set
up once keeps working. Its name includes the machine and the date, to tell
certificates from different machines apart on a device.

```sh
lantaran cert show                  # its name and where it is stored
lantaran cert export ca.crt         # .pem, .crt/.cer/.der, or .p12 with the private key
lantaran cert import backup.p12     # use another machine's certificate and key here
lantaran cert reset                 # a new certificate; every device must be set up again
```

A `.p12` file carries the private key, so it asks for a password; scripts can set
`LANTARAN_P12_PASSWORD` instead. Importing one on a second machine lets devices that
trusted the first work with it unchanged. Import and reset ask before replacing the
current certificate (`--yes` skips that), and a running Lantaran picks the new one up
when restarted. In the app these are under **Certificate**.

Apps that pin their certificates
cannot be decrypted, and Android release builds ignore user-installed
certificates unless the app opts in.

## Layout

```
core/           Rust workspace
  rules/        which hosts to decrypt; later map, block and breakpoint rules
  engine/       root certificate, interception, flow events
  cli/          the `lantaran` command
  ffi/          bindings the desktop apps use (uniffi)
apps/macos/     the macOS app (Swift package)
docs/           architecture notes
```

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the design and the reasons behind it.

## Contributing

Pick an issue from the next release's milestone and comment that you are taking it. Each issue has
a "Notes for implementers" section describing the code it touches. To report a bug or propose
something new, [open an issue](https://github.com/Tulungagung-Developer/lantaran/issues/new/choose)
with one of the templates. Before
opening a pull request, run this inside `core/`:

```sh
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
```

Commit messages follow the
[Angular format](https://github.com/angular/angular/blob/main/contributing-docs/commit-message-guidelines.md),
`<type>(<scope>): <summary>`, where the scope is the crate you changed:

```
fix(engine): report a failed TLS handshake as a flow event

Show a line when a device rejects the root certificate, so a missing
certificate no longer looks like silence.

Fixes #3
```

Do not add `Co-authored-by` trailers or tool attribution lines.

## License

[AGPL-3.0-or-later](LICENSE).
