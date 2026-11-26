# Lantaran

An open-source HTTP debugging proxy for iOS and Android development.

Point a phone, simulator or emulator at Lantaran and it shows the HTTP and HTTPS
requests the device makes. It is built as one proxy engine written in Rust, with
a native app for each desktop OS on top: macOS first, then Windows, then Linux.

## Status

Early. The engine and a command line tool work; the desktop apps do not exist yet.

| Works today | Planned next |
| --- | --- |
| HTTP proxy, HTTP/1.1 and HTTP/2 | Request and response bodies |
| HTTPS decryption for the hosts you list | WebSocket frames |
| Untouched tunnelling for every other host | Certificate download page and QR code |
| Root certificate created on first run | Guided iOS and Android setup |
| Flow events printed in the terminal | macOS app |

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

To capture a phone, set the address from the first line as the manual HTTP proxy
in the phone's Wi-Fi settings, then install and trust the root certificate on
the phone. Until the certificate download page lands, copy the `.pem` file to
the device yourself (AirDrop, or `adb push`).

Without `--listen`, Lantaran accepts connections from this machine only, which
is enough for `curl` and the iOS simulator. Listening on `0.0.0.0` lets every
device on the network use the proxy, so do that on networks you trust. Requests
from other devices to this machine's loopback addresses are always refused.

Only hosts passed with `--ssl` are decrypted. Apps that pin their certificates
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

Pick an issue from the next release's milestone and comment that you are taking it. Before
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
