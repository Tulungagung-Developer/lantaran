# Lantaran

An open-source HTTP debugging proxy for iOS and Android development.

Point a phone, simulator or emulator at Lantaran and it shows the HTTP and HTTPS
requests the device makes. It is built as one proxy engine written in Rust, with
a native app for each desktop OS on top: macOS first, then Windows, then Linux.

## Status

Early. The engine and a command line tool work; the desktop apps do not exist yet.

| Works today | Planned next |
| --- | --- |
| HTTP proxy, HTTP/1.1 and HTTP/2 | WebSocket frames |
| HTTPS decryption for the hosts you list | Guided iOS and Android setup |
| Untouched tunnelling for every other host | macOS app |
| Headers and bodies, decoded for display (`--verbose`) | |
| Certificate download page and QR code | |
| Root certificate created on first run | |

Each release is a [milestone](https://github.com/Tulungagung-Developer/lantaran/milestones), and the
[roadmap](https://github.com/Tulungagung-Developer/lantaran/issues/34) lists everything planned, including
work that has no release yet.

## Try it

You need [Rust](https://rustup.rs) 1.86 or newer.

```sh
cd core
cargo run -p lantaran-cli -- --listen 0.0.0.0:9090 --ssl api.example.com --ssl '*.example.org'
```

```
Listening on 192.168.1.3:9090
Any device on this network can use this proxy while it runs.
Root certificate: /Users/you/Library/Application Support/Lantaran/lantaran-ca.pem
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

Only hosts passed with `--ssl` are decrypted. When decrypting, Lantaran checks the
server's certificate against the public roots and every certificate authority
installed on this machine, so `mkcert` and company CAs work. For a development
server with a self-signed certificate, add `--insecure-host <HOST>`.

Apps that pin their certificates
cannot be decrypted, and Android release builds ignore user-installed
certificates unless the app opts in.

## Layout

```
core/           Rust workspace
  rules/        which hosts to decrypt; later map, block and breakpoint rules
  engine/       root certificate, interception, flow events
  cli/          the `lantaran` command
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
