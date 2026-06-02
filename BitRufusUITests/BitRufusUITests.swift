//
//  BitRufusUITests.swift
//  BitRufusUITests
//
//  Created by Tim Rufus on 23.05.2026.
//

import XCTest

final class BitRufusUITests: XCTestCase {

    override func setUpWithError() throws {
        continueAfterFailure = false
    }

    func testAddTorrentButtonVisible() throws {
        let app = XCUIApplication()
        app.launch()
        let button = app.buttons["Add Torrent"]
        XCTAssertTrue(button.waitForExistence(timeout: 5), "Expected Add Torrent toolbar button in TorrentListView")
    }

    func testLaunchPerformance() throws {
        measure(metrics: [XCTApplicationLaunchMetric()]) {
            XCUIApplication().launch()
        }
    }
}
