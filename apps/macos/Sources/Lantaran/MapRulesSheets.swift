import AppKit
import SwiftUI

struct MapLocalSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @State private var selection: LocalRule.ID?

    var body: some View {
        RuleListSheet(
            title: "Map Local",
            explanation: "Answer matching requests from a file on this Mac instead of the server. The file is read on every request, so edits apply straight away. Only requests Lantaran can read are mapped: plain HTTP, or HTTPS for hosts on the SSL Proxying List.",
            isEmpty: model.localRules.isEmpty,
            selection: selection != nil,
            add: { model.editingLocalRule = LocalRule() },
            edit: { if let rule = selected { model.editingLocalRule = rule } },
            remove: { model.setLocalRules(model.localRules.filter { $0.id != selection }); selection = nil },
            dismiss: { dismiss() }
        ) {
            Table(model.localRules, selection: $selection) {
                TableColumn("On") { rule in
                    Toggle("", isOn: Binding(
                        get: { rule.enabled },
                        set: { var changed = rule; changed.enabled = $0; model.saveLocalRule(changed) }
                    ))
                    .labelsHidden()
                }
                .width(30)
                TableColumn("Method") { Text($0.method.isEmpty ? "Any" : $0.method) }.width(56)
                TableColumn("Matching URL") { Text($0.pattern).lineLimit(1).truncationMode(.middle) }
                TableColumn("File") { rule in
                    Text((rule.file as NSString).lastPathComponent)
                        .foregroundStyle(FileManager.default.fileExists(atPath: (rule.file as NSString).expandingTildeInPath) ? .primary : Color.red)
                        .help(rule.file)
                }
            }
            .contextMenu(forSelectionType: LocalRule.ID.self) { _ in } primaryAction: { ids in
                if let id = ids.first, let rule = model.localRules.first(where: { $0.id == id }) {
                    model.editingLocalRule = rule
                }
            }
        }
    }

    private var selected: LocalRule? {
        model.localRules.first { $0.id == selection }
    }
}

struct MapRemoteSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @State private var selection: RemoteRule.ID?

    var body: some View {
        RuleListSheet(
            title: "Map Remote",
            explanation: "Send matching requests to a different server, for example from production to staging or to a server running on this Mac. The query string is always kept. Only requests Lantaran can read are mapped.",
            isEmpty: model.remoteRules.isEmpty,
            selection: selection != nil,
            add: { model.editingRemoteRule = RemoteRule() },
            edit: { if let rule = selected { model.editingRemoteRule = rule } },
            remove: { model.setRemoteRules(model.remoteRules.filter { $0.id != selection }); selection = nil },
            dismiss: { dismiss() }
        ) {
            Table(model.remoteRules, selection: $selection) {
                TableColumn("On") { rule in
                    Toggle("", isOn: Binding(
                        get: { rule.enabled },
                        set: { var changed = rule; changed.enabled = $0; model.saveRemoteRule(changed) }
                    ))
                    .labelsHidden()
                }
                .width(30)
                TableColumn("Method") { Text($0.method.isEmpty ? "Any" : $0.method) }.width(56)
                TableColumn("Matching URL") { Text($0.pattern).lineLimit(1).truncationMode(.middle) }
                TableColumn("Send To") { rule in
                    Text(rule.target).foregroundStyle(rule.isValid ? .primary : Color.red)
                }
            }
            .contextMenu(forSelectionType: RemoteRule.ID.self) { _ in } primaryAction: { ids in
                if let id = ids.first, let rule = model.remoteRules.first(where: { $0.id == id }) {
                    model.editingRemoteRule = rule
                }
            }
        }
    }

    private var selected: RemoteRule? {
        model.remoteRules.first { $0.id == selection }
    }
}

/// The frame both rule lists share.
private struct RuleListSheet<Content: View>: View {
    let title: String
    let explanation: String
    let isEmpty: Bool
    let selection: Bool
    let add: () -> Void
    let edit: () -> Void
    let remove: () -> Void
    let dismiss: () -> Void
    @ViewBuilder let content: () -> Content

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(title).font(.title2.weight(.semibold))
            Text(explanation).font(.callout).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            content()
                .frame(height: 220)
                .overlay {
                    if isEmpty {
                        Text("No rules yet. Add one, or right-click a request in the traffic list.")
                            .foregroundStyle(.secondary)
                    }
                }
            HStack {
                Button("Add…", action: add)
                Button("Edit…", action: edit).disabled(!selection)
                Button("Remove", action: remove).disabled(!selection)
                Spacer()
                Button("Done", action: dismiss).keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .frame(width: 680)
    }
}

private let methods = ["", "GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"]

struct LocalRuleEditor: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @State var rule: LocalRule
    @State private var status = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Map Local Rule").font(.title3.weight(.semibold))
            Form {
                TextField("Matching URL", text: $rule.pattern, prompt: Text("https://api.example.com/v1/users*"))
                Picker("Method", selection: $rule.method) {
                    ForEach(methods, id: \.self) { Text($0.isEmpty ? "Any" : $0).tag($0) }
                }
                HStack {
                    TextField("File", text: $rule.file, prompt: Text("Choose a file"))
                    Button("Choose…", action: chooseFile)
                    Button("Edit") { NSWorkspace.shared.open(URL(fileURLWithPath: expandedFile)) }
                        .disabled(!FileManager.default.fileExists(atPath: expandedFile))
                }
                TextField("Status code", text: $status)
                TextField("Content type", text: $rule.contentType, prompt: Text("From the file extension"))
            }
            Text("Use * as a wildcard. Without a path, the rule covers the whole host.")
                .font(.caption).foregroundStyle(.secondary)
            HStack {
                Spacer()
                Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("Save") {
                    rule.status = Int(status) ?? 200
                    model.saveLocalRule(rule)
                    dismiss()
                }
                .keyboardShortcut(.defaultAction)
                .disabled(rule.pattern.trimmingCharacters(in: .whitespaces).isEmpty || rule.file.isEmpty)
            }
        }
        .padding(20)
        .frame(width: 560)
        .onAppear { status = String(rule.status) }
    }

    private var expandedFile: String { (rule.file as NSString).expandingTildeInPath }

    private func chooseFile() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        if panel.runModal() == .OK, let url = panel.url {
            rule.file = url.path
        }
    }
}

struct RemoteRuleEditor: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @State var rule: RemoteRule

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Map Remote Rule").font(.title3.weight(.semibold))
            Form {
                TextField("Matching URL", text: $rule.pattern, prompt: Text("https://api.example.com/*"))
                Picker("Method", selection: $rule.method) {
                    ForEach(methods, id: \.self) { Text($0.isEmpty ? "Any" : $0).tag($0) }
                }
                TextField("Send to", text: $rule.target, prompt: Text("http://localhost:3000"))
            }
            Text("The scheme, host and port of \"Send to\" replace the request's. A path in it replaces the request's path; without one the path is kept. The query string is always kept.")
                .font(.caption).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            if !rule.target.isEmpty, rule.engineRule == nil {
                Label("\"Send to\" needs a full URL with a scheme and host, like http://localhost:3000.",
                      systemImage: "exclamationmark.triangle")
                    .foregroundStyle(.orange)
            }
            HStack {
                Spacer()
                Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("Save") {
                    model.saveRemoteRule(rule)
                    dismiss()
                }
                .keyboardShortcut(.defaultAction)
                .disabled(!rule.isValid)
            }
        }
        .padding(20)
        .frame(width: 560)
    }
}
