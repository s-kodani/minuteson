import Foundation
import XCTest

@testable import MONMacOSCapture

private final class TelemetryInputs: @unchecked Sendable {
  private let lock = NSLock()
  private var system = MicrophoneRecovery.Status(
    state: "connected", sourceEpoch: 1, interruptions: 0, reason: nil)
  private var systemHistory = InputEventHistory()
  func snapshots() -> (TrackStats, TrackStats) {
    lock.withLock {
      var mic = TrackStats()
      var pc = TrackStats()
      mic.microphoneRecovery = .init(
        state: "reconnecting", sourceEpoch: 1, interruptions: 0, reason: nil)
      mic.inputEventHistory = InputEventHistory()
      pc.systemRecovery = system
      pc.inputEventHistory = systemHistory
      return (mic, pc)
    }
  }
  func systemEnded() {
    lock.withLock {
      system = .init(
        state: "disconnected", sourceEpoch: 1, interruptions: 1, reason: "stream_stopped")
      systemHistory.append("interrupted", status: system)
    }
  }
}

final class CaptureTelemetryPublisherTests: XCTestCase {
  private let owner = "735027d4-a03e-43d3-830a-21e767dfaf88"
  private func report(_ directory: URL, leaf: String = "input-status.json") throws -> [String: Any]
  {
    let data = try Data(contentsOf: directory.appendingPathComponent(leaf))
    return try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
  }
  private func awaitRevision(_ revision: Int, in directory: URL) throws -> [String: Any] {
    let deadline = ContinuousClock.now.advanced(by: .seconds(3))
    while ContinuousClock.now < deadline {
      if let value = try? report(directory), (value["revision"] as? Int ?? 0) >= revision {
        return value
      }
      Thread.sleep(forTimeInterval: 0.01)
    }
    XCTFail("telemetry owner did not publish a fresh policy snapshot")
    return try report(directory)
  }

  @MainActor
  func testBluetoothInitializationWaitCannotAgeHealthyPCStatusOrReplayOldPolicy() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(
      "mon-telemetry-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
    defer { try? FileManager.default.removeItem(at: directory) }
    let inputs = TelemetryInputs()
    let publisher = CaptureTelemetryPublisher(
      output: directory, owner: owner, publishesStatus: true, snapshots: inputs.snapshots)
    defer { publisher.stop() }
    publisher.start()
    let first = try awaitRevision(1, in: directory)
    // The real session's MainActor can be held by a Bluetooth format request.
    // Keep it blocked longer than the UI's one-second freshness threshold.
    _ = DispatchSemaphore(value: 0).wait(timeout: .now() + 1.3)
    let fresh = try report(directory)
    XCTAssertGreaterThan(fresh["revision"] as? Int ?? 0, first["revision"] as? Int ?? 0)
    XCTAssertEqual((fresh["microphone"] as? [String: Any])?["state"] as? String, "reconnecting")
    XCTAssertEqual((fresh["system"] as? [String: Any])?["state"] as? String, "connected")
    let attributes = try FileManager.default.attributesOfItem(
      atPath: directory.appendingPathComponent("input-status.json").path)
    XCTAssertLessThan(
      Date().timeIntervalSince(try XCTUnwrap(attributes[.modificationDate] as? Date)), 0.8)
    inputs.systemEnded()
    let changed = try awaitRevision((fresh["revision"] as? Int ?? 0) + 1, in: directory)
    XCTAssertEqual((changed["system"] as? [String: Any])?["reason"] as? String, "stream_stopped")
    XCTAssertEqual(
      (changed["system"] as? [String: Any])?["state"] as? String, "disconnected",
      "never refresh a cached connected report")
    publisher.stop()
    let stopped = try Data(contentsOf: directory.appendingPathComponent("input-status.json"))
    Thread.sleep(forTimeInterval: 0.25)
    XCTAssertEqual(
      try Data(contentsOf: directory.appendingPathComponent("input-status.json")), stopped)
  }
}
