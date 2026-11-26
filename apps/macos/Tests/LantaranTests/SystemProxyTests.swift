import XCTest
@testable import Lantaran

final class SystemProxyTests: XCTestCase {
    func testParsesAnEnabledProxy() {
        let output = """
        Enabled: Yes
        Server: 127.0.0.1
        Port: 9090
        Authenticated Proxy Enabled: 0
        """
        XCTAssertEqual(SystemProxy.parse(output), .init(enabled: true, server: "127.0.0.1", port: 9090))
    }

    func testParsesAnUnsetProxy() {
        let output = """
        Enabled: No
        Server:
        Port: 0
        Authenticated Proxy Enabled: 0
        """
        XCTAssertEqual(SystemProxy.parse(output), .init(enabled: false, server: "", port: 0))
    }

    func testRestoringAnEnabledProxySetsServerThenState() {
        let commands = SystemProxy.commandsToRestore(
            "Wi-Fi", .init(enabled: true, server: "10.0.0.2", port: 8080),
            setter: "-setwebproxy", state: "-setwebproxystate")
        XCTAssertEqual(commands, [
            ["-setwebproxy", "Wi-Fi", "10.0.0.2", "8080"],
            ["-setwebproxystate", "Wi-Fi", "on"],
        ])
    }

    func testRestoringADisabledProxyWithAServerKeepsTheServerButTurnsItOff() {
        // Setting a server switches the proxy on, so the state must come last.
        let commands = SystemProxy.commandsToRestore(
            "Wi-Fi", .init(enabled: false, server: "10.0.0.2", port: 8080),
            setter: "-setsecurewebproxy", state: "-setsecurewebproxystate")
        XCTAssertEqual(commands.last, ["-setsecurewebproxystate", "Wi-Fi", "off"])
    }

    func testRestoringANeverConfiguredProxyOnlyTurnsItOff() {
        let commands = SystemProxy.commandsToRestore(
            "Thunderbolt Bridge", .init(enabled: false, server: "", port: 0),
            setter: "-setwebproxy", state: "-setwebproxystate")
        XCTAssertEqual(commands, [["-setwebproxystate", "Thunderbolt Bridge", "off"]])
    }
}

final class SimulatorsTests: XCTestCase {
    func testRuntimeNamesAreReadable() {
        XCTAssertEqual(Simulators.runtimeName("com.apple.CoreSimulator.SimRuntime.iOS-27-0"), "iOS 27.0")
        XCTAssertEqual(Simulators.runtimeName("com.apple.CoreSimulator.SimRuntime.watchOS-12-1"), "watchOS 12.1")
    }
}

final class BodyFormatterTests: XCTestCase {
    func testJSONIsIndentedWithoutReorderingKeys() {
        let text = BodyFormatter.indentJSON(#"{"b":1,"a":[1,2],"e":{}}"#)
        XCTAssertEqual(text, """
        {
          "b": 1,
          "a": [
            1,
            2
          ],
          "e": {}
        }
        """)
    }

    func testCommasAndColonsInsideStringsAreLeftAlone() {
        XCTAssertEqual(BodyFormatter.indentJSON(#"{"k":"a, b: {c}"}"#), "{\n  \"k\": \"a, b: {c}\"\n}")
    }
}

final class CurlTests: XCTestCase {
    func testTheMethodIsQuoted() {
        var flow = Flow.request(id: 1, client: "127.0.0.1", startedAt: 0, method: "GET|id",
                                url: "https://example.com/", version: "HTTP/1.1", headers: [])
        flow.method = "GET|id"
        XCTAssertTrue(Curl.command(for: flow, body: nil).contains("-X 'GET|id'"))
    }
}
