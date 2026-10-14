import CoreAudio
import Foundation
import XCTest

@testable import MONMacOSCapture

final class InputStatusTests: XCTestCase {
  private let owner = "735027d4-a03e-43d3-830a-21e767dfaf88"
  private let device = CaptureInputDevice(
    uid: "confirmed-usb", name: "USB", isDefault: false, objectID: 2)

  func testOnlyOwnedOrdinaryRecordingPublishesBindingStatus() throws {
    let common = [
      "--output", "/private/tmp/mon-status-options-only", "--stop-on-file", "--parent-pid", "1234",
      "--microphone-uid", "confirmed-usb",
    ]
    let ordinary = try Options.parse(common + ["--until-stopped", "--input-level-owner", owner])
    XCTAssertTrue(ordinary.publishesInputStatus)
    XCTAssertFalse(try Options.parse(common + ["--until-stopped"]).publishesInputStatus)
    XCTAssertFalse(
      try Options.parse(common + ["--duration", "30", "--input-level-owner", owner])
        .publishesInputStatus)
    XCTAssertFalse(try Options.parse(common + ["--five-second-test", owner]).publishesInputStatus)
  }

  func testSwiftBindingHeartbeatPublishesActualPolicyAndExportsForNativeReader() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
    defer { try? FileManager.default.removeItem(at: directory) }
    var recovery = try MicrophoneRecovery(uid: device.uid)
    var system = try SystemAudioRecovery(targetPID: nil, applicationIdentity: nil, retries: true)
    let systemFirst = try system.begin(at: 1)
    try system.activate(systemFirst)
    system.operationFinished(systemFirst)
    let first = try recovery.begin(device)
    try recovery.activate(first)
    func publish(_ revision: Int64) throws -> [String: Any] {
      try InputStatusReport(
        ownerId: owner, revision: revision, microphone: recovery.status, system: system.status,
        microphoneHistory: recovery.history, systemHistory: system.history
      )
      .publish(to: directory)
      return try XCTUnwrap(
        JSONSerialization.jsonObject(
          with: Data(contentsOf: directory.appendingPathComponent("input-status.json")))
          as? [String: Any])
    }
    let initial = try publish(1)
    XCTAssertEqual(
      Set(initial.keys),
      [
        "schemaVersion", "ownerId", "revision", "microphone", "system", "microphoneHistory",
        "systemHistory",
      ])
    XCTAssertEqual(initial["schemaVersion"] as? Int, 3)
    let connected = try XCTUnwrap(initial["microphone"] as? [String: Any])
    XCTAssertEqual(Set(connected.keys), ["state", "sourceEpoch", "interruptions", "reason"])
    XCTAssertTrue(connected["reason"] is NSNull)
    XCTAssertEqual(connected["state"] as? String, "connected")
    let initialSystem = try XCTUnwrap(initial["system"] as? [String: Any])
    XCTAssertEqual(Set(initialSystem.keys), ["state", "sourceEpoch", "interruptions", "reason"])
    XCTAssertEqual(initialSystem["state"] as? String, "connected")
    XCTAssertTrue(initialSystem["reason"] is NSNull)
    recovery.interrupt("device_changed")
    system.interrupt("stream_stopped")
    let interrupted = try XCTUnwrap(publish(2)["microphone"] as? [String: Any])
    XCTAssertEqual(interrupted["reason"] as? String, "device_changed")
    XCTAssertEqual(interrupted["interruptions"] as? Int, 1)
    let interruptedSystem = try XCTUnwrap(publish(2)["system"] as? [String: Any])
    XCTAssertEqual(interruptedSystem["state"] as? String, "disconnected")
    XCTAssertEqual(interruptedSystem["reason"] as? String, "stream_stopped")
    XCTAssertEqual(interruptedSystem["interruptions"] as? Int, 1)
    let returned = try recovery.begin(device)
    try recovery.activate(returned)
    let systemReturned = try system.begin(at: 1_000_000_001)
    try system.activate(systemReturned)
    system.operationFinished(systemReturned)
    let final = try publish(3)
    let restored = try XCTUnwrap(final["microphone"] as? [String: Any])
    XCTAssertEqual(restored["sourceEpoch"] as? Int, 2)
    XCTAssertTrue(restored["reason"] is NSNull)
    let restoredSystem = try XCTUnwrap(final["system"] as? [String: Any])
    XCTAssertEqual(restoredSystem["state"] as? String, "connected")
    XCTAssertEqual(restoredSystem["sourceEpoch"] as? Int, 2)
    XCTAssertEqual(restoredSystem["interruptions"] as? Int, 1)
    XCTAssertTrue(restoredSystem["reason"] is NSNull)
    XCTAssertEqual(try publish(4)["revision"] as? Int, 4)
    if let export = ProcessInfo.processInfo.environment["MON_INPUT_STATUS_FIXTURE_DIR"] {
      let output = URL(fileURLWithPath: export, isDirectory: true)
      XCTAssertTrue(try FileManager.default.contentsOfDirectory(atPath: output.path).isEmpty)
      try InputStatusReport(
        ownerId: owner, revision: 4, microphone: recovery.status, system: system.status,
        microphoneHistory: recovery.history, systemHistory: system.history
      ).publish(
        to: output)
    }
    XCTAssertEqual(
      try FileManager.default.contentsOfDirectory(atPath: directory.path), ["input-status.json"])
  }

  func testInvalidOwnerAndUnsafeCountersCannotOverwritePublishedStatus() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
    defer { try? FileManager.default.removeItem(at: directory) }
    var recovery = try MicrophoneRecovery(uid: device.uid)
    let first = try recovery.begin(device)
    try recovery.activate(first)
    let report = InputStatusReport(
      ownerId: owner, revision: 1, microphone: recovery.status,
      microphoneHistory: recovery.history)
    try report.publish(to: directory)
    let unsupported =
      try JSONSerialization.jsonObject(
        with: Data(contentsOf: directory.appendingPathComponent("input-status.json")))
      as! [String: Any]
    XCTAssertTrue(unsupported["system"] is NSNull)
    let path = directory.appendingPathComponent("input-status.json")
    let before = try Data(contentsOf: path)
    for invalid in ["../outside", owner.uppercased(), "735027d4-a03e-13d3-830a-21e767dfaf88"] {
      XCTAssertThrowsError(
        try InputStatusReport(
          ownerId: invalid, revision: 2, microphone: recovery.status,
          microphoneHistory: recovery.history
        ).publish(to: directory))
    }
    for revision in [Int64(0), -1, 9_007_199_254_740_992] {
      XCTAssertThrowsError(
        try InputStatusReport(
          ownerId: owner, revision: revision, microphone: recovery.status,
          microphoneHistory: recovery.history
        )
        .publish(to: directory))
    }
    for status in [
      MicrophoneRecovery.Status(
        state: "connected", sourceEpoch: 9_007_199_254_740_992, interruptions: 0, reason: nil),
      MicrophoneRecovery.Status(
        state: "connected", sourceEpoch: 1, interruptions: 9_007_199_254_740_992, reason: nil),
    ] {
      XCTAssertThrowsError(
        try InputStatusReport(
          ownerId: owner, revision: 2, microphone: status,
          microphoneHistory: recovery.history
        ).publish(to: directory))
    }
    XCTAssertEqual(try Data(contentsOf: path), before)
    let unsafe = SystemAudioRecovery.Status(
      state: "connected", sourceEpoch: 9_007_199_254_740_992, interruptions: 0, reason: nil)
    XCTAssertThrowsError(
      try InputStatusReport(
        ownerId: owner, revision: 2, microphone: recovery.status, system: unsafe,
        microphoneHistory: recovery.history, systemHistory: InputEventHistory()
      ).publish(to: directory))
    XCTAssertEqual(try Data(contentsOf: path), before)
  }

  func testSystemInterruptionAndStopPublishWithoutChangingMicrophoneBinding() throws {
    var microphone = try MicrophoneRecovery(uid: device.uid)
    let mic = try microphone.begin(device)
    try microphone.activate(mic)
    var system = try SystemAudioRecovery(targetPID: nil, applicationIdentity: nil, retries: true)
    let first = try system.begin(at: 1)
    try system.activate(first)
    system.operationFinished(first)
    system.interrupt("application_unavailable")
    func encoded(_ revision: Int64) throws -> [String: Any] {
      try XCTUnwrap(
        JSONSerialization.jsonObject(
          with: JSONEncoder().encode(
            InputStatusReport(
              ownerId: owner, revision: revision, microphone: microphone.status,
              system: system.status, microphoneHistory: microphone.history,
              systemHistory: system.history
            ))) as? [String: Any])
    }
    let interrupted = try encoded(1)
    let actualMic = try XCTUnwrap(interrupted["microphone"] as? [String: Any])
    let actualSystem = try XCTUnwrap(interrupted["system"] as? [String: Any])
    XCTAssertEqual(actualMic["state"] as? String, "connected")
    XCTAssertEqual(actualMic["interruptions"] as? Int, 0)
    XCTAssertEqual(actualSystem["reason"] as? String, "application_unavailable")
    XCTAssertEqual(actualSystem["interruptions"] as? Int, 1)
    system.stop()
    let stopped = try encoded(2)
    XCTAssertEqual((stopped["system"] as? [String: Any])?["state"] as? String, "stopped")
    XCTAssertEqual((stopped["microphone"] as? [String: Any])?["state"] as? String, "connected")
    XCTAssertEqual(
      Set(stopped.keys),
      [
        "schemaVersion", "ownerId", "revision", "microphone", "system", "microphoneHistory",
        "systemHistory",
      ])
  }
}
