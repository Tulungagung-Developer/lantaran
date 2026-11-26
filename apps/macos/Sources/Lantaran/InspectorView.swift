import AppKit
import LantaranCore
import SwiftUI

struct InspectorView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        if let flow = model.flow(withID: model.selectedFlowID) {
            VStack(spacing: 0) {
                FlowHeader(flow: flow)
                Divider()
                if flow.isTunnel {
                    NotDecryptedView(flow: flow)
                } else {
                    HSplitView {
                        MessagePane(flow: flow, response: false)
                            .frame(minWidth: 260)
                        MessagePane(flow: flow, response: true)
                            .frame(minWidth: 260)
                    }
                }
            }
        } else {
            ContentUnavailableView(
                "No Request Selected",
                systemImage: "arrow.left.arrow.right",
                description: Text("Select a row to see its headers and body.")
            )
        }
    }
}

private struct FlowHeader: View {
    let flow: Flow

    var body: some View {
        HStack(spacing: 8) {
            Badge(text: flow.method, color: .secondary)
            if let status = flow.status {
                Badge(text: String(status), color: status >= 400 ? .red : .green)
            } else if flow.error != nil {
                Badge(text: "FAILED", color: .red)
            } else if flow.isTunnel {
                Badge(text: "NOT DECRYPTED", color: .secondary)
            } else {
                Badge(text: "ACTIVE", color: .yellow)
            }
            if flow.unverifiedUpstream {
                Badge(text: "CERTIFICATE NOT CHECKED", color: .orange)
                    .help("This host is exempt from server certificate checks.")
            }
            Text(flow.url)
                .lineLimit(1)
                .truncationMode(.middle)
                .textSelection(.enabled)
            Spacer()
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
    }
}

private struct Badge: View {
    let text: String
    let color: Color

    var body: some View {
        Text(text)
            .font(.caption.weight(.semibold))
            .padding(.horizontal, 6)
            .padding(.vertical, 2)
            .background(color.opacity(0.18), in: RoundedRectangle(cornerRadius: 4))
            .foregroundStyle(color == .secondary ? Color.primary : color)
    }
}

private struct NotDecryptedView: View {
    @Environment(AppModel.self) private var model
    let flow: Flow

    var body: some View {
        VStack(spacing: 12) {
            Image(systemName: "lock").font(.largeTitle).foregroundStyle(.secondary)
            Text("HTTPS Not Decrypted").font(.title3.weight(.semibold))
            Text("This connection to \(flow.host) passed through unread.\nAdd the host to the SSL Proxying List to see its requests.")
                .multilineTextAlignment(.center)
                .foregroundStyle(.secondary)
            if model.isDecrypted(flow.host) {
                Label("\(flow.host) is on the list. New connections from the device will be decrypted.",
                      systemImage: "checkmark.circle")
                    .foregroundStyle(.green)
            } else {
                Button("Enable SSL Proxying for \(flow.host)") { model.enableSSL(for: flow.host) }
                    .controlSize(.large)
                if let parent = parentDomain(flow.host) {
                    Button("Enable for all of *.\(parent)") { model.enableSSL(for: "*.\(parent)") }
                }
            }
            Text("Applies to new connections. Apps often keep a connection open, so you may need to relaunch the app on the device.")
                .font(.caption)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
        }
        .padding()
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private func parentDomain(_ host: String) -> String? {
        let parts = host.split(separator: ".")
        guard parts.count > 2 else { return nil }
        return parts.suffix(2).joined(separator: ".")
    }
}

private enum Tab: String, CaseIterable, Identifiable {
    case headers = "Header", query = "Query", body = "Body", raw = "Raw"
    var id: String { rawValue }
}

private struct MessagePane: View {
    @Environment(AppModel.self) private var model
    let flow: Flow
    let response: Bool
    @State private var tab: Tab = .headers

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Text(response ? "Response" : "Request").font(.headline)
                Picker("", selection: $tab) {
                    ForEach(tabs) { Text($0.rawValue).tag($0) }
                }
                .pickerStyle(.segmented)
                .labelsHidden()
                .fixedSize()
                Spacer()
            }
            .padding(8)
            Divider()
            content
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        }
    }

    private var tabs: [Tab] {
        response ? [.headers, .body, .raw] : Tab.allCases
    }

    @ViewBuilder
    private var content: some View {
        if response, flow.status == nil {
            Group {
                if let error = flow.error {
                    Label(error, systemImage: "xmark.octagon").foregroundStyle(.red).textSelection(.enabled)
                } else {
                    Label("Waiting for the response", systemImage: "hourglass").foregroundStyle(.secondary)
                }
            }
            .padding()
        } else {
            switch tab {
            case .headers:
                PairsTable(pairs: (response ? flow.responseHeaders : flow.requestHeaders).map { ($0.name, $0.value) })
            case .query:
                PairsTable(pairs: flow.queryItems.map { ($0.name, $0.value ?? "") })
            case .body:
                BodyView(flow: flow, response: response, raw: false)
            case .raw:
                BodyView(flow: flow, response: response, raw: true)
            }
        }
    }
}

