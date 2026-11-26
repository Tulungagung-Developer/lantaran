import AppKit
import SwiftUI

@main
struct LantaranApp: App {
    @NSApplicationDelegateAdaptor private var delegate: AppDelegate
    @State private var model = AppModel()

    var body: some Scene {
        Window("Lantaran", id: "main") {
            ContentView()
                .environment(model)
                .frame(minWidth: 960, minHeight: 600)
        }
        .commands {
            CommandGroup(after: .newItem) {
                Button("Clear") { model.clear() }
                    .keyboardShortcut("k", modifiers: .command)
                Button(model.running ? "Stop Proxy" : "Start Proxy") {
                    model.running ? model.stop() : model.start()
                }
                .keyboardShortcut("r", modifiers: [.command, .shift])
            }
            CommandMenu("Certificate") {
                Button("Connect a Device…") { model.showConnectDevice = true }
                    .keyboardShortcut("d", modifiers: [.command, .shift])
                Divider()
                Button("Show Root Certificate in Finder") { model.revealCertificate() }
            }
            CommandMenu("Tools") {
                Button("SSL Proxying List…") { model.showSSLProxying = true }
                    .keyboardShortcut("p", modifiers: [.command, .option])
            }
        }

        Settings {
            SettingsView().environment(model)
        }
    }
}

final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        // Started from a plain executable, the app would otherwise stay in
        // the background without a Dock icon.
        NSApp.setActivationPolicy(.regular)
        NSApp.activate(ignoringOtherApps: true)
    }

    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        true
    }
}
