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
                Menu("Install Certificate on iOS") {
                    Button("Physical Devices…") { model.connectDevice(.ios) }
                    Button("Simulators…") { model.connectDevice(.simulator) }
                }
                Menu("Install Certificate on Android") {
                    Button("Physical Devices…") { model.connectDevice(.android) }
                    Button("Emulators…") { model.connectDevice(.emulator) }
                }
                Divider()
                Button("Show Root Certificate in Finder") { model.revealCertificate() }
            }
            CommandMenu("Tools") {
                Button("SSL Proxying List…") { model.showSSLProxying = true }
                    .keyboardShortcut("p", modifiers: [.command, .option])
                Divider()
                Button("Map Local…") { model.showMapLocal = true }
                    .keyboardShortcut("l", modifiers: [.command, .option])
                Button("Map Remote…") { model.showMapRemote = true }
                    .keyboardShortcut("r", modifiers: [.command, .option])
                Divider()
                Toggle("Route This Mac's Traffic Through Lantaran", isOn: Binding(
                    get: { model.systemProxyOn },
                    set: { model.setSystemProxy($0) }
                ))
                .keyboardShortcut("m", modifiers: [.command, .option])
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

    func applicationWillTerminate(_ notification: Notification) {
        MainActor.assumeIsolated {
            AppModel.active?.prepareToQuit()
        }
    }
}
