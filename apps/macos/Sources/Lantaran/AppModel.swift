import AppKit
import Foundation
import LantaranCore
import Observation

enum SidebarItem: Hashable {
    case all
    case client(String)
    case host(String)
}

/// Everything the window shows, and the one engine behind it.
@MainActor
@Observable
final class AppModel {
    private(set) var engine: LantaranEngine?
    private(set) var startupError: String?

    // Captured traffic. `visible` holds indexes into `flows` that pass the
    // current filter, in arrival order. `revision` changes whenever either
    // changes, so the table knows to redraw.
    private(set) var flows: [Flow] = []
    private(set) var visible: [Int] = []
    private(set) var revision = 0
    private var indexByID: [UInt64: Int] = [:]
    private(set) var droppedEvents: UInt64 = 0

    var selectedFlowID: UInt64?
    var followNewest = true
    var showConnectDevice = false
    var showSSLProxying = false

    var filterText = "" {
        didSet { refilter() }
    }

    var sidebarSelection: SidebarItem? = .all {
        didSet { refilter() }
    }

    // Proxy state.
    private(set) var running = false
    private(set) var boundAddress: String?
    var lastError: String?

    // Settings, kept in UserDefaults.
    var port: Int {
        didSet { defaults.set(port, forKey: Keys.port) }
    }

    var acceptOtherDevices: Bool {
        didSet { defaults.set(acceptOtherDevices, forKey: Keys.acceptOtherDevices) }
    }

    private(set) var sslHosts: [String]
    private(set) var insecureHosts: [String]

    private let defaults = UserDefaults.standard
    private var bridge: EventBridge?

    private enum Keys {
        static let port = "port"
        static let acceptOtherDevices = "acceptOtherDevices"
        static let sslHosts = "sslProxyingHosts"
        static let insecureHosts = "insecureHosts"
    }

    init() {
        // Like the CLI, only this Mac can connect until the user opts in,
        // which the Connect Device sheet offers.
        defaults.register(defaults: [Keys.port: 9090, Keys.acceptOtherDevices: false])
        port = defaults.integer(forKey: Keys.port)
        acceptOtherDevices = defaults.bool(forKey: Keys.acceptOtherDevices)
        sslHosts = defaults.stringArray(forKey: Keys.sslHosts) ?? []
        insecureHosts = defaults.stringArray(forKey: Keys.insecureHosts) ?? []

        do {
            let engine = try LantaranEngine(dataDir: Self.dataDirectory.path)
            self.engine = engine
            let bridge = EventBridge(model: self)
            self.bridge = bridge
            engine.setListener(listener: bridge)
            pushRules()
            start()
        } catch {
            startupError = "Lantaran could not start: \(error.localizedDescription)"
        }
    }

    /// Shared with the command line tool, so both use the same certificate.
    static var dataDirectory: URL {
        FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("Lantaran", isDirectory: true)
    }

    // MARK: Proxy

    var listenAddress: String {
        "\(acceptOtherDevices ? "0.0.0.0" : "127.0.0.1"):\(port)"
    }

    /// The address a phone should use, when the proxy accepts other devices.
    var addressForDevices: String? {
        guard running, acceptOtherDevices, let ip = localNetworkAddress() else { return nil }
        return "\(ip):\(port)"
    }

    var statusText: String {
        guard running, let boundAddress else { return "Not running" }
        if let address = addressForDevices {
            return "Listening on \(address)"
        }
        return "Listening on \(boundAddress)"
    }

    func start() {
        guard let engine else { return }
        do {
            boundAddress = try engine.start(address: listenAddress)
            running = true
            lastError = nil
        } catch {
            running = false
            boundAddress = nil
            let message = Self.message(error)
            lastError = message.localizedCaseInsensitiveContains("address already in use")
                ? "Port \(port) is already in use, perhaps by Proxyman or another copy of Lantaran. Quit it, or choose another port in Settings (⌘,)."
                : message
        }
    }

    func stop() {
        engine?.stop()
        running = false
        boundAddress = nil
    }

    func restart() {
        stop()
        start()
    }

    // MARK: Rules

    func setSSLHosts(_ hosts: [String]) {
        sslHosts = Self.clean(hosts)
        defaults.set(sslHosts, forKey: Keys.sslHosts)
        pushRules()
    }

    func setInsecureHosts(_ hosts: [String]) {
        insecureHosts = Self.clean(hosts)
        defaults.set(insecureHosts, forKey: Keys.insecureHosts)
        pushRules()
    }

    func enableSSL(for host: String) {
        guard !sslHosts.contains(host) else { return }
        setSSLHosts(sslHosts + [host])
    }

