import AVFoundation
import CoreAudio
import Darwin
import Foundation
import XCTest

@testable import MONMacOSCapture

final class MicrophoneRecoveryTests: XCTestCase {
  private let first = CaptureInputDevice(
    uid: "confirmed-usb", name: "同じ名前", isDefault: false, objectID: 2)
  private let other = CaptureInputDevice(
    uid: "other-default", name: "同じ名前", isDefault: true, objectID: 1)

  func testRecoverySelectsOnlyFrozenUIDAndAcceptsItsNewObjectID() throws {
    var recovery = try MicrophoneRecovery(uid: first.uid)
    XCTAssertEqual(try recovery.device(from: [other, first]), first)
    XCTAssertThrowsError(try recovery.device(from: [other]))
    XCTAssertThrowsError(try recovery.device(from: [first, first]))
    XCTAssertThrowsError(try recovery.begin(other))
    let original = try recovery.begin(first)
    try recovery.activate(original)
    recovery.interrupt("device_disconnected")
    let returned = CaptureInputDevice(
      uid: first.uid, name: "changed display name", isDefault: false, objectID: 99)
    let current = try recovery.begin(recovery.device(from: [other, returned]))
    try recovery.activate(current)
    XCTAssertEqual(current.uid, original.uid)
    XCTAssertEqual(current.objectID, 99)
    XCTAssertGreaterThan(current.epoch, original.epoch)
    XCTAssertTrue(recovery.accepts(current))
    XCTAssertFalse(recovery.accepts(original))
  }

  func testLateCallbacksAndRestartResultsCannotReactivateAnOldBinding() throws {
    var recovery = try MicrophoneRecovery(uid: first.uid)
    let firstToken = try recovery.begin(first)
    XCTAssertTrue(recovery.owns(firstToken))
    XCTAssertFalse(recovery.accepts(firstToken))
    XCTAssertThrowsError(try recovery.begin(first))
    try recovery.activate(firstToken)
    XCTAssertThrowsError(try recovery.begin(first))
    recovery.interrupt("engine_changed")
    XCTAssertFalse(recovery.owns(firstToken))
    let current = try recovery.begin(first)
    XCTAssertThrowsError(try recovery.activate(firstToken))
    XCTAssertFalse(recovery.accepts(current))
    try recovery.activate(current)
    XCTAssertThrowsError(try recovery.activate(current))
    XCTAssertFalse(recovery.accepts(firstToken))
    XCTAssertTrue(recovery.accepts(current))
  }

  func testRepeatedFailuresAreOneInterruptionAndStopPreventsEveryRetry() throws {
    var recovery = try MicrophoneRecovery(uid: first.uid)
    let token = try recovery.begin(first)
    try recovery.activate(token)
    for _ in 0..<100 {
      recovery.interrupt("device_disconnected")
      recovery.interrupt("restart_failed", unavailable: true)
    }
    XCTAssertEqual(recovery.status.interruptions, 1)
    let second = try recovery.begin(first)
    try recovery.activate(second)
    XCTAssertNil(recovery.status.reason)
    recovery.interrupt("engine_changed")
    XCTAssertEqual(recovery.status.interruptions, 2)
    recovery.stop()
    let stopped = recovery.status
    recovery.interrupt("device_disconnected")
    XCTAssertEqual(recovery.status, stopped)
    XCTAssertFalse(recovery.accepts(second))
    XCTAssertThrowsError(try recovery.device(from: [first]))
    XCTAssertThrowsError(try recovery.begin(first))
    XCTAssertThrowsError(try recovery.activate(second))
  }

