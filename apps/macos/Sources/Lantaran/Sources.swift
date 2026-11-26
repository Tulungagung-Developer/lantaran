import Foundation

/// Where a flow came from, as the sidebar groups it.
enum Device: Hashable {
    case mac
    case simulator(udid: String)
    case androidEmulator
    case remote(ip: String)

    /// Sidebar order: this Mac, its simulators and emulators, then other
    /// devices.
    var sortKey: (Int, String) {
        switch self {
        case .mac: (0, "")
        case .simulator(let udid): (1, udid)
        case .androidEmulator: (2, "")
        case .remote(let ip): (3, ip)
        }
    }
}

enum UserAgent {
    /// The app that sent a request, from its `User-Agent`, or `nil` when
    /// the header says nothing useful.
    ///
    /// iOS apps using the system stack send `AppName/1.0 CFNetwork/… Darwin/…`
    /// with the name percent-encoded; browsers send `Mozilla/5.0 (…)`.
    static func appName(_ userAgent: String?) -> String? {
        guard let userAgent = userAgent?.trimmingCharacters(in: .whitespaces), !userAgent.isEmpty else {
            return nil
        }
        if userAgent.hasPrefix("Mozilla/") {
            if userAgent.contains("CriOS/") || userAgent.contains("Chrome/") && !userAgent.contains("Edg") {
                return "Chrome"
            }
            if userAgent.contains("FxiOS/") || userAgent.contains("Firefox/") { return "Firefox" }
            if userAgent.contains("EdgiOS/") || userAgent.contains("Edg/") { return "Edge" }
            if userAgent.contains("Safari/") { return "Safari" }
            return "Web view"
        }
        if userAgent.hasPrefix("Dalvik/") { return "Android system" }

        let product = userAgent.prefix { $0 != " " && $0 != "/" && $0 != "(" }
        let name = String(product).removingPercentEncoding ?? String(product)
        return name.isEmpty ? nil : name
    }

    /// What kind of device sent it, when the header says.
    static func deviceKind(_ userAgent: String?) -> String? {
        guard let userAgent else { return nil }
        if userAgent.contains("iPad") { return "iPad" }
        if userAgent.contains("iPhone") { return "iPhone" }
        if userAgent.contains("Android") { return "Android" }
        return nil
    }
}

/// Flow counts for the sidebar: per device, per app on it, per domain of
/// that app, and per domain overall. Kept up to date flow by flow.
struct SidebarTree {
    struct AppNode {
        var count = 0
        var bundlePath: String?
        var domains: [String: Int] = [:]
    }

    struct DeviceNode {
        var count = 0
        var apps: [String: AppNode] = [:]
    }

    private(set) var devices: [Device: DeviceNode] = [:]
    private(set) var domains: [String: Int] = [:]

    mutating func add(_ flow: Flow) {
        devices[flow.device, default: DeviceNode()].count += 1
        devices[flow.device]!.apps[flow.app, default: AppNode()].count += 1
        if let path = flow.bundlePath {
            devices[flow.device]!.apps[flow.app]!.bundlePath = path
        }
        devices[flow.device]!.apps[flow.app]!.domains[flow.host, default: 0] += 1
        domains[flow.host, default: 0] += 1
    }

    mutating func remove(_ flow: Flow) {
        guard var device = devices[flow.device], var app = device.apps[flow.app] else { return }
        app.count -= 1
        app.domains[flow.host, default: 1] -= 1
        if app.domains[flow.host] == 0 { app.domains[flow.host] = nil }
        device.apps[flow.app] = app.count > 0 ? app : nil
        device.count -= 1
        devices[flow.device] = device.count > 0 ? device : nil

        domains[flow.host, default: 1] -= 1
        if domains[flow.host] == 0 { domains[flow.host] = nil }
    }

    mutating func removeAll() {
        devices.removeAll()
        domains.removeAll()
    }
}
