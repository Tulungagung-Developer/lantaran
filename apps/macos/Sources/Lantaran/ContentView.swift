import SwiftUI

struct ContentView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        @Bindable var model = model

        NavigationSplitView {
            SidebarView()
                .navigationSplitViewColumnWidth(min: 180, ideal: 220, max: 320)
        } detail: {
            VStack(spacing: 0) {
                if let error = model.startupError ?? model.lastError {
                    ErrorBanner(message: error)
                }
                VSplitView {
                    TrafficTable()
                        .frame(maxWidth: .infinity, minHeight: 160, maxHeight: .infinity)
                    InspectorView()
                        .frame(maxWidth: .infinity, minHeight: 200, maxHeight: .infinity)
                }
                StatusBar()
            }
        }
        .searchable(text: $model.filterText, placement: .toolbar, prompt: "Filter URL, method or status")
        .toolbar {
            ToolbarItem(placement: .navigation) {
                Button {
                    model.running ? model.stop() : model.start()
                } label: {
                    Label(model.running ? "Stop" : "Start",
                          systemImage: model.running ? "pause.fill" : "play.fill")
                }
                .help(model.running ? "Stop the proxy" : "Start the proxy")
            }
            ToolbarItem(placement: .navigation) {
                Button { model.clear() } label: { Label("Clear", systemImage: "trash") }
                    .help("Clear captured traffic (⌘K)")
            }
            ToolbarItem(placement: .principal) {
                StatusPill()
            }
            ToolbarItem(placement: .primaryAction) {
                Button { model.showSSLProxying = true } label: {
                    Label("SSL Proxying", systemImage: "lock.open")
                }
                .help("Choose which hosts to decrypt")
            }
            ToolbarItem(placement: .primaryAction) {
                Button { model.showConnectDevice = true } label: {
                    Label("Connect Device", systemImage: "iphone.and.arrow.forward")
                }
                .help("Set up an iPhone or Android device")
            }
        }
        .sheet(isPresented: $model.showConnectDevice) { ConnectDeviceSheet() }
        .sheet(isPresented: $model.showSSLProxying) { SSLProxyingSheet() }
    }
}

struct StatusPill: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        HStack(spacing: 6) {
            Circle()
                .fill(model.running ? Color.green : Color.secondary)
                .frame(width: 8, height: 8)
            Text(model.statusText)
                .font(.callout)
                .monospacedDigit()
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 4)
        .background(.quaternary, in: Capsule())
    }
}

struct ErrorBanner: View {
    let message: String

    var body: some View {
        HStack {
            Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(.yellow)
            Text(message).textSelection(.enabled)
            Spacer()
        }
        .padding(8)
        .background(Color.yellow.opacity(0.15))
    }
}

struct StatusBar: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        @Bindable var model = model

        HStack(spacing: 12) {
            Button("Clear") { model.clear() }
            Toggle("Follow newest", isOn: $model.followNewest)
                .toggleStyle(.checkbox)
            Spacer()
            if model.droppedEvents > 0 {
                Text("\(model.droppedEvents) events dropped")
                    .foregroundStyle(.orange)
                    .help("The window could not keep up with the traffic; some rows are missing.")
            }
            Text("\(model.visible.count) of \(model.flows.count) rows")
                .monospacedDigit()
                .foregroundStyle(.secondary)
        }
        .font(.callout)
        .controlSize(.small)
        .padding(.horizontal, 10)
        .padding(.vertical, 5)
        .background(.bar)
    }
}

struct SidebarView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        @Bindable var model = model

        List(selection: $model.sidebarSelection) {
            Label("All Traffic", systemImage: "tray.full")
                .tag(SidebarItem.all)
            Section("Devices") {
                ForEach(model.clients, id: \.0) { client, count in
                    SidebarRow(title: deviceName(client), systemImage: deviceIcon(client), count: count)
                        .tag(SidebarItem.client(client))
                }
            }
            Section("Domains") {
                ForEach(model.hosts, id: \.0) { host, count in
                    SidebarRow(
                        title: host,
                        systemImage: model.isDecrypted(host) ? "lock.open" : "globe",
                        count: count
                    )
                    .tag(SidebarItem.host(host))
                }
            }
        }
        .listStyle(.sidebar)
    }

    private func deviceName(_ client: String) -> String {
        client == "127.0.0.1" || client == "::1" ? "This Mac, simulators and emulators" : client
    }

    private func deviceIcon(_ client: String) -> String {
        client == "127.0.0.1" || client == "::1" ? "laptopcomputer" : "iphone"
    }
}

struct SidebarRow: View {
    let title: String
    let systemImage: String
    let count: Int

    var body: some View {
        HStack {
            Label(title, systemImage: systemImage).lineLimit(1)
            Spacer()
            Text("\(count)")
                .font(.caption)
                .monospacedDigit()
                .foregroundStyle(.secondary)
        }
    }
}
