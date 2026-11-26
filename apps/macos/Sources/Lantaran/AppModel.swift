import AppKit
import Foundation
import LantaranCore
import Observation

enum SidebarItem: Hashable {
    case all
    case device(Device)
    case app(Device, String)
    case appDomain(Device, String, String)
    case domain(String)
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

    // The sidebar's counts, and what is known about where flows come from.
    private(set) var tree = SidebarTree()
    private(set) var simulatorNames: [String: String] = [:]
    /// "iPhone", "iPad" or "Android" per remote address, from a User-Agent.
    private(set) var remoteKinds: [String: String] = [:]
    @ObservationIgnored private var liveProcesses: [UInt16: ClientProcess] = [:]
    @ObservationIgnored private var liveProcessesScannedAt = Date.distantPast
    @ObservationIgnored private var waitingPorts: [UInt16: [UInt64]] = [:]
    @ObservationIgnored private var scanning = false
    @ObservationIgnored private var scanAgain = false
    @ObservationIgnored private var loadingSimulatorNames = false
    @ObservationIgnored private let scanner = ClientProcessScanner()
    @ObservationIgnored private let scanQueue = DispatchQueue(label: "lantaran.client-processes", qos: .userInitiated)

    var selectedFlowID: UInt64?
    var followNewest = true
    var showConnectDevice = false
    var connectDevicePlatform = DevicePlatform.ios
    var showSSLProxying = false
    var showMapLocal = false
    var showMapRemote = false
    /// A rule being created or edited from the table or a sheet.
    var editingLocalRule: LocalRule?
    var editingRemoteRule: RemoteRule?

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

    /// Whether the Mac's own HTTP and HTTPS proxy points at Lantaran, which
    /// the iOS Simulator needs.
    private(set) var systemProxyOn = false
    private(set) var changingSystemProxy = false

    // iOS simulators.
    private(set) var bootedSimulators: [Simulators.Device] = []
    private(set) var simulatorsWithCertificate: Set<String> = []
    private(set) var simulatorMessage: String?

    /// The model of the running app, for the app delegate.
    nonisolated(unsafe) static weak var active: AppModel?

    // Settings, kept in UserDefaults.
    var port: Int {
        didSet { defaults.set(port, forKey: Keys.port) }
    }

    var acceptOtherDevices: Bool {
        didSet { defaults.set(acceptOtherDevices, forKey: Keys.acceptOtherDevices) }
    }

    private(set) var sslHosts: [String]
    private(set) var insecureHosts: [String]
    private(set) var localRules: [LocalRule]
    private(set) var remoteRules: [RemoteRule]

    private let defaults = UserDefaults.standard
    private var bridge: EventBridge?

    private enum Keys {
        static let port = "port"
        static let acceptOtherDevices = "acceptOtherDevices"
        static let sslHosts = "sslProxyingHosts"
        static let insecureHosts = "insecureHosts"
        static let localRules = "mapLocalRules"
        static let remoteRules = "mapRemoteRules"
    }

