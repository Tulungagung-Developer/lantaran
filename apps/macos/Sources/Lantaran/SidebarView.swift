import AppKit
import SwiftUI

/// Devices, the apps on each and the domains each app talks to, then all
/// domains. Selecting a row narrows the traffic table; the field at the top
/// narrows the sidebar itself.
struct SidebarView: View {
    @Environment(AppModel.self) private var model
    @State private var search = ""
    @State private var collapsedDevices: Set<Device> = []
    @State private var expandedApps: Set<AppKey> = []
    @State private var domainsExpanded = true

    struct AppKey: Hashable {
        let device: Device
        let app: String
    }

    var body: some View {
        @Bindable var model = model
        let query = search.trimmingCharacters(in: .whitespaces)

        List(selection: $model.sidebarSelection) {
            SidebarRow(title: "All", icon: .symbol("tray.full"), count: model.flows.count)
                .tag(SidebarItem.all)

            Section("Devices") {
                ForEach(model.sidebarDevices, id: \.self) { device in
                    deviceRow(device, query: query)
                }
            }

            let domains = matchingDomains(query)
            if !domains.isEmpty {
                Section(isExpanded: $domainsExpanded) {
                    ForEach(domains, id: \.0) { host, count in
                        SidebarRow(title: host, icon: .symbol(domainSymbol(host)), count: count)
                            .tag(SidebarItem.domain(host))
                    }
                } header: {
                    Text("Domains")
                }
            }
        }
        .listStyle(.sidebar)
        .safeAreaInset(edge: .top, spacing: 0) {
            SidebarSearchField(text: $search)
                .padding(.horizontal, 10)
                .padding(.vertical, 6)
        }
    }

    // MARK: Rows

    @ViewBuilder
    private func deviceRow(_ device: Device, query: String) -> some View {
        if let node = model.tree.devices[device] {
            let deviceMatches = matches(model.name(of: device), query)
            let apps = node.apps
                .filter { name, app in
                    deviceMatches || matches(name, query) || app.domains.keys.contains { matches($0, query) }
                }
                .sorted { $0.key.localizedStandardCompare($1.key) == .orderedAscending }

            if deviceMatches || !apps.isEmpty {
                DisclosureGroup(isExpanded: deviceExpanded(device, searching: !query.isEmpty)) {
                    ForEach(apps, id: \.key) { name, app in
                        appRow(device: device, name: name, app: app, query: query, parentMatches: deviceMatches)
                    }
                } label: {
                    SidebarRow(
                        title: model.name(of: device),
                        subtitle: subtitle(of: device),
                        icon: .symbol(symbol(of: device)),
                        count: node.count
                    )
                }
                .tag(SidebarItem.device(device))
            }
        }
    }

    @ViewBuilder
    private func appRow(
        device: Device, name: String, app: SidebarTree.AppNode, query: String, parentMatches: Bool
    ) -> some View {
        let key = AppKey(device: device, app: name)
        let appMatches = parentMatches || matches(name, query)
        let domains = app.domains
            .filter { appMatches || matches($0.key, query) }
            .sorted { $0.key.localizedStandardCompare($1.key) == .orderedAscending }

        DisclosureGroup(isExpanded: appExpanded(key, searching: !query.isEmpty && !appMatches)) {
            ForEach(domains, id: \.key) { host, count in
                SidebarRow(title: host, icon: .symbol(domainSymbol(host)), count: count)
                    .tag(SidebarItem.appDomain(device, name, host))
            }
        } label: {
            SidebarRow(title: name, icon: appIcon(name: name, path: app.bundlePath, device: device), count: app.count)
        }
        .tag(SidebarItem.app(device, name))
    }

    // MARK: Expansion

    /// Devices start open; apps start closed. A search opens what it found.
    private func deviceExpanded(_ device: Device, searching: Bool) -> Binding<Bool> {
        Binding(
            get: { searching || !collapsedDevices.contains(device) },
            set: { open in
                if open { collapsedDevices.remove(device) } else { collapsedDevices.insert(device) }
            }
        )
    }

    private func appExpanded(_ key: AppKey, searching: Bool) -> Binding<Bool> {
        Binding(
            get: { searching || expandedApps.contains(key) },
            set: { open in
                if open { expandedApps.insert(key) } else { expandedApps.remove(key) }
            }
        )
    }

    // MARK: Names and icons

    private func matches(_ text: String, _ query: String) -> Bool {
        query.isEmpty || text.localizedCaseInsensitiveContains(query)
    }

    private func matchingDomains(_ query: String) -> [(String, Int)] {
        model.tree.domains
            .filter { matches($0.key, query) }
            .sorted { $0.key.localizedStandardCompare($1.key) == .orderedAscending }
    }

    private func subtitle(of device: Device) -> String? {
        switch device {
        case .simulator: "Simulator"
        default: nil
        }
    }

    private func symbol(of device: Device) -> String {
        switch device {
        case .mac: "laptopcomputer"
        case .simulator(let udid):
            model.simulatorNames[udid]?.contains("iPad") == true ? "ipad" : "iphone"
        case .androidEmulator: "candybarphone"
        case .remote(let ip):
            switch model.remoteKinds[ip] {
            case "iPad": "ipad"
            case "iPhone": "iphone"
            default: "candybarphone"
            }
        }
    }

    private func domainSymbol(_ host: String) -> String {
        model.isDecrypted(host) ? "lock.open" : "globe"
    }

    private func appIcon(name: String, path: String?, device: Device) -> SidebarRow.Icon {
        if let path { return .file(path) }
        switch device {
        case .mac: return .symbol("terminal")
        default: return .symbol("app.dashed")
        }
    }
}

struct SidebarRow: View {
    enum Icon {
        case symbol(String)
        /// The Finder icon of an app bundle.
        case file(String)
    }

    let title: String
    var subtitle: String?
    let icon: Icon
    let count: Int

    var body: some View {
        HStack(spacing: 6) {
            iconView.frame(width: 16, height: 16)
            Text(title).lineLimit(1).truncationMode(.middle)
            if let subtitle {
                Text(subtitle).font(.caption).foregroundStyle(.secondary).lineLimit(1)
            }
            Spacer(minLength: 4)
            Text("\(count)")
                .font(.caption)
                .monospacedDigit()
                .foregroundStyle(.secondary)
        }
        .help(title)
    }

    @ViewBuilder
    private var iconView: some View {
        switch icon {
        case .symbol(let name):
            Image(systemName: name).foregroundStyle(.secondary)
        case .file(let path):
            Image(nsImage: AppIcons.icon(forFile: path))
                .resizable()
                .interpolation(.high)
        }
    }
}

/// App icons are read from disk once per bundle.
@MainActor
enum AppIcons {
    private static var cache: [String: NSImage] = [:]

    static func icon(forFile path: String) -> NSImage {
        if let icon = cache[path] { return icon }
        let icon = NSWorkspace.shared.icon(forFile: path)
        cache[path] = icon
        return icon
    }
}

struct SidebarSearchField: View {
    @Binding var text: String

    var body: some View {
        HStack(spacing: 4) {
            Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
            TextField("Filter devices, apps, domains", text: $text)
                .textFieldStyle(.plain)
            if !text.isEmpty {
                Button {
                    text = ""
                } label: {
                    Image(systemName: "xmark.circle.fill").foregroundStyle(.secondary)
                }
                .buttonStyle(.plain)
            }
        }
        .padding(.horizontal, 6)
        .padding(.vertical, 4)
        .background(.quaternary.opacity(0.5), in: RoundedRectangle(cornerRadius: 6))
    }
}
