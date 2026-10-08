import CoreAudio
import Foundation
import XCTest

@testable import MONMacOSCapture

final class InputEventHistoryTests: XCTestCase {
  private let owner = "735027d4-a03e-43d3-830a-21e767dfaf88"
  private let device = CaptureInputDevice(
    uid: "confirmed-usb", name: "USB", isDefault: false, objectID: 2)

  private func microphone() throws -> MicrophoneRecovery {
    var mic = try MicrophoneRecovery(uid: device.uid)
    let token = try mic.begin(device)
    try mic.activate(token)
    return mic
  }

  private func system() throws -> SystemAudioRecovery {
    var sys = try SystemAudioRecovery(targetPID: nil, applicationIdentity: nil, retries: true)
    let token = try sys.begin(at: 1)
    try sys.activate(token)
    sys.operationFinished(token)
    return sys
  }

  func testBothRapidTransitionsSurviveOneHeartbeatAndExportForRust() throws {
    var mic = try microphone()
    var sys = try system()
    XCTAssertTrue(mic.history.events.isEmpty)
    XCTAssertTrue(sys.history.events.isEmpty)
    // Neither mutation is sampled by a heartbeat or UI poll. The recovery
    // policies themselves must retain the real interruption and return.
    mic.interrupt("device_disconnected")
    sys.interrupt("stream_stopped")
    let micReturned = try mic.begin(device)
    try mic.activate(micReturned)
    let sysReturned = try sys.begin(at: 1_000_000_001)
    try sys.activate(sysReturned)
    sys.operationFinished(sysReturned)
    let report = InputStatusReport(
      ownerId: owner, revision: 7, microphone: mic.status, system: sys.status,
      microphoneHistory: mic.history, systemHistory: sys.history)
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
    defer { try? FileManager.default.removeItem(at: directory) }
    try report.publish(to: directory)
    let json = try XCTUnwrap(
      JSONSerialization.jsonObject(
        with: Data(contentsOf: directory.appendingPathComponent("input-status.json")))
        as? [String: Any])
    XCTAssertEqual(json["schemaVersion"] as? Int, 3)
    XCTAssertEqual(
      Set(json.keys),
      [
        "schemaVersion", "ownerId", "revision", "microphone", "system", "microphoneHistory",
        "systemHistory",
      ])
    for history in [mic.history, sys.history] {
      XCTAssertEqual(history.lastSequence, 2)
      XCTAssertEqual(history.droppedThrough, 0)
      XCTAssertEqual(history.events.map(\.sequence), [1, 2])
      XCTAssertEqual(history.events.map(\.kind), ["interrupted", "recovered"])
      XCTAssertEqual(history.events.map { $0.status.sourceEpoch }, [1, 2])
      XCTAssertEqual(history.events.map { $0.status.interruptions }, [1, 1])
    }
    XCTAssertEqual(mic.history.events[0].status.reason, "device_disconnected")
    XCTAssertEqual(sys.history.events[0].status.reason, "stream_stopped")
    XCTAssertTrue(mic.history.matches(mic.status))
    XCTAssertTrue(sys.history.matches(sys.status, input: "system"))
    if let export = ProcessInfo.processInfo.environment["MON_INPUT_HISTORY_FIXTURE_DIR"] {
      let output = URL(fileURLWithPath: export, isDirectory: true)
      XCTAssertTrue(try FileManager.default.contentsOfDirectory(atPath: output.path).isEmpty)
      try report.publish(to: output)
    }
  }

