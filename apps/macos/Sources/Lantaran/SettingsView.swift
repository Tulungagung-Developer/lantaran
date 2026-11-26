import SwiftUI

struct SettingsView: View {
    @Environment(AppModel.self) private var model
    @State private var port = ""

    var body: some View {
        @Bindable var model = model

        Form {
            Section {
                HStack {
                    TextField("Port", text: $port)
                        .frame(width: 80)
                        .onSubmit(applyPort)
                    Button("Apply", action: applyPort)
                        .disabled(Int(port) == model.port || !(1...65535).contains(Int(port) ?? 0))
                }
                Toggle("Accept connections from other devices on this network", isOn: Binding(
                    get: { model.acceptOtherDevices },
                    set: { model.acceptOtherDevices = $0; model.restart() }
                ))
                Text("Phones need this. The iOS Simulator and the Android Emulator connect from this Mac and work either way.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            } header: {
                Text("Proxy")
            }
            Section {
                LabeledContent("Root certificate") {
                    Button("Show in Finder") { model.revealCertificate() }
                }
                Text("Shared with the lantaran command line tool, so devices set up with either work with both.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            } header: {
                Text("Certificate")
            }
        }
        .formStyle(.grouped)
        .frame(width: 460)
        .onAppear { port = String(model.port) }
    }

    private func applyPort() {
        guard let value = Int(port), (1...65535).contains(value), value != model.port else { return }
        model.port = value
        model.restart()
    }
}
