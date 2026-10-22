import HushKit
import XCTest
@testable import Hush

@MainActor
final class ViewModelTests: XCTestCase {
    private var store: HushStore!

    override func setUp() async throws {
        store = try HushStore.start(.init(ephemeral: true))
    }

    override func tearDown() async throws {
        store.engine.shutdown()
        store = nil
    }

    private func link(group: Bool = false, passphrase: Bool = false, minutes: UInt64 = 60, now: Date = Date()) -> String {
        linkNew(group: group, ttlMin: minutes, passphrase: passphrase, nowMs: UInt64(now.timeIntervalSince1970 * 1000), base: linkBase)
    }

    func testJoinTellsLinksApart() throws {
        guard case let .success(info) = JoinModel.check(link(group: true)) else { return XCTFail("a good link") }
        XCTAssertTrue(info.group)
        XCTAssertFalse(info.passphrase)
        guard case let .success(p) = JoinModel.check("  \(link(passphrase: true))\n") else { return XCTFail("a protected link") }
        XCTAssertTrue(p.passphrase)
        XCTAssertEqual(JoinModel.check("hello"), .failure(.invalid))
        XCTAssertEqual(JoinModel.check(link(minutes: 10, now: Date(timeIntervalSinceNow: -3600))), .failure(.expired))
        XCTAssertTrue(JoinModel.isLink(link()))
        XCTAssertFalse(JoinModel.isLink("https://example.org/"))
    }

    func testAProtectedLinkAsksForItsPassphraseFirst() async {
        let model = JoinModel()
        let chat = await model.submit(link(passphrase: true), store: store)
        XCTAssertNil(chat)
        XCTAssertTrue(model.askPassphrase)
        XCTAssertNil(model.problem)
    }

    func testAWrongLinkSaysWhy() async {
        let model = JoinModel()
        let chat = await model.submit("not a link", store: store)
        XCTAssertNil(chat)
        XCTAssertEqual(model.problem, .invalid)
        XCTAssertEqual(model.problemText, String(localized: "invalidLink"))
    }

    func testCreatingALinkOpensItsConversation() async throws {
        let model = NewConversationModel()
        model.group = true
        model.minutes = 10
        let created = await model.create(store)
        let chat = try XCTUnwrap(created)
        for _ in 0..<100 where store.chat(chat)?.url == nil {
            try await Task.sleep(nanoseconds: 20_000_000)
        }
        let c = try XCTUnwrap(store.chat(chat))
        XCTAssertTrue(c.group && c.creator)
        let url = try XCTUnwrap(c.url)
        XCTAssertTrue(url.hasPrefix(linkBase))
        XCTAssertTrue(try linkInfo(input: url).group)
        store.end(chat)
    }

    func testVaultChecksThePassphraseBeforeTheEngine() async {
        let model = VaultModel()
        model.passphrase = "short"
        model.confirm = "short"
        let short = await model.submit(store, creating: true)
        XCTAssertFalse(short)
        XCTAssertEqual(model.error, String(localized: "vaultTooShort"))
        model.passphrase = "a long passphrase"
        model.confirm = "another one"
        let mismatch = await model.submit(store, creating: true)
        XCTAssertFalse(mismatch)
        XCTAssertEqual(model.error, String(localized: "vaultMismatch"))
    }

    func testABackupPassphraseIsAsLongAsTheVaults() {
        XCTAssertEqual(VaultModel.problem("short", confirm: "short"), String(localized: "vaultTooShort"))
        XCTAssertEqual(VaultModel.problem("a long passphrase", confirm: "a long passphrasf"), String(localized: "vaultMismatch"))
        XCTAssertNil(VaultModel.problem("a long passphrase", confirm: "a long passphrase"))
    }

    func testALinkFromElsewhereWaitsForJoin() {
        let model = JoinModel()
        let url = link()
        model.prefill(url)
        XCTAssertEqual(model.input, url)
        XCTAssertNil(model.problem)
        XCTAssertNil(model.link)
        XCTAssertTrue(store.state.chats.isEmpty)
        model.prefill(link(minutes: 10, now: Date(timeIntervalSinceNow: -3600)))
        XCTAssertEqual(model.problem, .expired)
    }

    func testAddByIdRefusesTypos() {
        let model = AddByIdModel()
        model.input = "hush1typo"
        model.submit(store)
        XCTAssertTrue(model.invalid)
        XCTAssertNil(model.pending)
    }

    func testRouterOpensOnlyHushLinks() {
        let router = Router()
        XCTAssertFalse(router.open(URL(string: "https://example.org/")!))
        XCTAssertTrue(router.path.isEmpty)
        XCTAssertTrue(router.open(URL(string: link())!))
        guard case .join = router.path.first else { return XCTFail("the join screen") }
        router.show(chat: 4)
        XCTAssertEqual(router.path, [.chat(4)])
    }

    func testWording() {
        XCTAssertEqual(Wording.duration(ms: 59 * 60_000), "59 min")
        XCTAssertEqual(Wording.duration(ms: 3 * 3_600_000), "3 h")
        XCTAssertEqual(Wording.size(512), "512 B")
        XCTAssertEqual(Wording.size(1536), "1.5 KB")
        XCTAssertEqual(Wording.ttl(300), "5 min")
        XCTAssertEqual(Wording.linkMinutes, [10, 60, 1440, 10_080])
        let sas = Wording.sas(Data([0, 1, 2, 3, 63]))
        XCTAssertEqual(sas.count, 5)
        XCTAssertFalse(sas.contains { $0.emoji.isEmpty || $0.name.isEmpty })
        XCTAssertEqual(Wording.sasFrench.count, 64)
        XCTAssertFalse(Wording.notice(.joined(name: "Ada")).isEmpty)
        XCTAssertNotEqual(Wording.endTitle(.taken), Wording.endTitle(.ended))
    }
}
