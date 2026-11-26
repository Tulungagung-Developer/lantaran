import Foundation
import LantaranCore

/// One request and what happened to it, built up from engine events.
struct Flow: Identifiable {
    enum State {
        case active, receiving, completed, failed, tunnel
    }

    let id: UInt64
    /// The client's IP address and source port.
    var client: String
    var clientPort: UInt16 = 0
    /// Where the sidebar files this flow. Set when the flow arrives, and
    /// refined once the process behind a local connection is known.
    var device = Device.mac
    var app = ""
    var bundlePath: String?
    var startedAt: Date
    var method: String
    var url: String
    var host: String
    var path: String
    var isHTTPS: Bool
    var isTunnel: Bool
    var requestVersion = ""
    var requestHeaders: [Header] = []
    var status: UInt16?
    var responseVersion = ""
    var responseHeaders: [Header] = []
    var elapsedMs: Double?
    var durationMs: Double?
    var requestSize: UInt64?
    var responseSize: UInt64?
    var error: String?
    var unverifiedUpstream = false
    /// Set when Map Local or Map Remote changed this flow.
    var mapping: String?

    var mappingKind: String? {
        guard let mapping else { return nil }
        if mapping.hasPrefix("Map Local") { return "Local" }
        if mapping.hasPrefix("Map Remote") { return "Remote" }
        return "Edited"
    }

    var state: State {
        if error != nil { return .failed }
        if isTunnel { return .tunnel }
        if durationMs != nil { return .completed }
        if status != nil { return .receiving }
        return .active
    }

    var responseContentType: String? {
        responseHeaders.first { $0.name.caseInsensitiveCompare("content-type") == .orderedSame }?.value
    }

    var userAgent: String? {
        requestHeaders.first { $0.name.caseInsensitiveCompare("user-agent") == .orderedSame }?.value
    }

    var requestContentType: String? {
        requestHeaders.first { $0.name.caseInsensitiveCompare("content-type") == .orderedSame }?.value
    }

    var queryItems: [URLQueryItem] {
        URLComponents(string: url)?.queryItems ?? []
    }

    static func request(
        id: UInt64, client: String, clientPort: UInt16, startedAt: Double, method: String,
        url: String, version: String, headers: [Header]
    ) -> Flow {
        let components = URLComponents(string: url)
        var flow = Flow(
            id: id,
            client: client,
            startedAt: Date(timeIntervalSince1970: startedAt),
            method: method,
            url: url,
            host: components?.host ?? "",
            path: components?.percentEncodedPath ?? url,
            isHTTPS: components?.scheme == "https",
            isTunnel: false
        )
        flow.clientPort = clientPort
        flow.requestVersion = version
        flow.requestHeaders = headers
        return flow
    }

    static func tunnel(id: UInt64, client: String, clientPort: UInt16, authority: String) -> Flow {
        let host = authority.split(separator: ":").dropLast().joined(separator: ":")
        var flow = Flow(
            id: id,
            client: client,
            startedAt: Date(),
            method: "CONNECT",
            url: "https://\(authority)",
            host: host.isEmpty ? authority : host,
            path: "",
            isHTTPS: true,
            isTunnel: true
        )
        flow.clientPort = clientPort
        return flow
    }
}
