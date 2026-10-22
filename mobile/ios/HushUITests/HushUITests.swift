import XCTest

final class HushUITests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    /// Launch, create a link: its QR code and its text appear.
    func testCreateALink() throws {
        let app = XCUIApplication()
        app.launchArguments = ["-ephemeral"]
        app.launch()

        let start = app.buttons["new-conversation"]
        XCTAssertTrue(start.waitForExistence(timeout: 20))
        start.tap()

        let create = app.buttons["create-link"]
        XCTAssertTrue(create.waitForExistence(timeout: 10))
        create.tap()

        let link = app.staticTexts["share-link"]
        XCTAssertTrue(link.waitForExistence(timeout: 20))
        XCTAssertTrue(link.label.hasPrefix("https://maxgfr.github.io/hush/#"), link.label)
        XCTAssertTrue(app.descendants(matching: .any)["qr-code"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["end"].exists)
    }
}
