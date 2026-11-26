import XCTest
import LantaranCore
@testable import Lantaran

final class QuickFilterTests: XCTestCase {
    private func flow(
        url: String = "https://api.test/v1/items", request: [Header] = [], response: [Header] = []
    ) -> Flow {
        var flow = Flow.request(id: 1, client: "127.0.0.1", clientPort: 1, startedAt: 0, method: "GET",
                                url: url, version: "HTTP/1.1", headers: request)
        flow.responseHeaders = response
        return flow
    }

    private func contentType(_ value: String) -> [Header] { [Header(name: "Content-Type", value: value)] }

    func testContentTypesPickTheirFilter() {
        XCTAssertTrue(QuickFilter.json.matches(flow(response: contentType("application/json; charset=utf-8"))))
        XCTAssertTrue(QuickFilter.document.matches(flow(response: contentType("text/html"))))
        XCTAssertTrue(QuickFilter.media.matches(flow(response: contentType("image/png"))))
        XCTAssertTrue(QuickFilter.js.matches(flow(response: contentType("text/javascript"))))
        XCTAssertTrue(QuickFilter.form.matches(flow(request: contentType("application/x-www-form-urlencoded"),
                                                    response: contentType("application/json"))))
        XCTAssertFalse(QuickFilter.json.matches(flow(response: contentType("text/html"))))
    }

    func testProtocolAndOther() {
        XCTAssertTrue(QuickFilter.https.matches(flow()))
        XCTAssertTrue(QuickFilter.http.matches(flow(url: "http://api.test/")))
        XCTAssertTrue(QuickFilter.other.matches(flow(response: contentType("application/octet-stream"))))
        XCTAssertFalse(QuickFilter.other.matches(flow(response: contentType("application/json"))))
    }

    func testGraphQLAndWebSocketAreRecognisedFromTheRequest() {
        XCTAssertTrue(QuickFilter.graphql.matches(flow(url: "https://api.test/graphql")))
        XCTAssertTrue(QuickFilter.websocket.matches(flow(request: [Header(name: "Upgrade", value: "websocket")])))
        XCTAssertFalse(QuickFilter.other.matches(flow(url: "https://api.test/graphql")))
    }
}
