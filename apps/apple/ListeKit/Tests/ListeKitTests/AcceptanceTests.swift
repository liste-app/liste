// The Apple acceptance driver (Section 15): scripts 1, 2, 5, and 7 against
// a real in-process host, with the `liste` binary as the other client.

import Foundation
import ListeCore
import XCTest

@testable import ListeKit

@MainActor
final class AcceptanceTests: XCTestCase {
    private var dir: URL!
    private var session: Session!

    override func setUp() async throws {
        // Unix socket paths are short; /tmp keeps them under the limit.
        let nanos = UInt64(Date().timeIntervalSince1970 * 1_000_000) % 1_000_000_000
        dir = URL(fileURLWithPath: "/tmp/liste-k-\(ProcessInfo.processInfo.processIdentifier)-\(nanos)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        session = try Session(dataDir: dir.path, socketPath: dir.appendingPathComponent("host.sock").path)
    }

    override func tearDown() async throws {
        session?.shutdown()
        session = nil
        try? FileManager.default.removeItem(at: dir)
    }

    /// The `liste` binary built by cargo, two levels up from the package.
    private func cli(_ args: [String]) throws -> (Int32, String) {
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        let candidates = [
            ProcessInfo.processInfo.environment["LISTE_CLI"],
            root.appendingPathComponent("target/debug/liste").path,
            root.appendingPathComponent("target/release/liste").path,
        ].compactMap { $0 }.filter { FileManager.default.isExecutableFile(atPath: $0) }
        guard let bin = candidates.first else {
            throw XCTSkip("liste binary not built; run `cargo build -p liste-cli` or set LISTE_CLI")
        }
        let process = Process()
        process.executableURL = URL(fileURLWithPath: bin)
        process.arguments = args
        process.environment = [
            "LISTE_DATA_DIR": dir.path,
            "LISTE_SOCKET": dir.appendingPathComponent("host.sock").path,
            "PATH": ProcessInfo.processInfo.environment["PATH"] ?? "/usr/bin:/bin",
        ]
        let out = Pipe()
        process.standardOutput = out
        process.standardError = FileHandle.nullDevice
        try process.run()
        process.waitUntilExit()
        let data = out.fileHandleForReading.readDataToEndOfFile()
        return (process.terminationStatus, String(decoding: data, as: UTF8.self))
    }

    // Script 1: natural-language capture creates the right task.
    func testCaptureCreatesTheRightTask() throws {
        let captured = try session.capture("call mom tomorrow 5pm #family !high /Errands")
        let task = captured.task
        XCTAssertEqual(task.title, "call mom")
        XCTAssertEqual(task.priority, "high")
        XCTAssertEqual(task.tags, ["family"])
        XCTAssertEqual(task.listTitle, "Errands")
        XCTAssertFalse(task.dueAllDay)
        XCTAssertTrue(task.due?.hasSuffix("17:00") == true, task.due ?? "nil")
        XCTAssertEqual(captured.spans.map(\.kind), ["date", "time", "tag", "priority", "list"])
        // The preview the panel shows agrees with what capture creates.
        let preview = try XCTUnwrap(session.preview("call mom tomorrow 5pm #family !high /Errands"))
        XCTAssertEqual(preview.title, "call mom")
        XCTAssertEqual(preview.spans.map(\.kind), captured.spans.map(\.kind))
        let highlighted = highlighted("call mom tomorrow 5pm", spans: preview.spans)
        XCTAssertEqual(String(highlighted.characters), "call mom tomorrow 5pm")
    }

    // Script 2: the create is visible immediately, with no network.
    func testCreateIsVisibleImmediatelyOffline() throws {
        session.selection = .today
        XCTAssertTrue(session.tasks.isEmpty)
        try session.capture("pay rent today")
        // The change notification has fired and the session refreshed; no
        // sync has happened (there is no network client), and the task is
        // in the list already.
        let deadline = Date().addingTimeInterval(2)
        while session.tasks.isEmpty, Date() < deadline {
            RunLoop.main.run(until: Date().addingTimeInterval(0.01))
        }
        XCTAssertEqual(session.tasks.map(\.title), ["pay rent"])
        XCTAssertEqual(session.status?.pendingOps ?? 0 > 0, true, "queued for push, not pushed")
    }

    // Script 5: undo restores the previous state.
    func testUndoRestoresThePreviousState() throws {
        session.selection = .inbox
        let captured = try session.capture("buy milk")
        session.refresh()
        XCTAssertEqual(session.tasks.map(\.id), [captured.task.id])
        session.setCompleted(captured.task, true)
        XCTAssertTrue(session.tasks.isEmpty, "completed tasks leave the open list")
        session.undo()
        XCTAssertEqual(session.tasks.map(\.id), [captured.task.id])
        session.undo()
        XCTAssertTrue(session.tasks.isEmpty, "undoing the capture removes the task")
        session.redo()
        XCTAssertEqual(session.tasks.map(\.title), ["buy milk"])
    }

    // Script 7: complete via the CLI shows completed in the GUI, and the
    // reverse, on the same store through the app's own host.
    func testCliAndGuiAgree() throws {
        session.selection = .inbox
        let captured = try session.capture("renew passport")
        let (code, _) = try cli(["complete", captured.task.id])
        XCTAssertEqual(code, 0)
        let deadline = Date().addingTimeInterval(2)
        while !session.tasks.isEmpty, Date() < deadline {
            RunLoop.main.run(until: Date().addingTimeInterval(0.01))
        }
        XCTAssertTrue(session.tasks.isEmpty, "the GUI saw the CLI's completion without polling")
        XCTAssertNotNil(try session.task(id: captured.task.id).completedAt)
        // The reverse: uncomplete in the GUI, the CLI sees it.
        session.setCompleted(captured.task, false)
        let (code2, out) = try cli(["--json", "show", captured.task.id])
        XCTAssertEqual(code2, 0)
        let json = try XCTUnwrap(try JSONSerialization.jsonObject(with: Data(out.utf8)) as? [String: Any])
        XCTAssertNil(json["completed_at"])
        // And a CLI capture appears in the GUI's list.
        let (code3, _) = try cli(["capture", "book flights"])
        XCTAssertEqual(code3, 0)
        let deadline2 = Date().addingTimeInterval(2)
        while session.tasks.count < 2, Date() < deadline2 {
            RunLoop.main.run(until: Date().addingTimeInterval(0.01))
        }
        XCTAssertEqual(Set(session.tasks.map(\.title)), ["renew passport", "book flights"])
    }

    func testASecondHostOnTheSameStoreIsRefused() {
        XCTAssertThrowsError(
            try Session(dataDir: dir.path, socketPath: dir.appendingPathComponent("other.sock").path)
        ) { error in
            guard case ListeError.StoreHeld = error else {
                return XCTFail("expected StoreHeld, got \(error)")
            }
        }
    }
}

@MainActor
final class FixtureAcceptanceTests: XCTestCase {
    private var dir: URL!
    private var session: Session!

