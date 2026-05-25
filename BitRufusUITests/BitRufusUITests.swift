//
//  BitRufusUITests.swift
//  BitRufusUITests
//
//  Created by Тимофей Ермилов on 23.05.2026.
//

import XCTest

final class BitRufusUITests: XCTestCase {

    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    func testRustPingLabelVisible() throws {
        let app = XCUIApplication()
        app.launch()
        let label = app.staticTexts["rust-ping-label"]
        XCTAssertTrue(label.waitForExistence(timeout: 5), "Expected rust-ping-label from FFI roundtrip")
        XCTAssertEqual(label.label, "Rust: pong", "rust-ping-label must show FFI response")
    }

    func testLaunchPerformance() throws {
        if #available(macOS 10.15, iOS 13.0, tvOS 13.0, watchOS 7.0, *) {
            measure(metrics: [XCTApplicationLaunchMetric()]) {
                XCUIApplication().launch()
            }
        }
    }
}