    func isDecrypted(_ host: String) -> Bool {
        sslHosts.contains { pattern in Self.matches(pattern: pattern, host: host) }
    }

    private func pushRules() {
        engine?.setSslProxying(include: sslHosts, exclude: [])
        engine?.setExemptHosts(hosts: insecureHosts)
    }

    private static func clean(_ hosts: [String]) -> [String] {
        var seen = Set<String>()
        return hosts
            .map { $0.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() }
            .filter { !$0.isEmpty && seen.insert($0).inserted }
    }

    /// Same rule as the engine: `*` matches any run of characters.
    static func matches(pattern: String, host: String) -> Bool {
        let regex = "^" + NSRegularExpression.escapedPattern(for: pattern)
            .replacingOccurrences(of: "\\*", with: ".*") + "$"
        return host.range(of: regex, options: [.regularExpression, .caseInsensitive]) != nil
    }

    // MARK: Events

    func apply(_ events: [FlowEventRecord], dropped: UInt64) {
        droppedEvents += dropped
        var appended: [Int] = []

        for event in events {
            switch event {
            case let .request(id, client, startedAt, method, url, version, headers):
                appended.append(insert(.request(
                    id: id, client: client, startedAt: startedAt, method: method,
                    url: url, version: version, headers: headers)))
            case let .tunnel(id, client, authority):
                appended.append(insert(.tunnel(id: id, client: client, authority: authority)))
            case let .response(id, status, version, headers, elapsedMs, unverified):
                update(id) {
                    $0.status = status
                    $0.responseVersion = version
                    $0.responseHeaders = headers
                    $0.elapsedMs = elapsedMs
                    $0.unverifiedUpstream = unverified
                }
            case let .completed(id, requestSize, responseSize, durationMs):
                update(id) {
                    $0.requestSize = requestSize
                    $0.responseSize = responseSize
                    $0.durationMs = durationMs
                }
            case let .failed(id, error):
                update(id) { $0.error = error }
            }
        }

        visible.append(contentsOf: appended.filter { passesFilter(flows[$0]) })
        revision += 1
    }

    private func insert(_ flow: Flow) -> Int {
        flows.append(flow)
        let index = flows.count - 1
        indexByID[flow.id] = index
        return index
    }

    private func update(_ id: UInt64, _ change: (inout Flow) -> Void) {
        // Events for a flow whose first event was dropped have nothing to
        // attach to; they are ignored.
        guard let index = indexByID[id] else { return }
        change(&flows[index])
    }

    func clear() {
        flows.removeAll()
        visible.removeAll()
        indexByID.removeAll()
        selectedFlowID = nil
        droppedEvents = 0
        revision += 1
    }

    func flow(withID id: UInt64?) -> Flow? {
        guard let id, let index = indexByID[id] else { return nil }
        return flows[index]
    }

    // MARK: Filtering

    private func refilter() {
        visible = flows.indices.filter { passesFilter(flows[$0]) }
        revision += 1
    }

    private func passesFilter(_ flow: Flow) -> Bool {
        switch sidebarSelection {
        case .client(let client) where flow.client != client: return false
        case .host(let host) where flow.host != host: return false
        default: break
        }
        let text = filterText.trimmingCharacters(in: .whitespaces)
        if text.isEmpty { return true }
        return flow.url.localizedCaseInsensitiveContains(text)
            || flow.method.localizedCaseInsensitiveContains(text)
            || (flow.status.map { String($0).hasPrefix(text) } ?? false)
    }

    /// Flow counts per device and per host, for the sidebar.
    var clients: [(String, Int)] { counts(\.client) }
    var hosts: [(String, Int)] { counts(\.host) }

    private func counts(_ key: KeyPath<Flow, String>) -> [(String, Int)] {
        var counts: [String: Int] = [:]
        for flow in flows { counts[flow[keyPath: key], default: 0] += 1 }
        return counts.sorted { $0.key.localizedStandardCompare($1.key) == .orderedAscending }
    }

    // MARK: Bodies

    /// The body for display: decompressed when it was compressed.
    func body(of flow: Flow, response: Bool) -> (data: Data, record: BodyRecord)? {
        guard let engine, let record = engine.body(id: flow.id, response: response) else {
            return nil
        }
        let data = engine.decodedBody(id: flow.id, response: response) ?? record.bytes
        return (data, record)
    }

    func revealCertificate() {
        guard let path = engine?.certificatePath() else { return }
        NSWorkspace.shared.activateFileViewerSelecting([URL(fileURLWithPath: path)])
    }

    static func message(_ error: Error) -> String {
        if case let LantaranError.Failed(message) = error { return message }
        return error.localizedDescription
    }
}