    init() {
        // Like the CLI, only this Mac can connect until the user opts in,
        // which the Connect Device sheet offers.
        defaults.register(defaults: [Keys.port: 9090, Keys.acceptOtherDevices: false])
        port = defaults.integer(forKey: Keys.port)
        acceptOtherDevices = defaults.bool(forKey: Keys.acceptOtherDevices)
        sslHosts = defaults.stringArray(forKey: Keys.sslHosts) ?? []
        insecureHosts = defaults.stringArray(forKey: Keys.insecureHosts) ?? []
        localRules = Self.decode([LocalRule].self, defaults.data(forKey: Keys.localRules)) ?? []
        remoteRules = Self.decode([RemoteRule].self, defaults.data(forKey: Keys.remoteRules)) ?? []

        Self.active = self

        // A backup left behind means Lantaran last quit without putting the
        // Mac's proxy back, which leaves the Mac pointing at a proxy that is
        // not running. Fix that first.
        if SystemProxy.isRedirected {
            do {
                try SystemProxy.restore()
            } catch {
                lastError = "Lantaran could not restore this Mac's earlier proxy settings: \(error.localizedDescription). Check System Settings › Network › Details › Proxies."
            }
        }

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
    nonisolated static var dataDirectory: URL {
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
        let address = addressForDevices ?? boundAddress
        return systemProxyOn ? "Listening on \(address) · Mac proxy on" : "Listening on \(address)"
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
        // A Mac proxy pointing at a stopped proxy would cut the Mac off.
        if systemProxyOn {
            setSystemProxy(false)
        }
        engine?.stop()
        running = false
        boundAddress = nil
    }

    /// Restarts on the current port and settings. The Mac's proxy, if it
    /// points at Lantaran, follows to the new port.
    func restart() {
        engine?.stop()
        running = false
        boundAddress = nil
        start()
        if systemProxyOn {
            if running {
                setSystemProxy(true)
            } else {
                setSystemProxy(false)
            }
        }
    }

    /// Called as the app quits: puts the Mac's proxy back. Runs
    /// synchronously because the process is about to end.
    func prepareToQuit() {
        guard SystemProxy.isRedirected else { return }
        try? SystemProxy.restore()
    }

    // MARK: Mac proxy and simulators

    func setSystemProxy(_ on: Bool) {
        // Turning it on again is allowed: it re-points the Mac's proxy at
        // the current port.
        guard on || systemProxyOn else { return }
        if on, !running {
            lastError = "Start the proxy before routing this Mac's traffic through it."
            return
        }
        changingSystemProxy = true
        let port = port
        Task.detached {
            let result: Result<Void, Error> = Result {
                if on {
                    try SystemProxy.redirect(to: "127.0.0.1", port: port)
                } else {
                    try SystemProxy.restore()
                }
            }
            await MainActor.run {
                self.changingSystemProxy = false
                switch result {
                case .success:
                    self.systemProxyOn = on
                case .failure(let error):
                    self.systemProxyOn = SystemProxy.isRedirected
                    self.lastError = on
                        ? "Could not change this Mac's proxy settings: \(error.localizedDescription)"
                        : "Could not restore this Mac's proxy settings: \(error.localizedDescription). Check System Settings › Network › Details › Proxies."
                }
            }
        }
    }

    func refreshSimulators() {
        Task.detached {
            let devices = Simulators.booted()
            await MainActor.run { self.bootedSimulators = devices }
        }
    }

    func installCertificate(in device: Simulators.Device) {
        guard let path = engine?.certificatePath() else { return }
        simulatorMessage = nil
        Task.detached {
            let result = Result { try Simulators.installCertificate(path, in: device) }
            await MainActor.run {
                switch result {
                case .success:
                    self.simulatorsWithCertificate.insert(device.id)
                case .failure(let error):
                    self.simulatorMessage = "Could not install the certificate in \(device.name): \(error.localizedDescription)"
                }
            }
        }
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

    func setLocalRules(_ rules: [LocalRule]) {
        localRules = rules
        defaults.set(try? JSONEncoder().encode(rules), forKey: Keys.localRules)
        pushRules()
    }

    func setRemoteRules(_ rules: [RemoteRule]) {
        remoteRules = rules
        defaults.set(try? JSONEncoder().encode(rules), forKey: Keys.remoteRules)
        pushRules()
    }

    /// Adds `rule`, or replaces the rule with the same id.
    func saveLocalRule(_ rule: LocalRule) {
        var rules = localRules
        if let index = rules.firstIndex(where: { $0.id == rule.id }) { rules[index] = rule } else { rules.append(rule) }
        setLocalRules(rules)
    }

    func saveRemoteRule(_ rule: RemoteRule) {
        var rules = remoteRules
        if let index = rules.firstIndex(where: { $0.id == rule.id }) { rules[index] = rule } else { rules.append(rule) }
        setRemoteRules(rules)
    }

    /// Saves the flow's response body to a file and starts a Map Local rule
    /// for its URL, ready to edit.
    func startMapLocal(from flow: Flow) {
        var rule = LocalRule(pattern: Self.pattern(for: flow), status: Int(flow.status ?? 200))
        let contentType = flow.responseContentType
        rule.contentType = contentType ?? ""
        do {
            try FileManager.default.createDirectory(at: MapFiles.directory, withIntermediateDirectories: true)
            let file = MapFiles.directory.appendingPathComponent(MapFiles.fileName(for: flow, contentType: contentType))
            let data = body(of: flow, response: true)?.data ?? Data()
            try data.write(to: file)
            rule.file = file.path
        } catch {
            lastError = "Could not save the response for Map Local: \(error.localizedDescription)"
        }
        editingLocalRule = rule
    }

    func startMapRemote(from flow: Flow) {
        editingRemoteRule = RemoteRule(pattern: Self.pattern(for: flow))
    }

    /// A pattern for a flow's URL that ignores the query.
    static func pattern(for flow: Flow) -> String {
        guard var components = URLComponents(string: flow.url) else { return flow.url }
        let hadQuery = components.query != nil
        components.query = nil
        components.fragment = nil
        let base = components.string ?? flow.url
        return hadQuery ? base + "?*" : base
    }

    private func pushRules() {
        engine?.setSslProxying(include: sslHosts, exclude: [])
        engine?.setExemptHosts(hosts: insecureHosts)
        engine?.setMapRules(
            local: localRules.filter(\.enabled).map(\.engineRule),
            remote: remoteRules.filter(\.enabled).compactMap(\.engineRule)
        )
    }

    private static func decode<T: Decodable>(_ type: T.Type, _ data: Data?) -> T? {
        data.flatMap { try? JSONDecoder().decode(type, from: $0) }
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
            case let .request(id, client, clientPort, startedAt, method, url, version, headers):
                appended.append(insert(.request(
                    id: id, client: client, clientPort: clientPort, startedAt: startedAt,
                    method: method, url: url, version: version, headers: headers)))
            case let .tunnel(id, client, clientPort, authority):
                appended.append(insert(.tunnel(
                    id: id, client: client, clientPort: clientPort, authority: authority)))
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
            case let .rewritten(id, description):
                update(id) { $0.mapping = description }
            }
        }

        visible.append(contentsOf: appended.filter { passesFilter(flows[$0]) })
        revision += 1
    }

