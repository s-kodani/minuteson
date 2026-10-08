import Darwin
import Foundation
import XCTest

@testable import MONMacOSCapture

private final class JournalGate: @unchecked Sendable {
  private let lock = NSLock()
  private var held = false
  let entered = DispatchSemaphore(value: 0)
  let released = DispatchSemaphore(value: 0)
  func hold() { lock.withLock { held = true } }
  func write() {
    if lock.withLock({ held }) {
      entered.signal()
      _ = released.wait(timeout: .now() + 10)
    }
  }
}

private final class JournalFailure: @unchecked Sendable {
  private let lock = NSLock()
  private var failing = true
  func recover() { lock.withLock { failing = false } }
  func write() throws {
    if lock.withLock({ failing }) { throw ProbeError.capture("synthetic journal I/O failure") }
  }
}

final class InputEventJournalTests: XCTestCase {
  private let owner = "735027d4-a03e-43d3-830a-21e767dfaf88"
  private let device = CaptureInputDevice(
    uid: "confirmed-usb", name: "USB", isDefault: false, objectID: 2)

  private func directory() throws -> URL {
    let value = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: value, withIntermediateDirectories: false)
    let canonical = try XCTUnwrap(realpath(value.path, nil))
    defer { free(canonical) }
    return URL(fileURLWithPath: String(cString: canonical), isDirectory: true)
  }

  private func read(_ directory: URL) throws -> [String: Any] {
    try XCTUnwrap(
      JSONSerialization.jsonObject(
        with: Data(contentsOf: directory.appendingPathComponent("input-events.json")))
        as? [String: Any])
  }

  private func mic(_ journal: InputEventJournal) throws -> MicrophoneRecovery {
    var policy = try MicrophoneRecovery(uid: device.uid, observe: journal.observer(.microphone))
    let token = try policy.begin(device)
    try policy.activate(token)
    return policy
  }

  private func cycle(_ policy: inout MicrophoneRecovery) throws {
    policy.interrupt("device_disconnected")
    let token = try policy.begin(device)
    try policy.activate(token)
  }

  func testNormalOwnedOptionsPreserveCanonicalNativePathInsteadOfSystemAlias() throws {
    let output = "/private/tmp/native-owned-recording"
    let options = try Options.parse([
      "--output", output, "--until-stopped", "--stop-on-file", "--parent-pid", "1",
      "--input-level-owner", owner,
    ])
    XCTAssertTrue(options.publishesInputStatus)
    XCTAssertEqual(options.output.path, output)
  }

  func testPolicyTransitionsAreRetainedWithoutHeartbeatAndSealedWithoutRendering() throws {
    let output = try directory()
    defer { try? FileManager.default.removeItem(at: output) }
    let journal = try InputEventJournal(output: output, owner: owner, supportsSystem: true)
    var mic = try mic(journal)
    var sys = try SystemAudioRecovery(
      targetPID: nil, applicationIdentity: nil, retries: true, observe: journal.observer(.system))
    let first = try sys.begin(at: 1)
    try sys.activate(first)
    sys.operationFinished(first)
    try cycle(&mic)
    sys.interrupt("stream_stopped")
    let second = try sys.begin(at: 1_000_000_001)
    try sys.activate(second)
    sys.operationFinished(second)
    try journal.flush()
    XCTAssertEqual(try read(output)["sealed"] as? Bool, false)
    mic.stop()
    sys.stop()
    try journal.finish()
    let value = try read(output)
    XCTAssertEqual(Set(value.keys), ["schemaVersion", "sealed", "report"])
    XCTAssertEqual(value["schemaVersion"] as? Int, 1)
    XCTAssertEqual(value["sealed"] as? Bool, true)
    let report = try XCTUnwrap(value["report"] as? [String: Any])
    XCTAssertEqual(report["ownerId"] as? String, owner)
    for lane in ["microphone", "system"] {
      XCTAssertEqual((report[lane] as? [String: Any])?["state"] as? String, "stopped")
      let history = try XCTUnwrap(report["\(lane)History"] as? [String: Any])
      XCTAssertEqual(history["lastSequence"] as? Int, 2)
      XCTAssertEqual(history["droppedThrough"] as? Int, 0)
      let events = try XCTUnwrap(history["events"] as? [[String: Any]])
      XCTAssertEqual(events.compactMap { $0["kind"] as? String }, ["interrupted", "recovered"])
      XCTAssertEqual(events.compactMap { $0["sequence"] as? Int }, [1, 2])
    }
    XCTAssertFalse(
      FileManager.default.fileExists(
        atPath: output.appendingPathComponent("input-status.json").path))
    let bytes = try Data(contentsOf: output.appendingPathComponent("input-events.json"))
    try journal.finish()  // no new revision or claim of rendering
    XCTAssertEqual(try Data(contentsOf: output.appendingPathComponent("input-events.json")), bytes)
    if let export = ProcessInfo.processInfo.environment["MON_INPUT_JOURNAL_FIXTURE_DIR"] {
      let target = URL(fileURLWithPath: export, isDirectory: true)
      XCTAssertTrue(try FileManager.default.contentsOfDirectory(atPath: target.path).isEmpty)
      try bytes.write(
        to: target.appendingPathComponent("input-events.json"), options: .withoutOverwriting)
      try FileManager.default.setAttributes(
        [.posixPermissions: 0o600],
        ofItemAtPath: target.appendingPathComponent("input-events.json").path)
      try InputJournalStatusReport(
        ownerId: owner, revision: 123, persistence: journal.persistence()
      )
      .publish(to: target)
    }
  }

  func testStalledWriterDoesNotBlockPolicyAndBoundsPendingSuffixWithExplicitLoss() throws {
    let output = try directory()
    defer { try? FileManager.default.removeItem(at: output) }
    let gate = JournalGate()
    gate.hold()
    let journal = try InputEventJournal(
      output: output, owner: owner, supportsSystem: false, beforeWrite: { gate.write() })
    var policy = try mic(journal)
    XCTAssertEqual(gate.entered.wait(timeout: .now() + 2), .success)
    // Writer is definitely stalled. These synchronous mutations must finish
    // without waiting for its file or semaphore, and coalesce bounded copies.
    for _ in 0..<200 { try cycle(&policy) }
    XCTAssertEqual(policy.history.lastSequence, 400)
    XCTAssertEqual(policy.history.events.count, 64)
    XCTAssertThrowsError(try journal.flush(timeout: 0.01))
    policy.stop()
    gate.released.signal()
    gate.released.signal()  // last coalesced snapshot
    gate.released.signal()  // sealing, if the writer caught up first
    try journal.finish()
    let report = try XCTUnwrap(try read(output)["report"] as? [String: Any])
    let history = try XCTUnwrap(report["microphoneHistory"] as? [String: Any])
    XCTAssertEqual(history["lastSequence"] as? Int, 400)
    XCTAssertEqual(history["droppedThrough"] as? Int, 336)
    XCTAssertEqual((history["events"] as? [Any])?.count, 64)
    XCTAssertTrue(report["system"] is NSNull)
    XCTAssertTrue(report["systemHistory"] is NSNull)
  }

  func testUnexpectedLinkOrModifiedLeafIsRetainedWithoutClaimingCommit() throws {
    for kind in ["symlink", "hardlink", "modified"] {
      let output = try directory()
      defer { try? FileManager.default.removeItem(at: output) }
      let journal = try InputEventJournal(output: output, owner: owner, supportsSystem: false)
      var policy = try mic(journal)
      try journal.flush()
      let leaf = output.appendingPathComponent("input-events.json")
      let external = output.appendingPathComponent("foreign.json")
      let marker = Data("unrelated file must survive".utf8)
      try marker.write(to: external)
      if kind == "modified" {
        try marker.write(to: leaf)
      } else {
        try FileManager.default.removeItem(at: leaf)
        if kind == "symlink" {
          try FileManager.default.createSymbolicLink(at: leaf, withDestinationURL: external)
        } else {
          XCTAssertEqual(link(external.path, leaf.path), 0)
        }
      }
      try cycle(&policy)
      XCTAssertThrowsError(try journal.flush())
      XCTAssertThrowsError(try journal.finish())
      XCTAssertEqual(try Data(contentsOf: external), marker)
      XCTAssertEqual(try Data(contentsOf: leaf), marker)
      XCTAssertEqual(
        try FileManager.default.contentsOfDirectory(atPath: output.path).sorted(),
        ["foreign.json", "input-events.json"])
    }
  }

  func testExclusiveCreationAndAncestorPinNeverOverwriteReplacementDirectory() throws {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    let output = root.appendingPathComponent("recording", isDirectory: true)
    let moved = root.appendingPathComponent("retained", isDirectory: true)
    try FileManager.default.createDirectory(at: output, withIntermediateDirectories: false)
    let journal = try InputEventJournal(output: output, owner: owner, supportsSystem: false)
    XCTAssertThrowsError(try InputEventJournal(output: output, owner: owner, supportsSystem: false))
    try FileManager.default.moveItem(at: output, to: moved)
    try FileManager.default.createDirectory(at: output, withIntermediateDirectories: false)
    let marker = Data("foreign replacement".utf8)
    try marker.write(to: output.appendingPathComponent("input-events.json"))
    var policy = try mic(journal)
    try cycle(&policy)
    try journal.finish()
    XCTAssertEqual(try Data(contentsOf: output.appendingPathComponent("input-events.json")), marker)
    let report = try XCTUnwrap(try read(moved)["report"] as? [String: Any])
    XCTAssertEqual((report["microphoneHistory"] as? [String: Any])?["lastSequence"] as? Int, 2)
    let alias = root.appendingPathComponent("alias", isDirectory: true)
    try FileManager.default.createSymbolicLink(at: alias, withDestinationURL: moved)
    XCTAssertThrowsError(try InputEventJournal(output: alias, owner: owner, supportsSystem: false))
  }

  func testInvalidOrBackwardsHistoryPoisonsRetentionWithoutRenumbering() throws {
    let output = try directory()
    defer { try? FileManager.default.removeItem(at: output) }
    let journal = try InputEventJournal(output: output, owner: owner, supportsSystem: false)
    var policy = try mic(journal)
    let old = policy.status
    try cycle(&policy)
    try journal.flush()
    let committed = try Data(contentsOf: output.appendingPathComponent("input-events.json"))
    journal.observer(.microphone)(old, InputEventHistory())
    XCTAssertEqual(journal.persistence().state, "invalid")
    XCTAssertThrowsError(try journal.finish())
    XCTAssertEqual(
      try Data(contentsOf: output.appendingPathComponent("input-events.json")), committed)
  }

  func testPersistenceFailureDoesNotStopPolicyAndLaterCommitRetainsTheActualTransitions() throws {
    let output = try directory()
    defer { try? FileManager.default.removeItem(at: output) }
    let failure = JournalFailure()
    let journal = try InputEventJournal(
      output: output, owner: owner, supportsSystem: false, beforeWrite: { try failure.write() })
    var policy = try mic(journal)
    XCTAssertThrowsError(try journal.flush())
    XCTAssertEqual(policy.status.state, "connected")
    XCTAssertEqual(journal.persistence().state, "failed")
    XCTAssertLessThan(
      journal.persistence().committedRevision, journal.persistence().requestedRevision)
    let before = try XCTUnwrap(try read(output)["report"] as? [String: Any])
    XCTAssertEqual((before["microphoneHistory"] as? [String: Any])?["lastSequence"] as? Int, 0)
    failure.recover()
    try cycle(&policy)
    try journal.finish()
    let after = try XCTUnwrap(try read(output)["report"] as? [String: Any])
    XCTAssertEqual(journal.persistence().state, "confirmed")
    XCTAssertTrue(journal.persistence().closed)
    XCTAssertEqual((after["microphoneHistory"] as? [String: Any])?["lastSequence"] as? Int, 2)
    XCTAssertEqual(policy.status.state, "connected")  // sealing never changes the audio policy
  }

  func testPersistenceDiagnosticsNeverWaitForIOOrTurnStopTimeoutIntoConfirmation() throws {
    let output = try directory()
    defer { try? FileManager.default.removeItem(at: output) }
    let gate = JournalGate()
    gate.hold()
    let journal = try InputEventJournal(
      output: output, owner: owner, supportsSystem: false, beforeWrite: { gate.write() })
    XCTAssertEqual(journal.persistence().state, "confirmed")
    var policy = try mic(journal)
    XCTAssertEqual(gate.entered.wait(timeout: .now() + 2), .success)
    try cycle(&policy)
    XCTAssertEqual(journal.persistence().state, "pending")
    XCTAssertEqual(journal.persistence().committedRevision, 1)
    XCTAssertFalse(journal.persistence().closed)
    XCTAssertThrowsError(try journal.finish(timeout: 0.01))
    XCTAssertEqual(journal.persistence().state, "pending")
    XCTAssertTrue(journal.persistence().closed)
    XCTAssertEqual(policy.status.state, "connected")
    for _ in 0..<3 { gate.released.signal() }
    try journal.flush()
    let confirmed = journal.persistence()
    XCTAssertEqual(confirmed.state, "confirmed")
    XCTAssertEqual(confirmed.requestedRevision, confirmed.committedRevision)
    XCTAssertTrue(confirmed.closed)
  }

  func testJournalStatusReportIsBoundedAndCannotClaimAudioOrFutureCommit() throws {
    let valid = InputJournalPersistence(
      requestedRevision: 1000, committedRevision: 999, state: "pending", closed: false)
    let value = try XCTUnwrap(
      JSONSerialization.jsonObject(
        with:
          InputJournalStatusReport(ownerId: owner, revision: 2, persistence: valid).encodedData())
        as? [String: Any])
    XCTAssertEqual(Set(value.keys), ["schemaVersion", "ownerId", "revision", "persistence"])
    for bad in [
      InputJournalPersistence(
        requestedRevision: 0, committedRevision: 0, state: "confirmed", closed: false),
      InputJournalPersistence(
        requestedRevision: 2, committedRevision: 3, state: "confirmed", closed: false),
      InputJournalPersistence(
        requestedRevision: 2, committedRevision: 1, state: "confirmed", closed: false),
      InputJournalPersistence(
        requestedRevision: 2, committedRevision: 1, state: "saved", closed: false),
    ] {
      XCTAssertThrowsError(
        try InputJournalStatusReport(ownerId: owner, revision: 2, persistence: bad).encodedData())
    }
    XCTAssertThrowsError(
      try InputJournalStatusReport(ownerId: "foreign", revision: 2, persistence: valid)
        .encodedData())
  }

  func testDiagnosticPublisherPinsAncestorAndNeverUpdatesOrRemovesForeignReplacement() throws {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    let output = root.appendingPathComponent("recording", isDirectory: true)
    let moved = root.appendingPathComponent("retained", isDirectory: true)
    try FileManager.default.createDirectory(at: output, withIntermediateDirectories: false)
    let value = InputJournalPersistence(
      requestedRevision: 2, committedRevision: 1, state: "failed", closed: false)
    let publisher = try InputJournalStatusPublisher(output: output, owner: owner, initial: value)
    XCTAssertThrowsError(
      try InputJournalStatusPublisher(output: output, owner: owner, initial: value))
    try FileManager.default.moveItem(at: output, to: moved)
    try FileManager.default.createDirectory(at: output, withIntermediateDirectories: false)
    let leaf = "input-journal-status.json"
    let marker = Data("foreign diagnostic must survive".utf8)
    try marker.write(to: output.appendingPathComponent(leaf))
    try publisher.publish(value)
    let published = try XCTUnwrap(
      JSONSerialization.jsonObject(
        with: Data(contentsOf: moved.appendingPathComponent(leaf))) as? [String: Any])
    XCTAssertEqual(published["revision"] as? Int, 2)
    publisher.removeIfOwned()
    XCTAssertFalse(FileManager.default.fileExists(atPath: moved.appendingPathComponent(leaf).path))
    XCTAssertEqual(try Data(contentsOf: output.appendingPathComponent(leaf)), marker)
    let another = try InputJournalStatusPublisher(output: moved, owner: owner, initial: value)
    try marker.write(to: moved.appendingPathComponent(leaf))
    XCTAssertThrowsError(try another.publish(value))
    another.removeIfOwned()
    XCTAssertEqual(try Data(contentsOf: moved.appendingPathComponent(leaf)), marker)
  }

  func testRetainedJournalCannotOptOutOfSyncOrUseEphemeralCleanup() throws {
    let output = try directory()
    defer { try? FileManager.default.removeItem(at: output) }
    XCTAssertThrowsError(
      try InputEventJournalFile(output: output, initial: Data("{}".utf8), synchronizes: false))
    let file = try InputEventJournalFile(output: output, initial: Data("{}".utf8))
    XCTAssertFalse(file.removeEphemeralIfOwned())
    XCTAssertTrue(
      FileManager.default.fileExists(
        atPath: output.appendingPathComponent("input-events.json").path))
  }

  func testSIGKILLRetainsCommittedSuffixButDoesNotInventUncommittedTransitionOrSeal() throws {
    let output = try directory()
    defer { try? FileManager.default.removeItem(at: output) }
    let child = Process()
    child.executableURL = URL(fileURLWithPath: "/usr/bin/xcrun")
    child.arguments = [
      "xctest", "-XCTest", "MONMacOSCaptureTests.InputEventJournalTests/testCrashChild",
      Bundle(for: InputEventJournalTests.self).bundleURL.path,
    ]
    child.environment = ProcessInfo.processInfo.environment.merging([
      "MON_INPUT_JOURNAL_CRASH_CHILD": output.path
    ]) { _, child in child }
    child.standardOutput = FileHandle.nullDevice
    child.standardError = FileHandle.nullDevice
    try child.run()
    defer { if child.isRunning { _ = kill(child.processIdentifier, SIGKILL) } }
    let ready = output.appendingPathComponent("ready")
    let deadline = Date().addingTimeInterval(10)
    while !FileManager.default.fileExists(atPath: ready.path), child.isRunning, Date() < deadline {
      Thread.sleep(forTimeInterval: 0.01)
    }
    guard child.isRunning, FileManager.default.fileExists(atPath: ready.path) else {
      XCTFail("Synthetic child did not reach its committed/pending boundary")
      return
    }
    XCTAssertEqual(kill(child.processIdentifier, SIGKILL), 0)
    child.waitUntilExit()
    XCTAssertEqual(child.terminationReason, .uncaughtSignal)
    XCTAssertEqual(child.terminationStatus, SIGKILL)
    let value = try read(output)
    XCTAssertEqual(value["sealed"] as? Bool, false)
    let report = try XCTUnwrap(value["report"] as? [String: Any])
    let history = try XCTUnwrap(report["microphoneHistory"] as? [String: Any])
    XCTAssertEqual(history["lastSequence"] as? Int, 2)
    XCTAssertEqual((history["events"] as? [Any])?.count, 2)
  }

  func testCrashChild() throws {
    guard let path = ProcessInfo.processInfo.environment["MON_INPUT_JOURNAL_CRASH_CHILD"] else {
      throw XCTSkip("Only the SIGKILL parent runs this isolated synthetic child")
    }
    let output = URL(fileURLWithPath: path, isDirectory: true)
    let gate = JournalGate()
    let journal = try InputEventJournal(
      output: output, owner: owner, supportsSystem: false, beforeWrite: { gate.write() })
    var policy = try mic(journal)
    try cycle(&policy)
    try journal.flush()
    gate.hold()
    policy.interrupt("device_disconnected")
    XCTAssertEqual(gate.entered.wait(timeout: .now() + 2), .success)
    try Data("committed 2; pending 3".utf8).write(to: output.appendingPathComponent("ready"))
    // Parent kills only this known synthetic XCTest child, never the product.
    while true { Thread.sleep(forTimeInterval: 0.05) }
  }
}