  func testRetryRetainsFormatAndClockDiagnosisUntilAProvenNewBindingActivates() throws {
    for reason in ["format_changed", "clock_changed"] {
      var recovery = try MicrophoneRecovery(uid: first.uid)
      let old = try recovery.begin(first)
      try recovery.activate(old)
      recovery.interrupt(reason, unavailable: reason == "format_changed")
      let interrupted = recovery.status
      for _ in 0..<100 {
        recovery.interruptForRetry()
        XCTAssertEqual(recovery.status, interrupted)
      }
      XCTAssertFalse(recovery.owns(old))
      XCTAssertFalse(recovery.accepts(old))
      XCTAssertEqual(recovery.history.events.count, 1)
      let unproven = try recovery.begin(first)
      XCTAssertEqual(recovery.status.reason, reason)
      XCTAssertFalse(recovery.accepts(unproven))
      recovery.interruptForRetry()
      XCTAssertEqual(recovery.status.reason, reason)
      XCTAssertEqual(recovery.status.state, interrupted.state)
      XCTAssertEqual(recovery.status.interruptions, 1)
      XCTAssertEqual(recovery.history.events.count, 1)
      XCTAssertThrowsError(try recovery.activate(unproven))
      let current = try recovery.begin(first)
      XCTAssertEqual(recovery.status.reason, reason)
      XCTAssertFalse(recovery.accepts(current))
      XCTAssertThrowsError(try recovery.activate(old))
      try recovery.activate(current)
      XCTAssertNil(recovery.status.reason)
      XCTAssertTrue(recovery.accepts(current))
      XCTAssertFalse(recovery.accepts(old))
      recovery.stop()
      let stopped = recovery.status
      recovery.interruptForRetry()
      XCTAssertEqual(recovery.status, stopped)
    }
  }

  func testRetryStillRetiresAnUnhealthyEngineAndAllowsFreshEvidenceToChangeDiagnosis() throws {
    var recovery = try MicrophoneRecovery(uid: first.uid)
    let old = try recovery.begin(first)
    try recovery.activate(old)
    recovery.interruptForRetry()
    XCTAssertEqual(recovery.status.reason, "engine_changed")
    XCTAssertEqual(recovery.status.state, "disconnected")
    XCTAssertEqual(recovery.status.interruptions, 1)
    XCTAssertFalse(recovery.accepts(old))
    recovery.interrupt("format_changed", unavailable: true)
    recovery.interruptForRetry()
    XCTAssertEqual(recovery.status.reason, "format_changed")
    recovery.interrupt("permission_unavailable", unavailable: true)
    XCTAssertEqual(recovery.status.reason, "permission_unavailable")
    XCTAssertEqual(recovery.status.interruptions, 1)
  }

  func testInvalidUIDAndUnknownObjectNeverCreateBindingAndObjectIDsAreNotExported() throws {
    for uid in ["", "a\0b", String(repeating: "x", count: 1025)] {
      XCTAssertThrowsError(try MicrophoneRecovery(uid: uid))
    }
    var recovery = try MicrophoneRecovery(uid: first.uid)
    let unknown = CaptureInputDevice(
      uid: first.uid, name: first.name, isDefault: false, objectID: kAudioObjectUnknown)
    XCTAssertThrowsError(try recovery.device(from: [unknown]))
    XCTAssertThrowsError(try recovery.begin(unknown))
    XCTAssertEqual(recovery.status.sourceEpoch, 0)
    let token = try recovery.begin(first)
    try recovery.activate(token)
    let json =
      try JSONSerialization.jsonObject(with: JSONEncoder().encode(recovery.status))
      as! [String: Any]
    XCTAssertEqual(Set(json.keys), ["state", "sourceEpoch", "interruptions", "reason"])
    XCTAssertTrue(json["reason"] is NSNull)
    XCTAssertEqual(json["state"] as? String, "connected")
  }

  func testResetSampleCounterRequiresANewEpochAndPreservesActualMissingTime() throws {
    var timing = try CaptureTrackTiming(sampleRate: 48000)
    try timing.recordSaved(
      frames: 480, observation: .timestamp(1_000_000_000), sourceFrame: 9000, sourceEpoch: 1)
    XCTAssertThrowsError(
      try timing.recordSaved(
        frames: 480, observation: .timestamp(1_030_000_000), sourceFrame: 0, sourceEpoch: 1))
    try timing.recordSaved(
      frames: 480, observation: .timestamp(1_030_000_000), sourceFrame: 0, sourceEpoch: 2)
    try timing.recordSaved(
      frames: 480, observation: .timestamp(1_040_000_000), sourceFrame: 480, sourceEpoch: 2)
    let maps = try timing.finish(originNanoseconds: 1_000_000_000, expectedFrames: 1440)
    XCTAssertEqual(maps.count, 2)
    XCTAssertEqual(maps[0].sourceFrameEnd, 480)
    XCTAssertEqual(maps[0].timelineEndUs, 10000)
    XCTAssertEqual(maps[1].sourceFrameStart, 480)
    XCTAssertEqual(maps[1].timelineStartUs, 30000)
    XCTAssertEqual(maps[1].sourceFrameEnd, 1440)
    XCTAssertEqual(maps[1].timelineEndUs, 50000)
  }