    private func insert(_ flow: Flow) -> Int {
        var flow = flow
        locate(&flow)
        tree.add(flow)
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
        tree.removeAll()
        waitingPorts.removeAll()
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
        case .device(let device) where flow.device != device:
            return false
        case .app(let device, let app) where flow.device != device || flow.app != app:
            return false
        case .appDomain(let device, let app, let host)
            where flow.device != device || flow.app != app || flow.host != host:
            return false
        case .domain(let host) where flow.host != host:
            return false
        default: break
        }
        let text = filterText.trimmingCharacters(in: .whitespaces)
        if text.isEmpty { return true }
        return flow.url.localizedCaseInsensitiveContains(text)
            || flow.method.localizedCaseInsensitiveContains(text)
            || (flow.status.map { String($0).hasPrefix(text) } ?? false)
    }

    // MARK: Where flows come from

    /// Files a new flow under a device and app. Traffic from another device
    /// is named from its User-Agent. Local traffic (this Mac, simulators,
    /// emulators) is matched to the process that opened the connection.
    private func locate(_ flow: inout Flow) {
        let userAgent = flow.userAgent
        flow.app = UserAgent.appName(userAgent) ?? "Unknown"

        guard Self.isLoopback(flow.client) else {
            flow.device = .remote(ip: flow.client)
            if remoteKinds[flow.client] == nil, let kind = UserAgent.deviceKind(userAgent) {
                remoteKinds[flow.client] = kind
            }
            return
        }

        flow.device = .mac
        // Ports are reused once a connection closes, so an old scan is not
        // trusted for long.
        if Date().timeIntervalSince(liveProcessesScannedAt) < 30,
           let process = liveProcesses[flow.clientPort] {
            attribute(&flow, to: process)
        } else {
            waitingPorts[flow.clientPort, default: []].append(flow.id)
            scanClientProcesses()
        }
    }

    private func attribute(_ flow: inout Flow, to process: ClientProcess) {
        if let udid = process.simulatorUDID {
            flow.device = .simulator(udid: udid)
            flow.app = process.name
            flow.bundlePath = process.bundlePath
            if simulatorNames[udid] == nil { loadSimulatorNames() }
        } else if process.isAndroidEmulator {
            // The emulator carries every app's traffic; keep the User-Agent's name.
            flow.device = .androidEmulator
        } else {
            flow.device = .mac
            flow.app = process.name
            flow.bundlePath = process.bundlePath
        }
    }

    private func scanClientProcesses() {
        guard !scanning else {
            scanAgain = true
            return
        }
        guard let proxyPort = boundAddress.flatMap({ UInt16($0.split(separator: ":").last ?? "") }) else {
            return
        }
        scanning = true
        let waitingAtStart = Set(waitingPorts.keys)
        let scanner = scanner
        scanQueue.async {
            let live = scanner.scan(proxyPort: proxyPort)
            DispatchQueue.main.async {
                MainActor.assumeIsolated { self.finishScan(live, waitingAtStart: waitingAtStart) }
            }
        }
    }

    private func finishScan(_ live: [UInt16: ClientProcess], waitingAtStart: Set<UInt16>) {
        liveProcesses = live
        liveProcessesScannedAt = Date()
        scanning = false

        var moved = false
        for (port, ids) in waitingPorts {
            if let process = live[port] {
                for id in ids {
                    guard let index = indexByID[id] else { continue }
                    tree.remove(flows[index])
                    attribute(&flows[index], to: process)
                    tree.add(flows[index])
                }
                waitingPorts[port] = nil
                moved = true
            } else if waitingAtStart.contains(port) {
                // Closed before the scan saw it: stays under This Mac.
                waitingPorts[port] = nil
            }
        }

        if moved {
            switch sidebarSelection {
            case .device, .app, .appDomain: refilter()
            default: revision += 1
            }
        }
        if scanAgain {
            scanAgain = false
            if !waitingPorts.isEmpty { scanClientProcesses() }
        }
    }

    private func loadSimulatorNames() {
        guard !loadingSimulatorNames else { return }
        loadingSimulatorNames = true
        Task.detached {
            let devices = Simulators.booted()
            await MainActor.run {
                for device in devices { self.simulatorNames[device.id] = device.name }
                self.loadingSimulatorNames = false
            }
        }
    }

    private static func isLoopback(_ address: String) -> Bool {
        address == "::1" || address.hasPrefix("127.") || address == "::ffff:127.0.0.1"
    }

    func name(of device: Device) -> String {
        switch device {
        case .mac: "This Mac"
        case .simulator(let udid): simulatorNames[udid] ?? "Simulator"
        case .androidEmulator: "Android Emulator"
        case .remote(let ip): remoteKinds[ip].map { "\($0) · \(ip)" } ?? ip
        }
    }

    /// Devices in sidebar order.
    var sidebarDevices: [Device] {
        tree.devices.keys.sorted { a, b in
            let (ka, kb) = (a.sortKey, b.sortKey)
            if ka.0 != kb.0 { return ka.0 < kb.0 }
            return name(of: a).localizedStandardCompare(name(of: b)) == .orderedAscending
        }
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

    /// After the root certificate was reset or imported: a running proxy
    /// starts signing with the new one, and no simulator is known to trust it.
    func rootCertificateChanged() {
        simulatorsWithCertificate = []
        if running, let engine {
            engine.stop()
            running = false
            start()
        }
    }

    /// Opens Connect Device on the steps for one kind of device.
    func connectDevice(_ platform: DevicePlatform) {
        connectDevicePlatform = platform
        showConnectDevice = true
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

/// The kinds of device Connect Device has steps for.
enum DevicePlatform: String, CaseIterable, Identifiable {
    case ios = "iPhone or iPad"
    case simulator = "iOS Simulator"
    case android = "Android"
    case emulator = "Android Emulator"
    var id: String { rawValue }
}
