import AppKit
import LantaranCore
import UniformTypeIdentifiers

/// The Certificate menu's Export, Import and Reset. Each asks with a panel
/// or an alert and reports a failure in an alert.
@MainActor
extension AppModel {
    enum CertificateExport {
        case pem, der, pkcs12

        var fileExtension: String {
            switch self {
            case .pem: "pem"
            case .der: "crt"
            case .pkcs12: "p12"
            }
        }
    }

    func exportCertificate(_ format: CertificateExport) {
        guard let engine else { return }

        let panel = NSSavePanel()
        panel.nameFieldStringValue = "Lantaran Root CA.\(format.fileExtension)"
        if let type = UTType(filenameExtension: format.fileExtension) {
            panel.allowedContentTypes = [type]
        }
        if format == .pkcs12 {
            panel.message = "The file holds the private key. Anyone with it and its password can read the traffic of every device that trusts this certificate."
        }
        guard panel.runModal() == .OK, let url = panel.url else { return }

        do {
            switch format {
            case .pem:
                try Data(engine.certificatePem().utf8).write(to: url)
            case .der:
                try engine.certificateDer().write(to: url)
            case .pkcs12:
                guard let password = Self.askForNewPassword() else { return }
                let data = try engine.exportPkcs12(password: password)
                try data.write(to: url)
                // The key is inside: readable by this user only.
                try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: url.path)
            }
        } catch {
            Self.showError("The certificate could not be exported.", error)
        }
    }

    func importCertificate() {
        guard let engine else { return }

        let panel = NSOpenPanel()
        panel.message = "Choose a .p12 file exported from Lantaran on another Mac."
        panel.allowedContentTypes = ["p12", "pfx"].compactMap { UTType(filenameExtension: $0) }
        guard panel.runModal() == .OK, let url = panel.url else { return }

        let confirm = NSAlert()
        confirm.alertStyle = .warning
        confirm.messageText = "Replace the root certificate?"
        confirm.informativeText = "Devices that trust \(engine.certificateName()) stop working with this Mac unless they also trust the imported certificate. Export the current one first if you may want it back."
        confirm.addButton(withTitle: "Import")
        confirm.addButton(withTitle: "Cancel")
        guard confirm.runModal() == .alertFirstButtonReturn else { return }

        do {
            let data = try Data(contentsOf: url)
            guard let password = Self.askForPassword(
                title: "Password for \(url.lastPathComponent)",
                confirming: false
            ) else { return }
            try engine.importPkcs12(data: data, password: password)
            rootCertificateChanged()
            Self.showDone("Imported \(engine.certificateName()).")
        } catch {
            Self.showError("The certificate could not be imported.", error)
        }
    }

    func resetCertificate() {
        guard let engine else { return }

        let confirm = NSAlert()
        confirm.alertStyle = .critical
        confirm.messageText = "Reset the root certificate?"
        confirm.informativeText = "Lantaran creates a new root certificate and key. Every device and simulator that trusts \(engine.certificateName()) has to install the new certificate and trust it again; remove the old Lantaran profile or certificate from them. This cannot be undone unless you have exported the current one as a .p12 file."
        confirm.addButton(withTitle: "Reset")
        confirm.addButton(withTitle: "Cancel")
        confirm.buttons.first?.hasDestructiveAction = true
        guard confirm.runModal() == .alertFirstButtonReturn else { return }

        do {
            try engine.resetCertificate()
            rootCertificateChanged()
            Self.showDone("Created \(engine.certificateName()). Set up your devices again from Connect Device.")
        } catch {
            Self.showError("The certificate could not be reset.", error)
        }
    }

    // MARK: Alerts

    private static func askForNewPassword() -> String? {
        askForPassword(title: "Choose a password for the file", confirming: true)
    }

    /// Asks for a password; with `confirming`, twice, and not empty.
    private static func askForPassword(title: String, confirming: Bool) -> String? {
        let first = NSSecureTextField(frame: NSRect(x: 0, y: confirming ? 30 : 0, width: 260, height: 22))
        first.placeholderString = "Password"
        let second = NSSecureTextField(frame: NSRect(x: 0, y: 0, width: 260, height: 22))
        second.placeholderString = "Again"
        let fields = NSView(frame: NSRect(x: 0, y: 0, width: 260, height: confirming ? 52 : 22))
        fields.addSubview(first)
        if confirming { fields.addSubview(second) }

        while true {
            let alert = NSAlert()
            alert.messageText = title
            alert.accessoryView = fields
            alert.addButton(withTitle: "OK")
            alert.addButton(withTitle: "Cancel")
            alert.window.initialFirstResponder = first
            guard alert.runModal() == .alertFirstButtonReturn else { return nil }

            let password = first.stringValue
            if confirming && password.isEmpty {
                showDone("The file holds the private key, so it needs a password.")
            } else if confirming && password != second.stringValue {
                showDone("The passwords do not match.")
            } else {
                return password
            }
        }
    }

    private static func showError(_ message: String, _ error: Error) {
        let alert = NSAlert()
        alert.alertStyle = .warning
        alert.messageText = message
        alert.informativeText = Self.message(error)
        alert.runModal()
    }

    private static func showDone(_ message: String) {
        let alert = NSAlert()
        alert.messageText = message
        alert.runModal()
    }
}
