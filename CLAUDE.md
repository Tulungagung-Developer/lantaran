# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

Lantaran is an HTTP debugging proxy for iOS and Android development: one proxy engine in Rust, with a native desktop app per OS planned on top (macOS first, then Windows, then Linux). Today only the engine and a command line tool exist. Capturing the desktop's own apps is not a goal; the targets are phones, simulators and emulators.

`docs/ARCHITECTURE.md` records the design decisions and why alternatives (a Python engine, Flutter, Tauri, Electron) were rejected. Read it before proposing a change of direction.

## Commands

The Cargo workspace lives in `core/`, not the repository root. Run everything from there.

```sh
cd core
cargo build
cargo test                                             # all crates
cargo test -p lantaran-rules host_pattern              # unit tests matching a name
cargo test -p lantaran-engine --test proxy tunnelled   # one integration test
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo run -p lantaran-cli -- --ssl example.com         # binary is named `lantaran`
```

CI runs `cargo fmt --all --check`, clippy with `-D warnings` and the tests on macOS, Linux and Windows. All three must pass, so anything Unix-only needs a `#[cfg(unix)]` guard. A separate Linux job measures coverage with `cargo-llvm-cov` and fails below 85% of lines in the engine and rules (the CLI's `main.rs` is excluded). The summary is on each run's page; raise the threshold in `.github/workflows/ci.yml` when coverage goes up.

To check HTTPS decryption by hand, use a throwaway data directory so the real root certificate is untouched:

```sh
cargo run -p lantaran-cli -- --listen 127.0.0.1:19090 --data-dir /tmp/lantaran-dev --ssl example.com
curl -x http://127.0.0.1:19090 --cacert /tmp/lantaran-dev/lantaran-ca.pem https://example.com/
```

Port 9090 is the default and is also Proxyman's default; pick another port if Proxyman is running.

### macOS app

```sh
apps/macos/scripts/build-core.sh --debug   # Rust engine -> XCFramework + Swift bindings
swift build --package-path apps/macos       # the app
swift test --package-path apps/macos        # its unit tests
apps/macos/scripts/build-app.sh             # release build, wrapped and signed as apps/macos/build/Lantaran.app
```

`build-core.sh` generates `apps/macos/Frameworks/` and `apps/macos/Sources/LantaranCore/lantaran_ffi.swift`; both are ignored by git, so run it after cloning and after every change to `core/ffi` or anything it calls. `SystemProxy` changes the Mac's own HTTP/HTTPS proxy (the iOS Simulator has none of its own). It must always be reversible: it writes every service's previous settings to `system-proxy-backup.json` before changing anything, restores on stop and on quit, and restores at launch if that file is still there. Test changes to it by force-killing the app with the proxy on and relaunching.

The app keeps its settings in the `com.github.tulungagung-developer.lantaran` defaults domain and shares the root certificate with the CLI in `~/Library/Application Support/Lantaran`.

## Architecture

Three crates, with dependencies pointing one way: `cli` → `engine` → `rules`.

- **`lantaran-rules`** is pure matching logic with no I/O and no dependencies. Every rule (today the SSL proxying list; later map, block and breakpoint rules) goes here so it can be unit tested alone.
- **`lantaran-engine`** owns the root certificate, runs the proxy and reports what it sees. Its public surface is deliberately small and message-shaped: construct an `Engine`, call methods on it, read `FlowEvent`s from the receiver. The desktop apps will use this same boundary over FFI, so do not expose shared mutable state or hudsucker types through it.
- **`lantaran-cli`** is a thin consumer that prints events. It exists to develop and verify the engine without a UI.
- **`lantaran-ffi`** wraps the engine for the desktop apps with [uniffi](https://github.com/mozilla/uniffi-rs): one `LantaranEngine` object, events delivered in batches to an `EventListener` the app implements. uniffi is pinned to 0.32 because `uniffi-bindgen-cs` (planned for Windows) targets that version.
- **`apps/macos`** is a Swift package. `AppModel` owns the engine and all state; `EventBridge` moves event batches to the main thread in order; the traffic list is an AppKit `NSTableView` (SwiftUI's `Table` cannot keep up), everything else is SwiftUI.

### How a request moves through the engine

The engine is built on [hudsucker](https://github.com/omjadas/hudsucker), which does the MITM plumbing. `core/engine/src/engine.rs` implements its `HttpHandler`:

1. `handle_request` runs for every request, including `CONNECT` and every request inside a decrypted connection. It first applies the guard (below), then emits `FlowEvent::Request` and wraps the request body in a `Tap`.
2. For `CONNECT`, `should_intercept_connect` asks the SSL proxying list whether to decrypt the host. If not, the connection is tunnelled byte for byte and reported once as `FlowEvent::Tunnel`. An empty list decrypts nothing.
3. `handle_response` emits `Response` (headers only) and wraps the response body in a `Tap`. `handle_error` emits `Failed` when the server could not be reached.
4. When the response `Tap` finishes it stores the captured body and emits `Completed`, or `Failed` if the body was cut off or the client went away.

hudsucker clones the handler per request and calls `handle_request` and then `handle_response`/`handle_error` on that same clone. The handler's `pending` field relies on this to tie a response to its request.

`Tap` (`core/engine/src/capture.rs`) forwards every frame unchanged as it arrives and keeps a copy up to the body limit. Never buffer a body before forwarding it: streaming responses and large downloads depend on this. A `Tap` finishes in `Drop` as well as on the final frame, because hyper stops polling a body once `is_end_stream` is true. Bodies stay out of events; front ends fetch them with `Engine::body`.

Events go through a bounded queue (`core/engine/src/events.rs`) and are read in batches from `EventStream::next_batch`. `Shared::emit` must never wait: when the queue is full the event is dropped and counted. Kept bodies live in `BodyStore` under a total budget, oldest dropped first, so `Engine::body` can return `None` for an old flow.

### The guard

`core/engine/src/guard.rs` refuses requests from other devices to addresses private to the machine running the proxy (loopback, unspecified, link-local). Without it, any device on the Wi-Fi could reach services bound to localhost here. Clients on the same machine are not restricted. It must stay in the path of every request, not only `CONNECT`, because an HTTP/2 request inside a decrypted connection names its own destination. It fails closed when a host name cannot be resolved. For the same reason the CLI listens on `127.0.0.1` unless `--listen` says otherwise.

### Upstream TLS

The proxy's own connections to servers use the client config from `core/engine/src/upstream.rs`, passed to hudsucker with `with_http_connector` and `with_websocket_connector`. It trusts the public web roots plus the OS store, and accepts any certificate for hosts set with `Engine::set_exempt_hosts`. If you change the connector, keep `enable_http2()` (without it upstream HTTP/2 silently stops) and leave ALPN unset on the `ClientConfig` (hyper-rustls panics if it is preset).

### Conventions that are easy to miss

- Use hudsucker's re-exports (`hudsucker::rcgen`, `hudsucker::hyper`, `hudsucker::rustls`) instead of adding those crates directly, so versions cannot drift apart.
- The root certificate is created once in the data directory and then reused. Never regenerate it implicitly: every device that trusts it would have to be set up again.
- Integration tests (`core/engine/tests/proxy.rs`) talk to a local stand-in server over real sockets and need no network. Keep it that way.
- Hosts are decrypted only when the user lists them. Do not add a path that decrypts by default.

## Commit messages

Follow the [Angular commit message format](https://github.com/angular/angular/blob/main/contributing-docs/commit-message-guidelines.md).

```
<type>(<scope>): <short summary>

<body>

<footer>
```

- **Type:** `build`, `ci`, `docs`, `feat`, `fix`, `perf`, `refactor` or `test`.
- **Scope** is the crate or app affected: `engine`, `rules`, `cli`, `ffi`, `macos`, and later `windows`, `linux`. Leave it out for documentation content and for changes that span everything, for example `docs: explain device setup` or `test: cover IPv6 hosts`.
- **Summary:** imperative present tense, not capitalized, no period at the end.
- **Body:** required for every type except `docs`, at least 20 characters, imperative present tense. Explain why the change is being made, not what the diff already shows.
- **Footer:** `Fixes #<issue>` or `Closes #<issue>`, and `BREAKING CHANGE: <summary>` or `DEPRECATED: <what>` followed by a blank line and the details.
- **Reverts** start with `revert: ` followed by the header of the reverted commit, and the body says `This reverts commit <SHA>` plus the reason.

Never add a `Co-authored-by` trailer, a "Generated with" line, or any other AI attribution to a commit message or pull request description. `.claude/settings.json` turns the automatic ones off; do not add them by hand either.

## Tracking work

- **Milestones are releases** (`v0.1.0`, `v0.2.0`). The milestone description says what the release delivers. Work not yet planned into a release has no milestone.
- **Issue types** (Feature, Bug, Task) classify issues. Do not create labels for areas or phases.
- **The Lantaran SDLC project** (org project #2) holds Status and Area for each issue. New issues are added to it automatically; Area has to be set by hand.
- Issue #34 is the pinned roadmap. Add new issues to it under the right release.

### Writing issues

Issues follow the forms in `.github/ISSUE_TEMPLATE/`. An issue created with `gh` or the API bypasses the form, so write the same sections by hand and set the type (`gh api -X PATCH repos/Tulungagung-Developer/lantaran/issues/<n> -f type=Feature`).

- **Feature:** `### Why`, `### What to build`, `### Done when`, `### Notes for implementers`, `### Depends on`.
- **Task:** the same, with `### What to do` in place of `### What to build`.
- **Bug:** what happened, what was expected, steps to reproduce, version, host OS, device, whether the host is on the SSL proxying list, and output.

"Notes for implementers" is where knowledge about the existing code goes: the function to start from, what hudsucker does at that point, and what has or has not been verified. When work on one issue uncovers something that affects another, add it to that issue's notes instead of leaving it in a commit message. State what was not tested as plainly as what was.