  func testEpochReversalOverlapAndMissingTimestampDoNotGainClockProof() throws {
    for (epoch, observation) in [
      (UInt64(0), CaptureClockObservation.timestamp(1_030_000_000)),
      (2, .timestamp(1_005_000_000)),
      (2, .invalid("missing hardware timestamp")),
      (9_007_199_254_740_992, .timestamp(1_030_000_000)),
    ] {
      var stats = TrackStats(frames: 480, sampleRate: 48000, channels: 2)
      stats.recordSavedClock(
        .timestamp(1_000_000_000), frames: 480, sourceFrame: 0, sourceEpoch: 1)
      stats.recordSavedClock(observation, frames: 480, sourceFrame: 0, sourceEpoch: epoch)
      XCTAssertNotNil(stats.clockFailure)
      stats.recordSavedClock(
        .timestamp(1_040_000_000), frames: 480, sourceFrame: 480, sourceEpoch: 3)
      XCTAssertThrowsError(try stats.finishClock(originNanoseconds: 1_000_000_000))
      XCTAssertNil(stats.clockSegments)
    }
  }

  func testAnEpochBoundaryWithoutMissingTimeDoesNotInventAGap() throws {
    var timing = try CaptureTrackTiming(sampleRate: 48000)
    try timing.recordSaved(
      frames: 480, observation: .timestamp(1_000_000_000), sourceFrame: 5000, sourceEpoch: 1)
    try timing.recordSaved(
      frames: 480, observation: .timestamp(1_010_000_000), sourceFrame: 0, sourceEpoch: 2)
    let maps = try timing.finish(originNanoseconds: 1_000_000_000, expectedFrames: 960)
    XCTAssertEqual(maps.count, 1)
    XCTAssertEqual(maps[0].sourceFrameEnd, 960)
    XCTAssertEqual(maps[0].timelineEndUs, 20000)
  }

