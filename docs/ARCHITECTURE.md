# Architecture

## Goal

A desktop app that captures HTTP and HTTPS traffic from iOS and Android devices,
simulators and emulators. Capturing the desktop's own apps is not a goal.

## Shape

```
  phone / simulator / emulator
            │  HTTP proxy on port 9090
            ▼
  ┌──────────────────────────────┐
  │ core (Rust)                  │
  │   rules   → what to decrypt  │
  │   engine  → CA, MITM, events │
  └──────────────┬───────────────┘
                 │ commands in, flow events out
     ┌───────────┼───────────────┐
     ▼           ▼               ▼
  cli        macOS app      Windows / Linux apps
 (today)   (Swift, AppKit)      (later)
```

One engine, shared by every front end. Each desktop OS gets its own native UI
on top of it.

## Decisions

**Rust core.** The engine has to run on macOS, Windows and Linux. Swift's
networking stack is not mature on Windows, and a Python engine (mitmproxy) was
ruled out to keep the app small and free of a bundled runtime.

**Built on [hudsucker](https://github.com/omjadas/hudsucker).** It provides the
MITM plumbing: HTTP/1.1, HTTP/2, WebSocket upgrade, per-host leaf certificates
and a hook to decide per connection whether to decrypt. Lantaran adds the rules,
the flow model and everything user facing.

**Decrypt only listed hosts.** HTTPS to a host that is not on the SSL proxying
list is tunnelled byte for byte. Apps with pinned certificates keep working
until the user opts a host in.

**Native UI per OS.** Flutter, Tauri and Electron were considered and rejected
for not feeling native. The cost is one UI codebase per OS, so platforms are
added one at a time: macOS, then Windows, then Linux.

**Message-shaped boundary.** A front end sends commands (start, stop, change a
rule) and receives a stream of `FlowEvent`s. The same boundary works over FFI
([uniffi](https://github.com/mozilla/uniffi-rs)) or a local socket, so the UIs do
not depend on how the engine is hosted.

**No privileged helper to begin with.** Phones and the Android emulator reach
the proxy over the network, which needs nothing privileged on the desktop. The
iOS simulator is the exception: it uses the Mac's network settings, so it needs
the system proxy switched on.

## Crates

| Crate | Purpose |
| --- | --- |
| `lantaran-rules` | Pure matching logic with no I/O and no dependencies. |
| `lantaran-engine` | Root certificate storage, the proxy, and the event stream. |
| `lantaran-cli` | Headless proxy used for development and for verifying the engine. |

## Flow events

A readable flow produces `Request`, `Response` (headers arrived) and `Completed`
(the response body reached the client), all with the same flow id. `Failed`
replaces `Response` or `Completed` when the flow ends early. `Tunnel` reports
an HTTPS connection that was passed through unread.

Bodies are not carried in events. Each body is copied as it streams through,
up to a limit (5 MB by default), and is fetched afterwards by flow id with
`Engine::body`. `CapturedBody::decoded` undoes `Content-Encoding` for display;
the client always receives the original bytes.

Events are delivered in batches from an `EventStream` (up to 500 events, after
waiting at most 50 ms for more). The queue behind it holds at most 100,000
events; when a reader falls that far behind, new events are dropped and the
next batch says how many. Kept bodies share a memory budget (256 MB by
default) and the oldest are dropped first. Forwarding never waits on either.

## References

- [Rockxy](https://github.com/RockxyApp/Rockxy) (AGPL-3.0-or-later): native macOS
  UI. Individual components may be reused with their license headers kept.
- [FRTMProxy](https://github.com/ValentinoPalomba/FRTMProxy) (Unlicense): device
  setup flow and rule pipeline design.
