//
//  BitRufusTests.swift
//  BitRufusTests
//
//  Created by Тимофей Ермилов on 23.05.2026.
//

import XCTest
@testable import BitRufus

final class BitRufusTests: XCTestCase {

    func testPingReturnsPong() {
        XCTAssertEqual(ping(), "pong")
    }

}
