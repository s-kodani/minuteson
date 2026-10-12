import Foundation
import XCTest

@testable import MONMacOSCapture

final class UnreceivedSystemAudioTests: XCTestCase, @unchecked Sendable {
  private func directory() throws -> URL {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(
      at: root, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
    let path = try XCTUnwrap(realpath(root.path, nil))
    defer { free(path) }
    return URL(fileURLWithPath: String(cString: path), isDirectory: true)
  }
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
        try await writer.finish(allowUnreceivedAudio: mode != "ordinary")
        XCTAssertEqual(mode, "test")
        try await writer.finish(allowUnreceivedAudio: true)
      } catch {
        XCTAssertNotEqual(mode, "test")
        let firstFailure = String(describing: error)
        do {
          try await writer.finish(allowUnreceivedAudio: true)
          XCTFail("retry must retain \(mode) failure")
        } catch { XCTAssertEqual(String(describing: error), firstFailure) }
      }
      XCTAssertFalse(FileManager.default.fileExists(atPath: url.path))
      XCTAssertEqual(writer.stats.frames, 0)
    }
  }

  func testCleanUnreceivedRecordingClosesEmptyRawProofWithoutCreatingAudio() async throws {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    let owner = UUID().uuidString.lowercased()
    let raw = try RawRecordingSegments(output: root, owner: owner, lane: "system")
    let url = root.appendingPathComponent("system.caf")
    let writer = SystemPCMWriter(outputURL: url, rawSegments: raw)
    try await writer.finish(allowUnreceivedAudio: true)
    try await writer.finish(allowUnreceivedAudio: true)
    XCTAssertEqual(raw.snapshot().sealedFrames, 0)
    XCTAssertEqual(raw.snapshot().sealedSegments, 0)
    XCTAssertTrue(raw.snapshot().producerClosed)
    XCTAssertNil(raw.snapshot().failure)
    XCTAssertFalse(FileManager.default.fileExists(atPath: url.path))
  }

  func testOrdinaryUnreceivedManifestRequiresOwnedRawAndBothAcquisitionClosures() async throws {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    let owner = UUID().uuidString.lowercased()
    let raw = try RawRecordingSegments(output: root, owner: owner, lane: "system")
    try raw.finish()
    func reading(_ value: Int64) -> CaptureClockReading {
      CaptureClockReading(
        hostBeforeNs: value, continuousNs: value + 10, utcMs: 1_700_000_000_000,
        hostAfterNs: value + 20)
    }
    let box = ReadingBox(reading(1_000_000_000))
    let clock = try CaptureClockDomain(units: MachTimeUnits(numerator: 1, denominator: 1)) {
      box.read()
    }
    var mic = TrackStats(callbackCount: 1, frames: 8000, sampleRate: 8000, channels: 2)
    mic.recordSavedClock(.timestamp(1_000_000_010), frames: 8000, sourceFrame: 0)
    var pc = TrackStats()
    pc.rawSegments = raw.snapshot()
    pc.systemRecovery = .init(state: "stopped", sourceEpoch: 1, interruptions: 0, reason: nil)
    box.set(reading(2_100_000_000))
    let micClosure = try clock.close(.microphone)
    let pcClosure = try clock.close(.system)
    let original = Manifest(
      schemaVersion: 2, scenario: "quiet-ordinary", captureMode: "system", backend: "coreaudio",
      targetPID: nil, targetPIDs: [], startedAtUTC: "synthetic", requestedDurationSeconds: nil,
      osVersion: "synthetic", architecture: "arm64", state: "saved", microphone: mic, system: pc,
      errors: [], recordingOwnerId: owner)
    for problem in ["raw", "owner", "capture", "callback", "format", "write", "state", "closure"] {
      var invalid = original
      switch problem {
      case "raw": invalid.system.rawSegments = nil
      case "owner": invalid.recordingOwnerId = UUID().uuidString.lowercased()
      case "capture": invalid.system.retainCaptureFailure(ProbeError.permission("synthetic denial"))
      case "callback": invalid.system.callbackCount = 1
      case "format": invalid.system.sampleRate = 8000
      case "write": invalid.system.appendFailures = 1
      case "state":
        invalid.system.systemRecovery = .init(
          state: "disconnected", sourceEpoch: 1, interruptions: 1, reason: "lost")
      default: break
      }
      XCTAssertThrowsError(
        try invalid.finalizeStoppedClock(
          clock, microphoneClosure: micClosure,
          systemClosure: problem == "closure" ? nil : pcClosure), problem)
    }
    var saved = original
    try saved.finalizeStoppedClock(clock, microphoneClosure: micClosure, systemClosure: pcClosure)
    XCTAssertEqual(saved.schemaVersion, 3)
    XCTAssertEqual(saved.systemAudioUnreceived, true)
    XCTAssertEqual(saved.system.clockSegments?.count, 0)
    XCTAssertEqual(saved.microphone.clockSegments?.last?.timelineEndUs, 1_000_000)
    XCTAssertEqual(saved.captureEnd?.endUs, 1_100_000)
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
    let starting = window.metadata()
    var mic = TrackStats(callbackCount: 1, frames: 8000, sampleRate: 8000, channels: 2)
    mic.recordSavedClock(.timestamp(1_000_000_000), frames: 8000, sourceFrame: 0)
    let original = Manifest(
      schemaVersion: 2, scenario: "synthetic-unreceived-test", captureMode: "system",
      backend: "coreaudio", targetPID: nil, targetPIDs: [], startedAtUTC: "synthetic",
      requestedDurationSeconds: 5, osVersion: "synthetic", architecture: "arm64", state: "saved",
      microphone: mic, system: TrackStats(), errors: [], captureTest: starting)
    XCTAssertFalse(original.captureTest!.completed)
    window.finish(at: 6_000_000_000)
    var saved = original
    try saved.finalizeTestClock(clock, window: window)
    XCTAssertEqual(saved.captureTest?.completed, true)
    XCTAssertEqual(saved.systemAudioUnreceived, true)
    XCTAssertEqual(saved.system.clockSegments?.count, 0)
    XCTAssertEqual(saved.microphone.clockSegments?.first?.timelineStartUs, 0)
    XCTAssertNil(saved.system.firstCaptureNanoseconds)
    var received = original
    var pc = TrackStats(callbackCount: 1, frames: 8000, sampleRate: 8000, channels: 2)
    pc.recordSavedClock(.timestamp(1_100_000_000), frames: 8000, sourceFrame: 0)
    received.system = pc
    try received.finalizeTestClock(clock, window: window)
    XCTAssertEqual(received.captureTest?.completed, true)
    XCTAssertNil(received.systemAudioUnreceived)
    XCTAssertEqual(received.system.clockSegments?.first?.timelineStartUs, 100_000)
    XCTAssertEqual(received.microphone.clockSegments?.first?.timelineStartUs, 0)
    for kind in [
      "no-test", "wrong-window", "wrong-owner", "format", "write", "capture", "callback", "no-mic",
    ] {
      var changed = original
      switch kind {
      case "no-test": changed.captureTest = nil
      case "wrong-window":
        changed.captureTest = CaptureTestMetadata(
          ownerId: window.ownerId, windowStartNs: 1, windowEndNs: 5_000_000_001,
          firstSavedCaptureNs: nil, completed: false)
      case "wrong-owner":
        changed.captureTest = CaptureTestMetadata(
          ownerId: "635027d4-a03e-43d3-830a-21e767dfaf88", windowStartNs: 1_000_000_000,
          windowEndNs: 6_000_000_000, firstSavedCaptureNs: nil, completed: true)
      case "format": changed.system.sampleRate = 8000
      case "write": changed.system.appendFailures = 1
      case "capture": changed.system.retainCaptureFailure(ProbeError.capture("lost acquisition"))
      case "callback": changed.system.callbackCount = 1
      default: changed.microphone = TrackStats()
      }
      XCTAssertThrowsError(try changed.finalizeTestClock(clock, window: window), kind)
    }
    var ordinary = original
    XCTAssertThrowsError(try ordinary.finalizeClock(clock))

    let unfinished = try CaptureTestWindow(ownerId: window.ownerId)
    try unfinished.open(at: 1_000_000_000)
    for earlyStop in [false, true] {
      if earlyStop { unfinished.finish(at: 5_999_999_999) }
      var changed = original
      XCTAssertThrowsError(try changed.finalizeTestClock(clock, window: unfinished))
    }
  }
}
