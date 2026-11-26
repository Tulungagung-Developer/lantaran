import Foundation

enum Curl {
    /// A curl command that repeats the request.
    static func command(for flow: Flow, body: Data?) -> String {
        var parts = ["curl", quote(flow.url)]
        if flow.method != "GET" {
            parts.append("-X \(flow.method)")
        }
        // HTTP/2 pseudo-headers and headers curl sets itself are left out.
        let skipped: Set<String> = ["host", "content-length", "connection", "accept-encoding"]
        for header in flow.requestHeaders where !header.name.hasPrefix(":") && !skipped.contains(header.name.lowercased()) {
            parts.append("-H " + quote("\(header.name): \(header.value)"))
        }
        if let body, !body.isEmpty {
            if let text = String(data: body, encoding: .utf8) {
                parts.append("--data-raw " + quote(text))
            } else {
                parts.append("--data-binary @request-body.bin")
            }
        }
        return parts.joined(separator: " \\\n  ")
    }

    private static func quote(_ text: String) -> String {
        "'" + text.replacingOccurrences(of: "'", with: "'\\''") + "'"
    }
}