    override func setUp() async throws {
        let nanos = UInt64(Date().timeIntervalSince1970 * 1_000_000) % 1_000_000_000
        dir = URL(fileURLWithPath: "/tmp/liste-f-\(ProcessInfo.processInfo.processIdentifier)-\(nanos)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        session = try Session(dataDir: dir.path, socketPath: dir.appendingPathComponent("host.sock").path)
    }

    override func tearDown() async throws {
        session?.shutdown()
        session = nil
        try? FileManager.default.removeItem(at: dir)
    }

    // Script 6: search returns on the first keystroke offline, including
    // against the large fixture.
    func testSearchReturnsOnEveryKeystrokeAgainstTheLargeFixture() throws {
        let start = Date()
        let created = session.populateFixture(tasks: 50_000)
        XCTAssertEqual(created, 50_000)
        print("fixture: 50,000 tasks in \(String(format: "%.1f", Date().timeIntervalSince(start))) s")
        var worst = 0.0
        var hits = 0
        for query in ["c", "ca", "cal", "call", "call m", "renew p", "x", "gym", "quarterly report"] {
            let t = Date()
            let results = session.search(query, limit: 50)
            let ms = Date().timeIntervalSince(t) * 1000
            worst = max(worst, ms)
            hits += results.count
            print("search \(query.debugDescription): \(results.count) hits in \(String(format: "%.2f", ms)) ms")
            XCTAssertLessThan(ms, 50, "keystroke \(query.debugDescription) took \(ms) ms")
        }
        XCTAssertGreaterThan(hits, 0)
        print("search worst keystroke: \(String(format: "%.2f", worst)) ms")
        // Opening a long list is a count and one window, not every row.
        let t = Date()
        session.selection = .anytime
        let listMs = Date().timeIntervalSince(t) * 1000
        print("anytime: \(session.count) rows, \(session.tasks.count) loaded, in \(String(format: "%.1f", listMs)) ms")
        XCTAssertGreaterThan(session.count, 30_000)
        XCTAssertEqual(session.tasks.count, session.firstWindow)
        XCTAssertLessThan(listMs, 16, "opening Anytime took \(listMs) ms")
        // Scrolling deep into the list loads that window and nothing else.
        let t2 = Date()
        session.ensureLoaded(20_000..<20_030)
        let windowMs = Date().timeIntervalSince(t2) * 1000
        XCTAssertEqual(session.windowStart, 20_000 - session.margin)
        XCTAssertNotNil(session.task(at: 20_029))
        XCTAssertNil(session.task(at: 0))
        XCTAssertLessThan(windowMs, 16, "loading a window took \(windowMs) ms")
        // A covered range costs nothing and keeps the window.
        session.ensureLoaded(20_010..<20_020)
        XCTAssertEqual(session.windowStart, 20_000 - session.margin)
    }

    func testReorderAndUndoRestoreManualOrder() throws {
        session.selection = .inbox
        let a = try session.capture("first").task
        let b = try session.capture("second").task
        let c = try session.capture("third").task
        XCTAssertEqual(session.tasks.map(\.id), [a.id, b.id, c.id])
        session.reorder(c, after: nil, before: a)
        XCTAssertEqual(session.tasks.map(\.id), [c.id, a.id, b.id])
        session.move(a, up: true)
        XCTAssertEqual(session.tasks.map(\.id), [a.id, c.id, b.id])
        session.undo()
        XCTAssertEqual(session.tasks.map(\.id), [c.id, a.id, b.id])
        session.undo()
        XCTAssertEqual(session.tasks.map(\.id), [a.id, b.id, c.id], "undo restores the order")
        // Subtasks come right under their parent, indented; collapsing the
        // parent hides them from the rows and the count.
        session.setParent(b, a.id)
        XCTAssertEqual(session.tasks.map { "\($0.title):\($0.depth)" }, ["first:0", "second:1", "third:0"])
        XCTAssertTrue(session.tasks[0].hasSubtasks)
        session.collapsed = [a.id]
        XCTAssertEqual(session.tasks.map(\.id), [a.id, c.id])
        XCTAssertEqual(session.count, 2)
        session.collapsed = []
        XCTAssertEqual(session.count, 3)
        // Saved filters live in the core and select through it.
        let filter = try XCTUnwrap(session.createFilter("Firsts", FilterCriteria(priority: "high")))
        XCTAssertEqual(session.filters.map(\.name), ["Firsts"])
        session.setPriority(a, "high")
        session.selection = .filter(id: filter.id)
        XCTAssertEqual(session.tasks.map(\.id), [a.id])
        session.deleteFilter(filter)
        XCTAssertTrue(session.filters.isEmpty)
        session.undo()
        XCTAssertEqual(session.filters.map(\.id), [filter.id], "undo restores a deleted filter")
        session.selection = .inbox
        let list = try XCTUnwrap(session.createList("Errands"))
        session.setList(c, list.title)
        XCTAssertEqual(try session.task(id: c.id).listTitle, "Errands")
        session.addTag(c, "weekend")
        XCTAssertEqual(session.tags.map(\.name), ["weekend"])
        session.selection = .tag(id: session.tags[0].id)
        XCTAssertEqual(session.tasks.map(\.id), [c.id])
        session.delete(c)
        XCTAssertTrue(session.tasks.isEmpty)
        session.undo()
        XCTAssertEqual(session.tasks.map(\.id), [c.id], "undo restores a deleted task")
    }
}
