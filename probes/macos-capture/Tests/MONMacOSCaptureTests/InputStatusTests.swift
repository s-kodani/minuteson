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
    let first = try recovery.begin(device)
    try recovery.activate(first)
    func publish(_ revision: Int64) throws -> [String: Any] {
      try InputStatusReport(ownerId: owner, revision: revision, microphone: recovery.status)
        .publish(to: directory)
      return try XCTUnwrap(
        JSONSerialization.jsonObject(
          with: Data(contentsOf: directory.appendingPathComponent("input-status.json")))
          as? [String: Any])
    }
    let initial = try publish(1)
    XCTAssertEqual(Set(initial.keys), ["schemaVersion", "ownerId", "revision", "microphone"])
    let connected = try XCTUnwrap(initial["microphone"] as? [String: Any])
    XCTAssertEqual(Set(connected.keys), ["state", "sourceEpoch", "interruptions", "reason"])
    XCTAssertTrue(connected["reason"] is NSNull)
    XCTAssertEqual(connected["state"] as? String, "connected")
    recovery.interrupt("device_disconnected")
    let interrupted = try XCTUnwrap(publish(2)["microphone"] as? [String: Any])
    XCTAssertEqual(interrupted["reason"] as? String, "device_disconnected")
    XCTAssertEqual(interrupted["interruptions"] as? Int, 1)
    let returned = try recovery.begin(device)
    try recovery.activate(returned)
    let final = try publish(3)
    let restored = try XCTUnwrap(final["microphone"] as? [String: Any])
    XCTAssertEqual(restored["sourceEpoch"] as? Int, 2)
    XCTAssertTrue(restored["reason"] is NSNull)
    XCTAssertEqual(try publish(4)["revision"] as? Int, 4)
    if let export = ProcessInfo.processInfo.environment["MON_INPUT_STATUS_FIXTURE_DIR"] {
      let output = URL(fileURLWithPath: export, isDirectory: true)
      XCTAssertTrue(try FileManager.default.contentsOfDirectory(atPath: output.path).isEmpty)
      try InputStatusReport(ownerId: owner, revision: 4, microphone: recovery.status).publish(
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
    let report = InputStatusReport(ownerId: owner, revision: 1, microphone: recovery.status)
    try report.publish(to: directory)
    let path = directory.appendingPathComponent("input-status.json")
    let before = try Data(contentsOf: path)
    for invalid in ["../outside", owner.uppercased(), "735027d4-a03e-13d3-830a-21e767dfaf88"] {
      XCTAssertThrowsError(
        try InputStatusReport(ownerId: invalid, revision: 2, microphone: recovery.status).publish(
          to: directory))
    }
    for revision in [Int64(0), -1, 9_007_199_254_740_992] {
      XCTAssertThrowsError(
        try InputStatusReport(ownerId: owner, revision: revision, microphone: recovery.status)
          .publish(to: directory))
    }
    for status in [
      MicrophoneRecovery.Status(
        state: "connected", sourceEpoch: 9_007_199_254_740_992, interruptions: 0, reason: nil),
      MicrophoneRecovery.Status(
        state: "connected", sourceEpoch: 1, interruptions: 9_007_199_254_740_992, reason: nil),
    ] {
      XCTAssertThrowsError(
        try InputStatusReport(ownerId: owner, revision: 2, microphone: status).publish(
          to: directory))
    }
    XCTAssertEqual(try Data(contentsOf: path), before)
  }
}
