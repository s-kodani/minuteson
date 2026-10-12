import Foundation
import XCTest

@testable import MONMacOSCapture

final class UnreceivedSystemAudioTests: XCTestCase, @unchecked Sendable {
  func testWriterPermitsOnlyExplicitCleanUnreceivedTestAndRetainsFailures() async throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false)
    defer { try? FileManager.default.removeItem(at: root) }
    for mode in ["test", "ordinary", "failed-write", "failed-capture"] {
      let url = root.appendingPathComponent("\(mode).caf")
      let writer = SystemPCMWriter(outputURL: url)
      if mode == "failed-write" {
        writer.recordFailure(ProbeError.capture("synthetic write failure"))
      }
      if mode == "failed-capture" {
        writer.retainFailure(ProbeError.permission("synthetic denial"))
      }
      do {
        try await writer.finish(allowUnreceivedTestAudio: mode != "ordinary")
        XCTAssertEqual(mode, "test")
        try await writer.finish(allowUnreceivedTestAudio: true)
      } catch {
        XCTAssertNotEqual(mode, "test")
        let firstFailure = String(describing: error)
        do {
          try await writer.finish(allowUnreceivedTestAudio: true)
          XCTFail("retry must retain \(mode) failure")
        } catch { XCTAssertEqual(String(describing: error), firstFailure) }
      }
      XCTAssertFalse(FileManager.default.fileExists(atPath: url.path))
      XCTAssertEqual(writer.stats.frames, 0)
    }
  }

  func testTestClockUsesSavedMicrophoneOriginAndRejectsEmptyPCContradictions() throws {
    let reading = CaptureClockReading(
      hostBeforeNs: 10_000_000_000, continuousNs: 10_000_000_010,
      utcMs: 1_700_000_000_000, hostAfterNs: 10_000_000_020)
    let clock = try CaptureClockDomain(units: MachTimeUnits(numerator: 1, denominator: 1)) {
      reading
    }
    let window = try CaptureTestWindow(ownerId: "735027d4-a03e-43d3-830a-21e767dfaf88")
    try window.open(at: 1_000_000_000)
    window.finish(at: 6_000_000_000)
    var mic = TrackStats(callbackCount: 1, frames: 8000, sampleRate: 8000, channels: 2)
    mic.recordSavedClock(.timestamp(1_000_000_000), frames: 8000, sourceFrame: 0)
    let original = Manifest(
      schemaVersion: 2, scenario: "synthetic-unreceived-test", captureMode: "system",
      backend: "coreaudio", targetPID: nil, targetPIDs: [], startedAtUTC: "synthetic",
      requestedDurationSeconds: 5, osVersion: "synthetic", architecture: "arm64", state: "saved",
      microphone: mic, system: TrackStats(), errors: [], captureTest: window.metadata())
    var saved = original
    try saved.finalizeTestClock(clock)
    XCTAssertEqual(saved.systemAudioUnreceived, true)
    XCTAssertEqual(saved.system.clockSegments?.count, 0)
    XCTAssertEqual(saved.microphone.clockSegments?.first?.timelineStartUs, 0)
    XCTAssertNil(saved.system.firstCaptureNanoseconds)
    for kind in ["no-test", "not-completed", "format", "write", "capture", "callback", "no-mic"] {
      var changed = original
      switch kind {
      case "no-test": changed.captureTest = nil
      case "not-completed":
        changed.captureTest = CaptureTestMetadata(
          ownerId: window.ownerId, windowStartNs: 1, windowEndNs: 5_000_000_001,
          firstSavedCaptureNs: nil, completed: false)
      case "format": changed.system.sampleRate = 8000
      case "write": changed.system.appendFailures = 1
      case "capture": changed.system.retainCaptureFailure(ProbeError.capture("lost acquisition"))
      case "callback": changed.system.callbackCount = 1
      default: changed.microphone = TrackStats()
      }
      XCTAssertThrowsError(try changed.finalizeTestClock(clock), kind)
    }
    var ordinary = original
    XCTAssertThrowsError(try ordinary.finalizeClock(clock))
  }
}
