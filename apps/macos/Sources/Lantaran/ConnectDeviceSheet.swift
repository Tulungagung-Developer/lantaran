import CoreImage.CIFilterBuiltins
import LantaranCore
import SwiftUI

struct ConnectDeviceSheet: View {
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @State private var platform = Platform.ios

    enum Platform: String, CaseIterable, Identifiable {
        case ios = "iPhone or iPad"
        case android = "Android"
        case emulator = "Android Emulator"
        var id: String { rawValue }
    }

    var body: some View {
        @Bindable var model = model

        VStack(alignment: .leading, spacing: 16) {
            Text("Connect a Device").font(.title2.weight(.semibold))

            Picker("", selection: $platform) {
                ForEach(Platform.allCases) { Text($0.rawValue).tag($0) }
            }
            .pickerStyle(.segmented)
            .labelsHidden()

            if platform == .emulator {
                emulatorSteps
            } else if let address = model.addressForDevices {
                deviceSteps(address: address)
            } else {
                VStack(alignment: .leading, spacing: 8) {
                    Label("Lantaran only accepts connections from this Mac.", systemImage: "exclamationmark.triangle")
                    Toggle("Accept connections from other devices on this network", isOn: Binding(
                        get: { model.acceptOtherDevices },
                        set: { model.acceptOtherDevices = $0; model.restart() }
                    ))
                    Text("Any device on the same network can then send traffic through this Mac while Lantaran runs. It cannot reach services that only listen on this Mac.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }

            HStack {
                Button("Show Certificate in Finder") { model.revealCertificate() }
                Spacer()
                Button("Done") { dismiss() }.keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .frame(width: 560)
    }

    private func deviceSteps(address: String) -> some View {
        let parts = address.split(separator: ":")
        let host = parts.dropLast().joined(separator: ":")
        let port = parts.last.map(String.init) ?? String(model.port)
        let page = "http://\(address)/"

        return HStack(alignment: .top, spacing: 20) {
            VStack(spacing: 6) {
                QRCodeView(text: page).frame(width: 170, height: 170)
                Text(page).font(.caption.monospaced()).textSelection(.enabled)
            }
            VStack(alignment: .leading, spacing: 10) {
                Step(number: 1, text: "Join the same Wi-Fi network as this Mac.")
                Step(number: 2, text: "Scan the code with the camera and follow the steps on the page to install the certificate.")
                if platform == .ios {
                    Step(number: 3, text: "Settings › General › About › Certificate Trust Settings: switch on Lantaran Root CA.")
                    Step(number: 4, text: "Settings › Wi-Fi › (i) next to the network › Configure Proxy › Manual.")
                } else {
                    Step(number: 3, text: "Install it under Settings › Security › Encryption & credentials › Install a certificate › CA certificate.")
                    Step(number: 4, text: "Settings › Wi-Fi › the network › Edit › Proxy › Manual.")
                }
                VStack(alignment: .leading, spacing: 2) {
                    LabeledContent("Server", value: host).textSelection(.enabled)
                    LabeledContent("Port", value: port).textSelection(.enabled)
                }
                .font(.callout.monospaced())
                .padding(.leading, 28)
                Step(number: 5, text: "Add the hosts you want to read to the SSL Proxying List.")
            }
        }
    }

    private var emulatorSteps: some View {
        VStack(alignment: .leading, spacing: 10) {
            Step(number: 1, text: "In the emulator: Settings › Network & internet › Internet › AndroidWifi › Edit › Proxy › Manual.")
            LabeledContent("Server", value: "10.0.2.2").font(.callout.monospaced()).padding(.leading, 28)
            LabeledContent("Port", value: String(model.port)).font(.callout.monospaced()).padding(.leading, 28)
            Step(number: 2, text: "In the emulator's browser open http://\(onboardingHost())/ and follow the steps to install the certificate.")
            Step(number: 3, text: "Browsers will be readable. Apps need a debug network_security_config that trusts user certificates, or a system certificate on a Google APIs image.")
        }
    }
}

private struct Step: View {
    let number: Int
    let text: String

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Text("\(number)")
                .font(.caption.weight(.bold))
                .frame(width: 20, height: 20)
                .background(Circle().fill(Color.accentColor.opacity(0.2)))
            Text(text).fixedSize(horizontal: false, vertical: true)
        }
    }
}

struct QRCodeView: View {
    let text: String

    var body: some View {
        if let image = Self.image(for: text) {
            Image(nsImage: image)
                .interpolation(.none)
                .resizable()
                .scaledToFit()
                .background(Color.white)
        } else {
            Color.secondary.opacity(0.2)
        }
    }

    private static func image(for text: String) -> NSImage? {
        let filter = CIFilter.qrCodeGenerator()
        filter.message = Data(text.utf8)
        filter.correctionLevel = "M"
        guard let output = filter.outputImage?.transformed(by: CGAffineTransform(scaleX: 8, y: 8)) else {
            return nil
        }
        let rep = NSCIImageRep(ciImage: output)
        let image = NSImage(size: rep.size)
        image.addRepresentation(rep)
        return image
    }
}
