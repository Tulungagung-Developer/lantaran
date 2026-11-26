import Foundation
import LantaranCore

/// A Map Local rule as the app stores it.
struct LocalRule: Codable, Identifiable, Hashable {
    var id = UUID()
    var enabled = true
    var pattern = ""
    /// Empty matches every method.
    var method = ""
    var file = ""
    var status = 200
    /// Empty means guess from the file's extension.
    var contentType = ""

    var engineRule: MapLocalRule {
        MapLocalRule(
            pattern: pattern,
            method: method.isEmpty ? nil : method,
            file: (file as NSString).expandingTildeInPath,
            status: UInt16(clamping: status),
            contentType: contentType.isEmpty ? nil : contentType
        )
    }
}

/// A Map Remote rule as the app stores it. `target` is a URL whose scheme,
/// host, port and path replace the request's; a path of "/" or none keeps
/// the request's path.
struct RemoteRule: Codable, Identifiable, Hashable {
    var id = UUID()
    var enabled = true
    var pattern = ""
    var method = ""
    var target = ""

    var engineRule: MapRemoteRule? {
        guard let url = URLComponents(string: target), let scheme = url.scheme, let host = url.host,
              !host.isEmpty
        else { return nil }
        let path = url.percentEncodedPath
        return MapRemoteRule(
            pattern: pattern,
            method: method.isEmpty ? nil : method,
            scheme: scheme,
            host: host,
            port: url.port.map { UInt16(clamping: $0) },
            path: path.isEmpty || path == "/" ? nil : path
        )
    }

    var isValid: Bool { engineRule != nil && !pattern.isEmpty }
}

enum MapFiles {
    /// Where response bodies saved for Map Local go.
    static var directory: URL {
        AppModel.dataDirectory.appendingPathComponent("Map Local", isDirectory: true)
    }

    static func fileExtension(for contentType: String?) -> String {
        let type = contentType?.lowercased() ?? ""
        if type.contains("json") { return "json" }
        if type.contains("html") { return "html" }
        if type.contains("xml") { return "xml" }
        if type.contains("javascript") { return "js" }
        if type.contains("css") { return "css" }
        if type.contains("png") { return "png" }
        if type.contains("jpeg") { return "jpg" }
        if type.contains("gif") { return "gif" }
        if type.contains("webp") { return "webp" }
        if type.hasPrefix("text/") { return "txt" }
        return "bin"
    }

    /// A readable file name for a flow's saved response.
    static func fileName(for flow: Flow, contentType: String?) -> String {
        let last = URLComponents(string: flow.url)?.path.split(separator: "/").last.map(String.init) ?? "response"
        let safe = (flow.host + "-" + last)
            .map { $0.isLetter || $0.isNumber || "-_.".contains($0) ? $0 : "_" }
        return String(safe.prefix(80)) + "-\(flow.id)." + fileExtension(for: contentType)
    }
}