private struct PairsTable: View {
    let pairs: [(String, String)]

    var body: some View {
        if pairs.isEmpty {
            Text("None").foregroundStyle(.secondary).padding()
        } else {
            Table(pairs.indices.map { Pair(index: $0, key: pairs[$0].0, value: pairs[$0].1) }) {
                TableColumn("Key") { Text($0.key).textSelection(.enabled) }
                    .width(min: 80, ideal: 180)
                TableColumn("Value") { Text($0.value).textSelection(.enabled) }
            }
        }
    }

    private struct Pair: Identifiable {
        let index: Int
        let key: String
        let value: String
        var id: Int { index }
    }
}

private struct BodyView: View {
    @Environment(AppModel.self) private var model
    let flow: Flow
    let response: Bool
    let raw: Bool
    @State private var loaded: Loaded?

    private struct Loaded {
        var text: String?
        var image: NSImage?
        var notes: [String]
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if let loaded {
                ForEach(loaded.notes, id: \.self) { note in
                    Label(note, systemImage: "info.circle")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                        .padding(.horizontal, 8)
                        .padding(.top, 6)
                }
                if let image = loaded.image, !raw {
                    ScrollView([.horizontal, .vertical]) {
                        Image(nsImage: image).padding()
                    }
                } else {
                    TextPane(text: loaded.text ?? "")
                }
            } else {
                ProgressView().padding()
            }
        }
        .task(id: loadKey) { load() }
    }

    /// Reload when the selection changes or the body finishes.
    private var loadKey: String {
        "\(flow.id)-\(response)-\(raw)-\(flow.durationMs != nil)-\(flow.error != nil)"
    }

    private func load() {
        var notes: [String] = []
        let headers = response ? flow.responseHeaders : flow.requestHeaders
        let firstLine = response
            ? "\(flow.responseVersion) \(flow.status.map(String.init) ?? "")"
            : "\(flow.method) \(flow.path.isEmpty ? flow.url : flow.path) \(flow.requestVersion)"

        guard let (data, record) = model.body(of: flow, response: response) else {
            let pending = flow.durationMs == nil && flow.error == nil
            let message = pending ? "The body is still arriving." : "No body was kept for this message."
            loaded = Loaded(text: raw ? head(firstLine, headers) : "", notes: [message])
            return
        }

        if record.truncated {
            notes.append("Only the first \(ByteCountFormatter.string(fromByteCount: Int64(record.bytes.count), countStyle: .file)) of \(ByteCountFormatter.string(fromByteCount: Int64(record.size), countStyle: .file)) were kept.")
        }
        if record.incomplete {
            notes.append("The body ended before it was complete.")
        }
        if let encoding = record.contentEncoding {
            notes.append("Shown decoded from \(encoding).")
        }

        let contentType = (response ? flow.responseContentType : flow.requestContentType)?.lowercased() ?? ""
        if !raw, contentType.hasPrefix("image/"), let image = NSImage(data: data) {
            loaded = Loaded(image: image, notes: notes)
            return
        }

        let text = BodyFormatter.text(data, contentType: contentType, pretty: !raw)
        loaded = Loaded(text: raw ? head(firstLine, headers) + text : text, notes: notes)
    }

    private func head(_ firstLine: String, _ headers: [Header]) -> String {
        ([firstLine] + headers.map { "\($0.name): \($0.value)" }).joined(separator: "\n") + "\n\n"
    }
}

