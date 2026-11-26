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
                    VStack(spacing: 0) {
                        FilterBar()
                        Divider()
                        TrafficTable()
                    }
                    .frame(maxWidth: .infinity, minHeight: 160, maxHeight: .infinity)
                    InspectorView()
                        .frame(maxWidth: .infinity, minHeight: 200, maxHeight: .infinity)
                }
                StatusBar()
            }
        }
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
        .sheet(isPresented: $model.showMapLocal) {
            MapLocalSheet()
                .sheet(item: $model.editingLocalRule) { LocalRuleEditor(rule: $0) }
                .sheet(item: $model.editingRemoteRule) { RemoteRuleEditor(rule: $0) }
        }
        .sheet(isPresented: $model.showMapRemote) {
            MapRemoteSheet()
                .sheet(item: $model.editingLocalRule) { LocalRuleEditor(rule: $0) }
                .sheet(item: $model.editingRemoteRule) { RemoteRuleEditor(rule: $0) }
        }
        .sheet(item: Binding(
            get: { model.showMapLocal || model.showMapRemote ? nil : model.editingLocalRule },
            set: { model.editingLocalRule = $0 }
        )) { LocalRuleEditor(rule: $0) }
        .sheet(item: Binding(
            get: { model.showMapLocal || model.showMapRemote ? nil : model.editingRemoteRule },
            set: { model.editingRemoteRule = $0 }
        )) { RemoteRuleEditor(rule: $0) }
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