  func testRepeatedFailurePendingRetryAndObsoleteResultsDoNotCreateTransitions() throws {
    var mic = try microphone()
    mic.interrupt("device_disconnected")
    let first = mic.history
    let retry = try mic.begin(device)
    for _ in 0..<100 { mic.interrupt("restart_failed", unavailable: true) }
    XCTAssertEqual(mic.history, first)
    XCTAssertTrue(mic.history.matches(mic.status))
    XCTAssertThrowsError(try mic.activate(retry))
    let current = try mic.begin(device)
    try mic.activate(current)
    XCTAssertEqual(mic.history.events.count, 2)
    XCTAssertEqual(mic.history.events[1].status.sourceEpoch, 3)
    XCTAssertThrowsError(try mic.activate(current))
    let beforeStop = mic.history
    mic.stop()
    mic.interrupt("engine_changed")
    XCTAssertEqual(mic.history, beforeStop)
    XCTAssertTrue(mic.history.matches(mic.status))

    var sys = try system()
    sys.interrupt("stream_stopped")
    let pending = try sys.begin(at: 1_000_000_001)
    XCTAssertTrue(sys.expire(at: 6_000_000_001))
    XCTAssertTrue(sys.expire(at: 7_000_000_001))
    XCTAssertEqual(sys.history.events.count, 1)
    XCTAssertEqual(sys.history.events[0].status.reason, "stream_stopped")
    XCTAssertThrowsError(try sys.activate(pending))
    sys.operationFinished(pending)
    let returned = try sys.begin(at: 8_000_000_001)
    try sys.activate(returned)
    sys.operationFinished(returned)
    XCTAssertEqual(sys.history.events.map(\.kind), ["interrupted", "recovered"])
    XCTAssertTrue(sys.history.matches(sys.status, input: "system"))
    sys.interrupt("format_changed", unavailable: true)
    let blocked = sys.history
    sys.interrupt("stream_stopped")
    sys.stop()
    XCTAssertEqual(sys.history, blocked)
    XCTAssertTrue(sys.history.matches(sys.status, input: "system"))
  }

  func testBoundedSuffixExplicitlyReportsEvictionForEachInput() throws {
    var mic = try microphone()
    var sys = try system()
    for index in 1...100 {
      mic.interrupt("engine_changed")
      let token = try mic.begin(device)
      try mic.activate(token)
      sys.interrupt("stream_stopped")
      let sysToken = try sys.begin(at: Int64(index) * 1_000_000_000 + 1)
      try sys.activate(sysToken)
      sys.operationFinished(sysToken)
      for (history, status, input) in [
        (mic.history, mic.status, "microphone"), (sys.history, sys.status, "system"),
      ] {
        XCTAssertTrue(history.matches(status, input: input))
        XCTAssertEqual(history.events.count, min(index * 2, InputEventHistory.capacity))
        XCTAssertEqual(history.lastSequence, UInt64(index * 2))
        XCTAssertEqual(
          history.droppedThrough, UInt64(max(0, index * 2 - InputEventHistory.capacity)))
        XCTAssertEqual(history.events.first?.sequence, history.droppedThrough + 1)
      }
    }
    let data = try JSONEncoder().encode(
      InputStatusReport(
        ownerId: owner, revision: 200, microphone: mic.status, system: sys.status,
        microphoneHistory: mic.history, systemHistory: sys.history))
    XCTAssertLessThan(data.count, 64 * 1024)
  }

  func testHistoryAndCurrentStateMustMatchBeforeAtomicPublication() throws {
    var mic = try microphone()
    let initial = mic.status
    mic.interrupt("device_disconnected")
    let token = try mic.begin(device)
    try mic.activate(token)
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
    defer { try? FileManager.default.removeItem(at: directory) }
    let valid = InputStatusReport(
      ownerId: owner, revision: 1, microphone: mic.status, microphoneHistory: mic.history)
    try valid.publish(to: directory)
    let path = directory.appendingPathComponent("input-status.json")
    let before = try Data(contentsOf: path)
    for invalid in [
      InputStatusReport(
        ownerId: owner, revision: 2, microphone: initial, microphoneHistory: mic.history),
      InputStatusReport(
        ownerId: owner, revision: 2, microphone: mic.status, microphoneHistory: InputEventHistory()),
      InputStatusReport(
        ownerId: owner, revision: 2, microphone: mic.status, system: initial,
        microphoneHistory: mic.history),
      InputStatusReport(
        ownerId: owner, revision: 2, microphone: mic.status, systemHistory: mic.history),
    ] {
      XCTAssertThrowsError(try invalid.publish(to: directory))
      XCTAssertEqual(try Data(contentsOf: path), before)
    }
    var stats = TrackStats()
    stats.microphoneRecovery = mic.status
    stats.inputEventHistory = mic.history
    let encoded = try XCTUnwrap(
      JSONSerialization.jsonObject(with: JSONEncoder().encode(stats)) as? [String: Any])
    XCTAssertNil(encoded["inputEventHistory"])
    XCTAssertNil(encoded["microphoneHistory"])
    let report = try XCTUnwrap(
      JSONSerialization.jsonObject(with: before) as? [String: Any])
    XCTAssertTrue(report["system"] is NSNull)
    XCTAssertTrue(report["systemHistory"] is NSNull)
  }
}