enum BodyFormatter {
    /// Text for display: indented JSON, plain text, or a hex dump for
    /// binary data.
    static func text(_ data: Data, contentType: String, pretty: Bool) -> String {
        if data.isEmpty { return "" }
        guard let string = String(data: data, encoding: .utf8) else {
            return hexDump(data.prefix(64 * 1024)) + (data.count > 64 * 1024 ? "\n…" : "")
        }
        if pretty, contentType.contains("json") || looksLikeJSON(string) {
            return indentJSON(string)
        }
        return string
    }

    private static func looksLikeJSON(_ text: String) -> Bool {
        guard let first = text.first(where: { !$0.isWhitespace }) else { return false }
        return first == "{" || first == "["
    }

    /// Re-indents JSON text without parsing it, so key order is kept and
    /// invalid JSON still shows.
    static func indentJSON(_ text: String) -> String {
        var out = ""
        out.reserveCapacity(text.count + text.count / 4)
        var depth = 0
        var inString = false
        var escaped = false
        let indent = "  "

        func newline() {
            out.append("\n")
            out.append(String(repeating: indent, count: max(depth, 0)))
        }

        var chars = Array(text)
        var i = 0
        while i < chars.count {
            let c = chars[i]
            if inString {
                out.append(c)
                if escaped { escaped = false } else if c == "\\" { escaped = true } else if c == "\"" { inString = false }
                i += 1
                continue
            }
            switch c {
            case "\"":
                inString = true
                out.append(c)
            case "{", "[":
                // Keep empty containers on one line.
                var j = i + 1
                while j < chars.count, chars[j].isWhitespace { j += 1 }
                if j < chars.count, chars[j] == (c == "{" ? "}" : "]") {
                    out.append(c); out.append(chars[j]); i = j
                } else {
                    out.append(c); depth += 1; newline()
                }
            case "}", "]":
                depth -= 1; newline(); out.append(c)
            case ",":
                out.append(c); newline()
            case ":":
                out.append(": ")
            case " ", "\n", "\r", "\t":
                break
            default:
                out.append(c)
            }
            i += 1
        }
        chars.removeAll()
        return out
    }

    static func hexDump(_ data: Data) -> String {
        var lines: [String] = []
        let bytes = [UInt8](data)
        for offset in stride(from: 0, to: bytes.count, by: 16) {
            let row = bytes[offset..<min(offset + 16, bytes.count)]
            let hex = row.map { String(format: "%02x", $0) }.joined(separator: " ")
            let ascii = String(row.map { (0x20...0x7e).contains($0) ? Character(UnicodeScalar($0)) : "." })
            lines.append(String(format: "%08x  ", offset) + hex.padding(toLength: 48, withPad: " ", startingAt: 0) + "  " + ascii)
        }
        return lines.joined(separator: "\n")
    }
}

/// A read-only, selectable, searchable text view. NSTextView handles
/// multi-megabyte bodies that a SwiftUI Text would choke on.
struct TextPane: NSViewRepresentable {
    let text: String

    func makeNSView(context: Context) -> NSScrollView {
        let scroll = NSTextView.scrollableTextView()
        guard let textView = scroll.documentView as? NSTextView else { return scroll }
        textView.isEditable = false
        textView.isSelectable = true
        textView.usesFindBar = true
        textView.isIncrementalSearchingEnabled = true
        textView.font = .monospacedSystemFont(ofSize: NSFont.systemFontSize - 1, weight: .regular)
        textView.textContainerInset = NSSize(width: 6, height: 6)
        textView.isAutomaticQuoteSubstitutionEnabled = false
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        guard let textView = scroll.documentView as? NSTextView, textView.string != text else { return }
        textView.string = text
        textView.scroll(.zero)
    }
}
