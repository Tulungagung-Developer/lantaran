import Darwin
import Foundation

/// The process on this Mac at the other end of a proxy connection.
struct ClientProcess: Sendable, Equatable {
    let pid: pid_t
    /// What the user knows the process as: the app's name, or the
    /// executable's name for tools and daemons.
    let name: String
    /// The `.app` bundle the process runs from, for its icon.
    let bundlePath: String?
    /// Set for processes running inside an iOS Simulator.
    let simulatorUDID: String?
    /// The Android Emulator's own process, which carries the emulated
    /// device's traffic.
    let isAndroidEmulator: Bool
}

/// Finds which local process opened a connection to the proxy, from the
/// connection's source port. Uses libproc, which reads the sockets of the
/// user's own processes without extra permissions.
///
/// Not thread safe: use it from one queue (hence `@unchecked Sendable`).
final class ClientProcessScanner: @unchecked Sendable {
    private var described: [pid_t: (path: String, process: ClientProcess)] = [:]

    /// Every open TCP connection on this Mac to `proxyPort`, by source port.
    /// The proxy's own process is skipped: its accepted sockets have the
    /// proxy port as their local port, not the remote one, but a client
    /// inside the same process would be misread.
    func scan(proxyPort: UInt16, excluding excluded: pid_t = getpid()) -> [UInt16: ClientProcess] {
        var owners: [UInt16: pid_t] = [:]
        for pid in Self.allPIDs() where pid > 0 && pid != excluded {
            for port in Self.sourcePorts(of: pid, connectedTo: proxyPort) {
                owners[port] = pid
            }
        }
        return owners.compactMapValues { describe($0) }
    }

    private func describe(_ pid: pid_t) -> ClientProcess? {
        guard let path = Self.executablePath(pid) else { return nil }
        // A pid can be reused by another program; the path tells them apart.
        if let known = described[pid], known.path == path { return known.process }

        let process = ClientProcess(
            pid: pid,
            name: Self.displayName(path: path, pid: pid),
            bundlePath: Self.bundlePath(of: path),
            simulatorUDID: Self.environment(of: pid)["SIMULATOR_UDID"],
            isAndroidEmulator: (path as NSString).lastPathComponent.hasPrefix("qemu-system")
        )
        described[pid] = (path, process)
        return process
    }

    // MARK: libproc

    private static func allPIDs() -> [pid_t] {
        let count = proc_listallpids(nil, 0)
        guard count > 0 else { return [] }
        var pids = [pid_t](repeating: 0, count: Int(count) + 64)
        let got = proc_listallpids(&pids, Int32(pids.count * MemoryLayout<pid_t>.size))
        return Array(pids.prefix(max(0, Int(got))))
    }

    private static func sourcePorts(of pid: pid_t, connectedTo proxyPort: UInt16) -> [UInt16] {
        let size = proc_pidinfo(pid, PROC_PIDLISTFDS, 0, nil, 0)
        guard size > 0 else { return [] }
        let stride = MemoryLayout<proc_fdinfo>.stride
        var fds = [proc_fdinfo](repeating: proc_fdinfo(), count: Int(size) / stride)
        let got = proc_pidinfo(pid, PROC_PIDLISTFDS, 0, &fds, size)
        guard got > 0 else { return [] }

        var ports: [UInt16] = []
        for fd in fds.prefix(Int(got) / stride) where fd.proc_fdtype == UInt32(PROX_FDTYPE_SOCKET) {
            var info = socket_fdinfo()
            let infoSize = Int32(MemoryLayout<socket_fdinfo>.size)
            guard proc_pidfdinfo(pid, fd.proc_fd, PROC_PIDFDSOCKETINFO, &info, infoSize) == infoSize,
                  info.psi.soi_kind == SOCKINFO_TCP
            else { continue }
            let addresses = info.psi.soi_proto.pri_tcp.tcpsi_ini
            // Ports are stored in network byte order.
            let local = UInt16(bigEndian: UInt16(truncatingIfNeeded: addresses.insi_lport))
            let remote = UInt16(bigEndian: UInt16(truncatingIfNeeded: addresses.insi_fport))
            if remote == proxyPort { ports.append(local) }
        }
        return ports
    }

    private static func executablePath(_ pid: pid_t) -> String? {
        var buffer = [CChar](repeating: 0, count: Int(MAXPATHLEN) * 4)
        guard proc_pidpath(pid, &buffer, UInt32(buffer.count)) > 0 else { return nil }
        return String(cString: buffer)
    }

    /// The process's environment, read with `KERN_PROCARGS2`: argc, the
    /// executable path, then NUL-separated arguments and environment.
    private static func environment(of pid: pid_t) -> [String: String] {
        var mib: [Int32] = [CTL_KERN, KERN_PROCARGS2, pid]
        var size = 0
        guard sysctl(&mib, 3, nil, &size, nil, 0) == 0, size > 4 else { return [:] }
        var buffer = [UInt8](repeating: 0, count: size)
        guard sysctl(&mib, 3, &buffer, &size, nil, 0) == 0, size > 4 else { return [:] }

        let argc = buffer.withUnsafeBytes { Int($0.load(as: Int32.self)) }
        let strings = buffer[4..<size]
            .split(separator: 0, omittingEmptySubsequences: true)
            .map { String(decoding: $0, as: UTF8.self) }
        // The executable path comes first, then argc arguments.
        var environment: [String: String] = [:]
        for entry in strings.dropFirst(1 + argc) {
            guard let equals = entry.firstIndex(of: "=") else { continue }
            environment[String(entry[..<equals])] = String(entry[entry.index(after: equals)...])
        }
        return environment
    }

    // MARK: Names

    /// The innermost `.app` the executable lives in.
    static func bundlePath(of executable: String) -> String? {
        let components = (executable as NSString).pathComponents
        guard let index = components.lastIndex(where: { $0.hasSuffix(".app") }) else { return nil }
        return NSString.path(withComponents: Array(components[...index]))
    }

    static func displayName(path: String, pid: pid_t) -> String {
        let executable = (path as NSString).lastPathComponent
        if let pretty = knownNames[executable] { return pretty }
        if let bundle = bundlePath(of: path).flatMap(Bundle.init(path:)),
           let name = bundle.object(forInfoDictionaryKey: "CFBundleDisplayName") as? String
            ?? bundle.object(forInfoDictionaryKey: "CFBundleName") as? String,
           !name.isEmpty {
            return name
        }
        return executable
    }

    /// Processes that carry traffic for something better known by another
    /// name.
    private static let knownNames = [
        "com.apple.WebKit.Networking": "Safari and web views",
    ]
}
