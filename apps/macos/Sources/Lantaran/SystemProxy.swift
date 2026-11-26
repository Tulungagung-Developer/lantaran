import Foundation

/// Points the Mac's HTTP and HTTPS proxy at Lantaran, and puts it back.
///
/// The iOS Simulator has no proxy settings of its own: it uses the Mac's.
/// Before changing anything, the current settings of every network service
/// are written to a backup file; restoring reads that file back. If
/// Lantaran quits unexpectedly, the next launch finds the file and restores
/// it, so the Mac is never left pointing at a proxy that is not running.
enum SystemProxy {
    struct Setting: Codable, Equatable {
        var enabled: Bool
        var server: String
        var port: Int
    }

    struct ServiceBackup: Codable, Equatable {
        var service: String
        var web: Setting
        var secure: Setting
    }

    enum Failure: LocalizedError {
        case command(String)

        var errorDescription: String? {
            switch self {
            case .command(let message): message
            }
        }
    }

    private static let networksetup = "/usr/sbin/networksetup"

    static var backupFile: URL {
        AppModel.dataDirectory.appendingPathComponent("system-proxy-backup.json")
    }

    /// Whether Lantaran has changed the Mac's proxy and not yet put it back.
    static var isRedirected: Bool {
        FileManager.default.fileExists(atPath: backupFile.path)
    }

    /// Enabled network services, such as "Wi-Fi".
    static func services() throws -> [String] {
        let output = try run(networksetup, ["-listallnetworkservices"])
        return output
            .split(separator: "\n")
            .dropFirst() // "An asterisk (*) denotes that a network service is disabled."
            .map(String.init)
            .filter { !$0.hasPrefix("*") && !$0.isEmpty }
    }

    /// Sends HTTP and HTTPS from every network service to `host:port`.
    static func redirect(to host: String, port: Int) throws {
        let services = try services()
        // Keep the first backup: if Lantaran redirects again (say, after a
        // port change) the original settings are still the ones to restore.
        if !isRedirected {
            let backup = try services.map { service in
                ServiceBackup(
                    service: service,
                    web: try read(service, secure: false),
                    secure: try read(service, secure: true)
                )
            }
            try FileManager.default.createDirectory(
                at: backupFile.deletingLastPathComponent(), withIntermediateDirectories: true)
            try JSONEncoder().encode(backup).write(to: backupFile, options: .atomic)
        }

        let commands = services.flatMap { service in
            [
                ["-setwebproxy", service, host, String(port)],
                ["-setsecurewebproxy", service, host, String(port)],
            ]
        }
        try runPrivileged(commands)
    }

    /// Puts back the settings saved by `redirect`. Does nothing if there is
    /// no backup.
    static func restore() throws {
        guard let data = try? Data(contentsOf: backupFile) else { return }
        let backup = try JSONDecoder().decode([ServiceBackup].self, from: data)
        let current = Set((try? services()) ?? [])

        let commands = backup
            .filter { current.isEmpty || current.contains($0.service) }
            .flatMap { entry in
                commandsToRestore(entry.service, entry.web, setter: "-setwebproxy", state: "-setwebproxystate")
                    + commandsToRestore(entry.service, entry.secure, setter: "-setsecurewebproxy", state: "-setsecurewebproxystate")
            }
        try runPrivileged(commands)
        try FileManager.default.removeItem(at: backupFile)
    }

    static func commandsToRestore(_ service: String, _ setting: Setting, setter: String, state: String) -> [[String]] {
        var commands: [[String]] = []
        // Setting a server also switches the proxy on, so the server comes
        // first and the state last.
        if !setting.server.isEmpty {
            commands.append([setter, service, setting.server, String(setting.port)])
        }
        commands.append([state, service, setting.enabled ? "on" : "off"])
        return commands
    }

    static func read(_ service: String, secure: Bool) throws -> Setting {
        let output = try run(networksetup, [secure ? "-getsecurewebproxy" : "-getwebproxy", service])
        return parse(output)
    }

    /// Parses `networksetup -getwebproxy` output.
    static func parse(_ output: String) -> Setting {
        var values: [String: String] = [:]
        for line in output.split(separator: "\n") {
            let parts = line.split(separator: ":", maxSplits: 1).map { $0.trimmingCharacters(in: .whitespaces) }
            if parts.count == 2 { values[parts[0]] = parts[1] }
        }
        return Setting(
            enabled: values["Enabled"] == "Yes",
            server: values["Server"] ?? "",
            port: Int(values["Port"] ?? "") ?? 0
        )
    }

    /// Runs networksetup commands. When the user is not allowed to change
    /// network settings directly, runs them all at once with administrator
    /// rights, which shows one macOS password prompt.
    private static func runPrivileged(_ commands: [[String]]) throws {
        guard !commands.isEmpty else { return }
        do {
            for command in commands {
                try run(networksetup, command)
            }
        } catch {
            let script = commands
                .map { ([networksetup] + $0).map(shellQuote).joined(separator: " ") }
                .joined(separator: " && ")
            let appleScript = "do shell script \(appleScriptQuote(script)) with administrator privileges"
            try run("/usr/bin/osascript", ["-e", appleScript])
        }
    }

    @discardableResult
    static func run(_ executable: String, _ arguments: [String]) throws -> String {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: executable)
        process.arguments = arguments
        let output = Pipe()
        process.standardOutput = output
        process.standardError = output
        try process.run()
        let data = output.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        let text = String(data: data, encoding: .utf8) ?? ""
        // networksetup reports some failures on stdout with status 0.
        if process.terminationStatus != 0
            || text.localizedCaseInsensitiveContains("requires admin")
            || text.localizedCaseInsensitiveContains("not authorized")
        {
            throw Failure.command(text.trimmingCharacters(in: .whitespacesAndNewlines))
        }
        return text
    }

    private static func shellQuote(_ text: String) -> String {
        "'" + text.replacingOccurrences(of: "'", with: "'\\''") + "'"
    }

    private static func appleScriptQuote(_ text: String) -> String {
        "\"" + text.replacingOccurrences(of: "\\", with: "\\\\").replacingOccurrences(of: "\"", with: "\\\"") + "\""
    }
}
