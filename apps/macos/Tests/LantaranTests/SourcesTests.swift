import Darwin
import XCTest
@testable import Lantaran

final class UserAgentTests: XCTestCase {
    func testIOSAppsAreNamedFromCFNetworkAgents() {
        XCTAssertEqual(UserAgent.appName("MyApp/42 CFNetwork/1490.0.4 Darwin/23.2.0"), "MyApp")
        XCTAssertEqual(UserAgent.appName("My%20Shop/1.0 CFNetwork/1490 Darwin/23"), "My Shop")
    }

    func testBrowsersAreNamed() {
        let safari = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.1 Mobile/15E148 Safari/604.1"
        XCTAssertEqual(UserAgent.appName(safari), "Safari")
        XCTAssertEqual(UserAgent.deviceKind(safari), "iPhone")
        let chrome = "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Mobile Safari/537.36"
        XCTAssertEqual(UserAgent.appName(chrome), "Chrome")
        XCTAssertEqual(UserAgent.deviceKind(chrome), "Android")
        XCTAssertEqual(UserAgent.appName("Mozilla/5.0 (iPad; CPU OS 18_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148"), "Web view")
    }

    func testUnhelpfulAgents() {
        XCTAssertNil(UserAgent.appName(nil))
        XCTAssertNil(UserAgent.appName("  "))
        XCTAssertEqual(UserAgent.appName("okhttp/4.12.0"), "okhttp")
        XCTAssertEqual(UserAgent.appName("Dalvik/2.1.0 (Linux; U; Android 14; Pixel 8)"), "Android system")
        XCTAssertEqual(UserAgent.appName("curl/8.7.1"), "curl")
    }
}

final class SidebarTreeTests: XCTestCase {
    private func flow(_ id: UInt64, device: Device, app: String, host: String) -> Flow {
        var flow = Flow.tunnel(id: id, client: "127.0.0.1", clientPort: 1, authority: "\(host):443")
        flow.device = device
        flow.app = app
        return flow
    }

    func testCountsByDeviceAppAndDomain() {
        var tree = SidebarTree()
        let sim = Device.simulator(udid: "A")
        tree.add(flow(1, device: .mac, app: "curl", host: "a.test"))
        tree.add(flow(2, device: .mac, app: "curl", host: "b.test"))
        tree.add(flow(3, device: sim, app: "Shop", host: "a.test"))

        XCTAssertEqual(tree.devices[.mac]?.count, 2)
        XCTAssertEqual(tree.devices[.mac]?.apps["curl"]?.domains, ["a.test": 1, "b.test": 1])
        XCTAssertEqual(tree.devices[sim]?.apps["Shop"]?.count, 1)
        XCTAssertEqual(tree.domains, ["a.test": 2, "b.test": 1])
    }

    func testMovingAFlowLeavesNoEmptyNodes() {
        var tree = SidebarTree()
        var moving = flow(1, device: .mac, app: "Unknown", host: "a.test")
        tree.add(moving)
        tree.remove(moving)
        moving.device = .simulator(udid: "A")
        moving.app = "Shop"
        tree.add(moving)

        XCTAssertNil(tree.devices[.mac])
        XCTAssertEqual(tree.devices[.simulator(udid: "A")]?.apps["Shop"]?.count, 1)
        XCTAssertEqual(tree.domains, ["a.test": 1])
    }
}

final class ClientProcessScannerTests: XCTestCase {
    /// Connects to a listener in this process and checks the scan names
    /// this process as the owner of the client end.
    func testFindsTheProcessBehindAConnection() throws {
        let listener = socket(AF_INET, SOCK_STREAM, 0)
        defer { close(listener) }
        var address = sockaddr_in()
        address.sin_family = sa_family_t(AF_INET)
        address.sin_addr.s_addr = inet_addr("127.0.0.1")
        address.sin_port = 0
        var length = socklen_t(MemoryLayout<sockaddr_in>.size)
        withUnsafeMutablePointer(to: &address) { pointer in
            pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                XCTAssertEqual(Darwin.bind(listener, $0, length), 0)
                XCTAssertEqual(Darwin.listen(listener, 1), 0)
                XCTAssertEqual(getsockname(listener, $0, &length), 0)
            }
        }
        let proxyPort = UInt16(bigEndian: address.sin_port)

        let client = socket(AF_INET, SOCK_STREAM, 0)
        defer { close(client) }
        var target = address
        let connected = withUnsafePointer(to: &target) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                connect(client, $0, socklen_t(MemoryLayout<sockaddr_in>.size))
            }
        }
        XCTAssertEqual(connected, 0)
        var local = sockaddr_in()
        var localLength = socklen_t(MemoryLayout<sockaddr_in>.size)
        _ = withUnsafeMutablePointer(to: &local) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { getsockname(client, $0, &localLength) }
        }
        let clientPort = UInt16(bigEndian: local.sin_port)

        let found = ClientProcessScanner().scan(proxyPort: proxyPort, excluding: 0)

        let process = try XCTUnwrap(found[clientPort])
        XCTAssertEqual(process.pid, getpid())
        XCTAssertNil(process.simulatorUDID)
        XCTAssertFalse(process.isAndroidEmulator)
        XCTAssertFalse(process.name.isEmpty)
    }

    func testBundlePathIsTheInnermostApp() {
        XCTAssertEqual(
            ClientProcessScanner.bundlePath(of: "/Applications/Google Chrome.app/Contents/Frameworks/Helpers/Google Chrome Helper.app/Contents/MacOS/Google Chrome Helper"),
            "/Applications/Google Chrome.app/Contents/Frameworks/Helpers/Google Chrome Helper.app"
        )
        XCTAssertNil(ClientProcessScanner.bundlePath(of: "/usr/bin/curl"))
    }
}
