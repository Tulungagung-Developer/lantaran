import Foundation

/// Booted iOS simulators, and installing the root certificate in them.
enum Simulators {
    struct Device: Identifiable, Hashable {
        let id: String
        let name: String
        let runtime: String
    }

    private struct List: Decodable {
        struct Entry: Decodable {
            let udid: String
            let name: String
            let state: String
        }

        let devices: [String: [Entry]]
    }

    /// Simulators that are running now. Empty when Xcode is not installed.
    static func booted() -> [Device] {
        guard let output = try? SystemProxy.run("/usr/bin/xcrun", ["simctl", "list", "devices", "booted", "--json"]),
              let data = output.data(using: .utf8),
              let list = try? JSONDecoder().decode(List.self, from: data)
        else { return [] }

        return list.devices
            .flatMap { runtime, entries in
                entries.filter { $0.state == "Booted" }.map {
                    Device(id: $0.udid, name: $0.name, runtime: runtimeName(runtime))
                }
            }
            .sorted { $0.name < $1.name }
    }

    /// Adds `certificate` (a PEM file) to the simulator's trusted roots. Safe
    /// to repeat.
    static func installCertificate(_ certificate: String, in device: Device) throws {
        try SystemProxy.run("/usr/bin/xcrun", ["simctl", "keychain", device.id, "add-root-cert", certificate])
    }

    /// "com.apple.CoreSimulator.SimRuntime.iOS-27-0" becomes "iOS 27.0".
    static func runtimeName(_ identifier: String) -> String {
        guard let last = identifier.split(separator: ".").last else { return identifier }
        let parts = last.split(separator: "-")
        guard let platform = parts.first else { return String(last) }
        return "\(platform) " + parts.dropFirst().joined(separator: ".")
    }
}