  func testCallbackFormatRequiresFullProcessingIdentityAndMatchingSampleClock() throws {
    let expected = try XCTUnwrap(AVAudioFormat(standardFormatWithSampleRate: 48000, channels: 2))
    let stats = TrackStats(sampleRate: 48000, channels: 2)
    let when = AVAudioTime(sampleTime: 480, atRate: 48000)
    let normal = try XCTUnwrap(AVAudioPCMBuffer(pcmFormat: expected, frameCapacity: 480))
    XCTAssertTrue(stats.acceptsMicrophonePCM(normal, at: when, processingFormat: expected))
    XCTAssertTrue(
      stats.acceptsMicrophonePCM(
        normal, at: AVAudioTime(hostTime: 1), processingFormat: expected))
    XCTAssertFalse(
      stats.acceptsMicrophonePCM(
        normal, at: AVAudioTime(sampleTime: 480, atRate: 44100), processingFormat: expected))
    XCTAssertFalse(
      TrackStats().acceptsMicrophonePCM(normal, at: when, processingFormat: expected))
    for changed in [
      AVAudioFormat(standardFormatWithSampleRate: 44100, channels: 2),
      AVAudioFormat(standardFormatWithSampleRate: 48000, channels: 1),
      AVAudioFormat(
        commonFormat: .pcmFormatInt16, sampleRate: 48000, channels: 2, interleaved: false),
      AVAudioFormat(
        commonFormat: .pcmFormatInt32, sampleRate: 48000, channels: 2, interleaved: false),
      AVAudioFormat(
        commonFormat: .pcmFormatFloat64, sampleRate: 48000, channels: 2, interleaved: false),
      AVAudioFormat(
        commonFormat: .pcmFormatFloat32, sampleRate: 48000, channels: 2, interleaved: true),
    ] {
      let format = try XCTUnwrap(changed)
      let buffer = try XCTUnwrap(AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 480))
      XCTAssertFalse(stats.acceptsMicrophonePCM(buffer, at: when, processingFormat: expected))
    }
    // A stale stats declaration is not repaired by a matching file/buffer.
    XCTAssertFalse(
      TrackStats(sampleRate: 44100, channels: 2).acceptsMicrophonePCM(
        normal, at: when, processingFormat: expected))
    XCTAssertFalse(
      TrackStats(sampleRate: 48000, channels: 1).acceptsMicrophonePCM(
        normal, at: when, processingFormat: expected))
  }

  func testRejectedRepresentationAndLayoutPreserveRawPrefixAndResumeOnlyOriginalFormat() throws {
    let root = FileManager.default.temporaryDirectory
      .appendingPathComponent("mon-microphone-format-\(UUID().uuidString)", isDirectory: true)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false)
    defer { try? FileManager.default.removeItem(at: root) }
    let canonical = try XCTUnwrap(realpath(root.path, nil))
    defer { free(canonical) }
    let directory = URL(fileURLWithPath: String(cString: canonical), isDirectory: true)
    let url = directory.appendingPathComponent("microphone.caf")
    let format = try XCTUnwrap(AVAudioFormat(standardFormatWithSampleRate: 8000, channels: 2))
    var file: AVAudioFile? = try AVAudioFile(
      forWriting: url, settings: format.settings, commonFormat: .pcmFormatFloat32,
      interleaved: false)
    let raw = try RawRecordingSegments(
      output: directory, owner: UUID().uuidString.lowercased(), lane: "microphone")
    var stats = TrackStats(sampleRate: 8000, channels: 2)
    var recovery = try MicrophoneRecovery(uid: first.uid)
    var token = try recovery.begin(first)
    try recovery.activate(token)
    func pcm(_ format: AVAudioFormat, value: Float = 0.25) throws -> AVAudioPCMBuffer {
      let buffer = try XCTUnwrap(AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 2000))
      buffer.frameLength = 2000
      if let data = buffer.floatChannelData, !format.isInterleaved {
        for channel in 0..<Int(format.channelCount) {
          for frame in 0..<2000 { data[channel][frame] = channel == 0 ? value : -value }
        }
      }
      return buffer
    }
    // Exercise the same production format guard before clock/file/raw writes.
    // This harness does not invoke AVAudioEngine or claim device integration.
    func receive(
      _ buffer: AVAudioPCMBuffer, token: MicrophoneRecovery.Binding, sourceFrame: Int64,
      at timestamp: Int64
    ) throws -> Bool {
      guard recovery.accepts(token) else { return false }
      let file = try XCTUnwrap(file)
      let when = AVAudioTime(sampleTime: sourceFrame, atRate: 8000)
      guard stats.acceptsMicrophonePCM(buffer, at: when, processingFormat: file.processingFormat)
      else {
        recovery.interrupt("format_changed", unavailable: true)
        stats.inputLevel.invalidate()
        return false
      }
      try stats.prepareSavedClock(
        .timestamp(timestamp), frames: 2000, sourceFrame: sourceFrame, sourceEpoch: token.epoch)
      try file.write(from: buffer)
      stats.callbackCount += 1
      stats.frames += 2000
      try raw.enqueue(
        buffer, observation: .timestamp(timestamp), sourceFrame: sourceFrame,
        sourceEpoch: token.epoch)
      try raw.drainAcceptedPCM()
      return true
    }
    let normal = try pcm(format)
    for index in 0..<17 {
      XCTAssertTrue(
        try receive(
          normal, token: token, sourceFrame: Int64(index) * 2000,
          at: 1_000_000_000 + Int64(index) * 250_000_000))
    }
    XCTAssertEqual(raw.snapshot().sealedFrames, 32000)
    let prefixURL = directory.appendingPathComponent("raw-microphone-00000001.caf")
    let entryURL = directory.appendingPathComponent("raw-microphone-00000001.json")
    let tipURL = directory.appendingPathComponent("raw-microphone.json")
    let prefix = try Data(contentsOf: prefixURL)
    let entry = try Data(contentsOf: entryURL)
    let tip = try Data(contentsOf: tipURL)
    for (index, changed) in [
      AVAudioFormat(
        commonFormat: .pcmFormatFloat32, sampleRate: 16000, channels: 2, interleaved: false),
      AVAudioFormat(
        commonFormat: .pcmFormatFloat32, sampleRate: 8000, channels: 1, interleaved: false),
      AVAudioFormat(
        commonFormat: .pcmFormatInt16, sampleRate: 8000, channels: 2, interleaved: false),
      AVAudioFormat(
        commonFormat: .pcmFormatFloat32, sampleRate: 8000, channels: 2, interleaved: true),
    ].enumerated() {
      let before = stats.frames
      let callbacks = stats.callbackCount
      let retired = token
      XCTAssertFalse(
        try receive(
          pcm(try XCTUnwrap(changed)), token: retired, sourceFrame: 34000,
          at: 5_000_000_000 + Int64(index) * 1_000_000_000))
      XCTAssertEqual(recovery.status.reason, "format_changed")
      XCTAssertEqual(recovery.status.state, "unavailable")
      recovery.interruptForRetry()
      XCTAssertEqual(recovery.status.reason, "format_changed")
      XCTAssertEqual(recovery.status.state, "unavailable")
      XCTAssertEqual(stats.frames, before)
      XCTAssertEqual(stats.callbackCount, callbacks)
      XCTAssertEqual(stats.appendFailures, 0)
      XCTAssertNil(stats.clockFailure)
      XCTAssertNil(raw.snapshot().failure)
      XCTAssertEqual(try XCTUnwrap(file).length, before)
      XCTAssertEqual(try Data(contentsOf: prefixURL), prefix)
      XCTAssertEqual(try Data(contentsOf: entryURL), entry)
      XCTAssertEqual(try Data(contentsOf: tipURL), tip)
      token = try recovery.begin(first)
      try recovery.activate(token)
      XCTAssertFalse(
        try receive(normal, token: retired, sourceFrame: 0, at: 5_000_000_000))
      XCTAssertTrue(
        try receive(
          pcm(format, value: 0.5), token: token, sourceFrame: 0,
          at: 6_000_000_000 + Int64(index) * 1_000_000_000))
    }
    try raw.finish()
    file = nil
    try stats.finishClock(originNanoseconds: 1_000_000_000)
    XCTAssertEqual(stats.frames, 42000)
    XCTAssertEqual(stats.clockSegments?.count, 5)
    XCTAssertEqual(stats.clockSegments?[0].timelineEndUs, 4_250_000)
    XCTAssertEqual(stats.clockSegments?[1].timelineStartUs, 5_000_000)
    XCTAssertEqual(stats.clockSegments?[2].timelineStartUs, 6_000_000)
    XCTAssertEqual(stats.clockSegments?[4].timelineStartUs, 8_000_000)
    XCTAssertEqual(raw.snapshot().sealedFrames, 42000)
    XCTAssertTrue(raw.snapshot().producerClosed)
    XCTAssertEqual(try Data(contentsOf: prefixURL), prefix)
    XCTAssertEqual(try Data(contentsOf: entryURL), entry)
    let reader = try AVAudioFile(forReading: url)
    XCTAssertEqual(reader.length, 42000)
    let read = try XCTUnwrap(
      AVAudioPCMBuffer(pcmFormat: reader.processingFormat, frameCapacity: 42000))
    try reader.read(into: read)
    XCTAssertEqual(read.frameLength, 42000)
    for channel in 0..<2 {
      for frame in 0..<42000 {
        let value: Float = frame < 34000 ? 0.25 : 0.5
        XCTAssertEqual(read.floatChannelData![channel][frame], channel == 0 ? value : -value)
      }
    }
  }

  func testSyntheticCAFAppendsAcrossVerifiedBindingsWithoutOverwritingOrPadding() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(
      "mon-microphone-recovery-\(UUID().uuidString)", isDirectory: true)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
    defer { try? FileManager.default.removeItem(at: directory) }
    let url = directory.appendingPathComponent("microphone.caf")
    let format = try XCTUnwrap(AVAudioFormat(standardFormatWithSampleRate: 48000, channels: 2))
    var file: AVAudioFile? = try AVAudioFile(
      forWriting: url, settings: format.settings, commonFormat: .pcmFormatFloat32,
      interleaved: false)
    XCTAssertTrue(try XCTUnwrap(file).processingFormat.isEqual(format))
    for changed in [
      AVAudioFormat(standardFormatWithSampleRate: 44100, channels: 2),
      AVAudioFormat(standardFormatWithSampleRate: 48000, channels: 1),
      AVAudioFormat(
        commonFormat: .pcmFormatInt16, sampleRate: 48000, channels: 2, interleaved: false),
    ] {
      XCTAssertFalse(try XCTUnwrap(file).processingFormat.isEqual(try XCTUnwrap(changed)))
    }
    var recovery = try MicrophoneRecovery(uid: first.uid)
    var stats = TrackStats(sampleRate: 48000, channels: 2)
    func append(_ token: MicrophoneRecovery.Binding, at time: Int64, value: Float) throws {
      guard recovery.accepts(token) else { return }
      let buffer = try XCTUnwrap(AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 480))
      buffer.frameLength = 480
      for channel in 0..<2 {
        for frame in 0..<480 {
          buffer.floatChannelData![channel][frame] = channel == 0 ? value : -value
        }
      }
      try XCTUnwrap(file).write(from: buffer)
      stats.frames += 480
      stats.recordSavedClock(
        .timestamp(time), frames: 480, sourceFrame: 0, sourceEpoch: token.epoch)
    }
    let original = try recovery.begin(first)
    try recovery.activate(original)
    try append(original, at: 1_000_000_000, value: 0.25)
    recovery.interrupt("device_disconnected")
    try append(original, at: 1_010_000_000, value: 9)
    let returned = try recovery.begin(first)
    try recovery.activate(returned)
    try append(original, at: 1_020_000_000, value: 9)
    try append(returned, at: 1_030_000_000, value: 0.5)
    file = nil
    recovery.stop()
    stats.microphoneRecovery = recovery.status
    let systemURL = directory.appendingPathComponent("system.caf")
    var systemFile: AVAudioFile? = try AVAudioFile(
      forWriting: systemURL, settings: format.settings, commonFormat: .pcmFormatFloat32,
      interleaved: false)
    let systemBuffer = try XCTUnwrap(AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 1920))
    systemBuffer.frameLength = 1920
    for channel in 0..<2 {
      for frame in 0..<1920 {
        systemBuffer.floatChannelData![channel][frame] = channel == 0 ? 0.125 : -0.125
      }
    }
    try XCTUnwrap(systemFile).write(from: systemBuffer)
    systemFile = nil
    var system = TrackStats(frames: 1920, sampleRate: 48000, channels: 2)
    system.recordSavedClock(.timestamp(1_000_000_000), frames: 1920, sourceFrame: nil)
    let reading = CaptureClockReading(
      hostBeforeNs: 999_999_990, continuousNs: 1_000_000_000,
      utcMs: 1_700_000_000_000, hostAfterNs: 1_000_000_010)
    let clock = try CaptureClockDomain(units: MachTimeUnits(numerator: 1, denominator: 1)) {
      reading
    }
    var manifest = Manifest(
      schemaVersion: 2, scenario: "synthetic-microphone-recovery", captureMode: "system",
      backend: "screencapturekit", targetPID: nil, targetPIDs: [],
      startedAtUTC: "2023-11-14T22:13:20Z", requestedDurationSeconds: nil,
      osVersion: "synthetic", architecture: "arm64", state: "saved",
      microphone: stats, system: system, errors: [])
    manifest.microphoneDeviceUID = first.uid
    try manifest.finalizeClock(clock)
    stats = manifest.microphone
    try verifyRecordedCAF(url, expected: stats)
    try verifyRecordedCAF(systemURL, expected: manifest.system)
    XCTAssertEqual(stats.frames, 960)
    XCTAssertEqual(stats.clockSegments?.count, 2)
    XCTAssertEqual(stats.clockSegments?[0].timelineEndUs, 10000)
    XCTAssertEqual(stats.clockSegments?[1].timelineStartUs, 30000)
    let reader = try AVAudioFile(forReading: url)
    XCTAssertEqual(reader.length, 960)
    let read = try XCTUnwrap(
      AVAudioPCMBuffer(pcmFormat: reader.processingFormat, frameCapacity: 960))
    try reader.read(into: read)
    XCTAssertEqual(read.frameLength, 960)
    for channel in 0..<2 {
      for frame in 0..<960 {
        let expected: Float = (frame < 480 ? 0.25 : 0.5) * (channel == 0 ? 1 : -1)
        XCTAssertEqual(read.floatChannelData![channel][frame], expected)
      }
    }
    try manifest.encodedData().write(
      to: directory.appendingPathComponent("manifest.json"), options: .withoutOverwriting)
    if let destinationPath = ProcessInfo.processInfo.environment[
      "MON_MICROPHONE_RECOVERY_FIXTURE_DIR"]
    {
      let destination = URL(fileURLWithPath: destinationPath, isDirectory: true)
      XCTAssertTrue(try FileManager.default.contentsOfDirectory(atPath: destinationPath).isEmpty)
      for name in ["manifest.json", "microphone.caf", "system.caf"] {
        try FileManager.default.copyItem(
          at: directory.appendingPathComponent(name), to: destination.appendingPathComponent(name))
      }
    }
  }
}
