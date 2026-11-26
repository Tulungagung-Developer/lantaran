import SwiftUI

struct SSLProxyingSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("SSL Proxying").font(.title2.weight(.semibold))

            HostListEditor(
                title: "Decrypt these hosts",
                explanation: "HTTPS to every other host passes through unread. Use * as a wildcard, for example *.example.com. Apps that pin their certificates stop working for hosts on this list.",
                hosts: model.sslHosts,
                save: model.setSSLHosts
            )

            HostListEditor(
                title: "Do not check these servers' certificates",
                explanation: "For development servers with self-signed certificates. Certificate authorities installed on this Mac, such as mkcert's, are trusted already.",
                hosts: model.insecureHosts,
                save: model.setInsecureHosts
            )

            HStack {
                Spacer()
                Button("Done") { dismiss() }.keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .frame(width: 520)
    }
}

private struct HostListEditor: View {
    let title: String
    let explanation: String
    let hosts: [String]
    let save: ([String]) -> Void
    @State private var newHost = ""
    @State private var selection: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(title).font(.headline)
            Text(explanation).font(.caption).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            List(hosts, id: \.self, selection: $selection) { host in
                Text(host).font(.body.monospaced())
            }
            .frame(height: 120)
            .onDeleteCommand { remove() }
            HStack {
                TextField("api.example.com or *.example.com", text: $newHost)
                    .textFieldStyle(.roundedBorder)
                    .onSubmit(add)
                Button("Add", action: add).disabled(newHost.trimmingCharacters(in: .whitespaces).isEmpty)
                Button("Remove", action: remove).disabled(selection == nil)
            }
        }
    }

    private func add() {
        let host = newHost.trimmingCharacters(in: .whitespaces)
        guard !host.isEmpty else { return }
        save(hosts + [host])
        newHost = ""
    }

    private func remove() {
        guard let selection else { return }
        save(hosts.filter { $0 != selection })
        self.selection = nil
    }
}
