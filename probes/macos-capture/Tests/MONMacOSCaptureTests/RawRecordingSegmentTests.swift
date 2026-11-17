import AVFoundation
import CoreMedia
import CryptoKit
import Darwin
import Foundation
import XCTest

@testable import MONMacOSCapture

private final class RawSegmentGate: @unchecked Sendable {
  let entered = DispatchSemaphore(value: 0)
  let released = DispatchSemaphore(value: 0)
  private let lock = NSLock()
  private var waited = false
  func wait() {
    entered.signal()
    _ = released.wait(timeout: .now() + 5)
  }
  func waitOnce() {
    let first = lock.withLock {
      if waited { return false }
      waited = true
      return true
    }
    if first { wait() }
  }
  func holdOnce() {
    let first = lock.withLock {
      if waited { return false }
      waited = true
      return true
    }
    if first {
      entered.signal()
      released.wait()
    }
  }
}

private final class RawCommitLog: @unchecked Sendable {
  private let lock = NSLock()
  private var writes: [Int] = []
  private var turns: [String] = []
  func audio(_ data: Data) {
    if data.starts(with: Data("caff".utf8)) { lock.withLock { writes.append(data.count) } }
  }
  func turn(_ lane: String) { lock.withLock { turns.append(lane) } }
  var audioWrites: [Int] { lock.withLock { writes } }
  var order: [String] { lock.withLock { turns } }
}

private final class StorageFault: @unchecked Sendable {
  private let lock = NSLock()
  private var armed = false
  private var remaining = 0
  func arm(skipping: Int = 0) {
    lock.withLock {
      armed = true
      remaining = skipping
    }
  }
  func active() -> Bool {
    lock.withLock {
      guard armed else { return false }
      if remaining > 0 {
        remaining -= 1
        return false
      }
      return true
    }
  }
  func io(code: POSIXErrorCode, sync: Bool = false) -> RawRecordingSegments.StorageIO {
    let native = RawRecordingSegments.StorageIO()
    return RawRecordingSegments.StorageIO(
      write: { [self] data, descriptor in
        if !sync, active() {
          // A real short write can precede ENOSPC. Never advertise those bytes.
          try native.write(Data(data.prefix(8)), descriptor)
          throw POSIXError(code)
        }
        try native.write(data, descriptor)
      },
      synchronize: { [self] descriptor in
        if sync, active() { throw POSIXError(code) }
        try native.synchronize(descriptor)
      })
  }
}

final class RawRecordingSegmentTests: XCTestCase, @unchecked Sendable {
  private let owner = "735027d4-a03e-43d3-830a-21e767dfaf88"
  private let origin: Int64 = 9_000_000_000_000

  func testRebasedOwnedLanesRetainEveryPCMFrameAndReportEstimatedTiming() throws {
    for rate in [8000, 48000] {
      for lane in ["microphone", "system"] {
        let root = try directory()
        defer { try? FileManager.default.removeItem(at: root) }
        let raw = try RawRecordingSegments(
          output: root, owner: owner, lane: lane, allowsClockRebasing: true)
        let pcm = try buffer(frames: UInt32(rate / 4), rate: Double(rate))
        for index in 0..<40 {
          let observedIndex = index < 20 ? index : index - 18
          try raw.enqueue(
            pcm, observation: .timestamp(origin + Int64(observedIndex) * 250_000_000),
            sourceFrame: nil, sourceEpoch: 1)
        }
        try raw.finish()
        var stats = raw.applyingSavedAudio(to: TrackStats(callbackCount: 40))
        try raw.verifyClosed(expected: stats)
        XCTAssertEqual(stats.frames, 160_000)
        XCTAssertTrue(raw.snapshot().producerClosed)
        XCTAssertNil(raw.snapshot().failure)
        XCTAssertEqual(stats.clockAlignment?.clockAdjustments, 1)
        XCTAssertEqual(stats.clockAlignment?.retimedPackets, 20)
        XCTAssertEqual(stats.clockAlignment?.correctionNanoseconds, 4_500_000_000)
        try stats.finishClock(originNanoseconds: origin)
        XCTAssertEqual(stats.clockSegments?.last?.timelineEndUs, 10_000_000)
        var saved: Int64 = 0
        for sequence in 1...raw.snapshot().sealedSegments {
          let path = root.appendingPathComponent(String(format: "raw-%@-%08d.caf", lane, sequence))
          let file = try AVAudioFile(forReading: path)
          let decoded = try XCTUnwrap(
            AVAudioPCMBuffer(
              pcmFormat: file.processingFormat, frameCapacity: AVAudioFrameCount(file.length)))
          try file.read(into: decoded)
          saved += Int64(decoded.frameLength)
          XCTAssertTrue(
            (0..<Int(decoded.frameLength)).allSatisfy {
              abs(decoded.floatChannelData![0][$0] + 0.125) < 0.001
            })
        }
        XCTAssertEqual(saved, 160_000)
      }
    }
  }

  func testRejectedSourceDataDoesNotPoisonSavedPrefixAndNewBindingContinues() throws {
    for lane in ["microphone", "system"] {
      for fault in ["counter", "nonfinite", "infinity", "amplitude", "format"] {
        let root = try directory()
        defer { try? FileManager.default.removeItem(at: root) }
        let raw = try RawRecordingSegments(
          output: root, owner: owner, lane: lane, allowsClockRebasing: true)
        let pcm = try buffer()
        for index in 0..<17 {
          try raw.enqueue(
            pcm, observation: .timestamp(origin + Int64(index) * 250_000_000),
            sourceFrame: Int64(index * 2000), sourceEpoch: 1)
        }
        try raw.drainAcceptedPCM()
        let original = try Data(contentsOf: root.appendingPathComponent("raw-\(lane).json"))
        let bad = try buffer(rate: fault == "format" ? 16000 : 8000)
        if fault == "nonfinite" { bad.floatChannelData![0][0] = .nan }
        if fault == "infinity" { bad.floatChannelData![0][0] = .infinity }
        if fault == "amplitude" { bad.floatChannelData![0][0] = 17 }
        XCTAssertThrowsError(
          try raw.enqueue(
            bad, observation: .timestamp(origin + 4_250_000_000),
            sourceFrame: fault == "counter" ? 0 : 34_000, sourceEpoch: 1)
        ) { error in
          XCTAssertTrue(error is CaptureInputFault)
        }
        XCTAssertEqual(
          try Data(contentsOf: root.appendingPathComponent("raw-\(lane).json")), original)
        XCTAssertNil(raw.snapshot().failure)
        try raw.retireSource()
        try raw.drainAcceptedPCM()
        XCTAssertEqual(
          raw.applyingSavedAudio(to: TrackStats()).frames, 68_000,
          "valid converter tail must survive")
        try raw.enqueue(
          pcm, observation: .timestamp(origin + 10_000_000_000), sourceFrame: 0, sourceEpoch: 2)
        try raw.finish()
        let stats = raw.applyingSavedAudio(to: TrackStats())
        XCTAssertEqual(stats.frames, 72_000)
        XCTAssertEqual(
          raw.snapshot().sealedSegments, 2, "no short intermediate receipt may be published")
        XCTAssertTrue(raw.snapshot().producerClosed)
        XCTAssertNil(raw.snapshot().failure)
        try raw.verifyClosed(expected: stats)
      }
    }
  }

  func testRetiredBindingsAndUnrecoverableClockMappingStillFailClosed() throws {
    for fault in ["epoch", "strict"] {
      let root = try directory()
      defer { try? FileManager.default.removeItem(at: root) }
      let raw = try RawRecordingSegments(
        output: root, owner: owner, lane: "system", allowsClockRebasing: fault != "strict")
      for index in 0..<17 {
        try raw.enqueue(
          buffer(), observation: .timestamp(origin + Int64(index) * 250_000_000),
          sourceFrame: Int64(index * 2000), sourceEpoch: 1)
      }
      try raw.drainAcceptedPCM()
      if fault == "epoch" {
        XCTAssertThrowsError(
          try raw.enqueue(
            buffer(), observation: .timestamp(origin + 4_250_000_000), sourceFrame: 34_000,
            sourceEpoch: 0)
        ) {
          XCTAssertFalse($0 is CaptureInputFault, "generation ownership is not a source error")
        }
      } else {
        try raw.enqueue(
          buffer(), observation: .timestamp(origin + 500_000_000), sourceFrame: 34_000,
          sourceEpoch: 1)
        XCTAssertThrowsError(try raw.finish())
        XCTAssertNotNil(raw.snapshot().failure)
      }
    }
  }

  func testEstimatedTimelineRequiresBothClosuresAndCoversPreservedAudioPastNativeStop() throws {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    let microphone = try RawRecordingSegments(
      output: root, owner: owner, lane: "microphone", allowsClockRebasing: true)
    let system = try RawRecordingSegments(
      output: root, owner: owner, lane: "system", allowsClockRebasing: true)
    for index in 0..<8 {
      let observedIndex = index < 4 ? index : index - 3
      try system.enqueue(
        buffer(), observation: .timestamp(origin + Int64(observedIndex) * 250_000_000),
        sourceFrame: nil, sourceEpoch: 1)
      if index < 4 {
        try microphone.enqueue(
          buffer(), observation: .timestamp(origin + Int64(index) * 250_000_000),
          sourceFrame: nil, sourceEpoch: 1)
      }
    }
    try microphone.finish()
    try system.finish()
    func reading(_ time: Int64) -> CaptureClockReading {
      CaptureClockReading(
        hostBeforeNs: time, continuousNs: time + 10,
        utcMs: 1_700_000_000_000, hostAfterNs: time + 20)
    }
    let time = ReadingBox(reading(origin - 100))
    let clock = try CaptureClockDomain(
      units: MachTimeUnits(numerator: 1, denominator: 1), read: time.read)
    time.set(reading(origin + 1_500_000_000))
    let micClosed = try clock.close(.microphone)
    let pcClosed = try clock.close(.system)
    let original = Manifest(
      schemaVersion: 4, scenario: "synthetic-rebased-stop", captureMode: "system",
      backend: "coreaudio",
      targetPID: nil, targetPIDs: [], startedAtUTC: "synthetic", requestedDurationSeconds: nil,
      osVersion: "synthetic", architecture: "arm64", state: "saved",
      microphone: microphone.applyingSavedAudio(to: TrackStats(callbackCount: 4)),
      system: system.applyingSavedAudio(to: TrackStats(callbackCount: 8)), errors: [],
      recordingOwnerId: owner)
    var saved = original
    try saved.finalizeStoppedClock(clock, microphoneClosure: micClosed, systemClosure: pcClosed)
    XCTAssertEqual(saved.timingEstimated, true)
    XCTAssertEqual(saved.system.frames, 32_000)
    XCTAssertEqual(saved.system.clockSegments?.last?.timelineEndUs, 2_000_000)
    XCTAssertEqual(saved.captureEnd?.endUs, 2_000_000)
    XCTAssertEqual(saved.observedCaptureEnd?.endUs, 1_500_000)
    XCTAssertGreaterThanOrEqual(saved.captureClock!.uncertaintyNs, 2_001_000_000)
    let json = try XCTUnwrap(
      JSONSerialization.jsonObject(with: saved.encodedData()) as? [String: Any])
    XCTAssertEqual(json["timingEstimated"] as? Bool, true)
    var missingProof = original
    XCTAssertThrowsError(
      try missingProof.finalizeStoppedClock(clock, microphoneClosure: nil, systemClosure: pcClosed))
    var unmarked = original
    unmarked.system.clockAlignment = nil
    XCTAssertThrowsError(
      try unmarked.finalizeStoppedClock(
        clock, microphoneClosure: micClosed, systemClosure: pcClosed))
  }

  private func directory() throws -> URL {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(
      at: root, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
    let path = try XCTUnwrap(realpath(root.path, nil))
    defer { free(path) }
    return URL(fileURLWithPath: String(cString: path), isDirectory: true)
  }
  private func buffer(frames: UInt32 = 2000, rate: Double = 8000, interleaved: Bool = false) throws
    -> AVAudioPCMBuffer
  {
    let format = try XCTUnwrap(
      AVAudioFormat(
        commonFormat: .pcmFormatFloat32, sampleRate: rate, channels: 2, interleaved: interleaved))
    let buffer = try XCTUnwrap(AVAudioPCMBuffer(pcmFormat: format, frameCapacity: frames))
    buffer.frameLength = frames
    let samples = try XCTUnwrap(buffer.floatChannelData)
    for frame in 0..<Int(frames) {
      samples[0][interleaved ? frame * 2 : frame] = 0.25
      samples[interleaved ? 0 : 1][interleaved ? frame * 2 + 1 : frame] = -0.5
    }
    return buffer
  }
  private func value(_ root: URL, _ name: String = "raw-microphone.json") throws -> [String: Any] {
    try XCTUnwrap(
      JSONSerialization.jsonObject(with: Data(contentsOf: root.appendingPathComponent(name)))
        as? [String: Any])
  }
  private func append(
    _ writer: RawRecordingSegments, start: Int = 0, count: Int = 17, gap: Int64 = 0,
    epoch: UInt64 = 0, interleaved: Bool = false
  ) throws {
    for index in start..<start + count {
      try writer.enqueue(
        buffer(interleaved: interleaved),
        observation: .timestamp(origin + Int64(index) * 250_000_000 + gap),
        sourceFrame: Int64(index) * 2000, sourceEpoch: epoch)
      try writer.drainAcceptedPCM()
    }
  }
  private func waitForPrefix(_ writer: RawRecordingSegments, count: Int = 1) throws {
    let deadline = ContinuousClock.now.advanced(by: .seconds(5))
    while writer.snapshot().sealedSegments < count, writer.snapshot().failure == nil,
      ContinuousClock.now < deadline
    { Thread.sleep(forTimeInterval: 0.005) }
    XCTAssertNil(writer.snapshot().failure)
    XCTAssertEqual(writer.snapshot().sealedSegments, count)
  }
  private func verifyAudio(_ url: URL, frames: Int64, rate: Double = 8000) throws {
    let file = try AVAudioFile(forReading: url)
    let compact = file.fileFormat.commonFormat == .pcmFormatInt16
    let expectedFrames = compact ? Int64(ceil(Double(frames) * 16000 / rate)) : frames
    XCTAssertEqual(file.length, expectedFrames)
    XCTAssertEqual(file.fileFormat.sampleRate, compact ? 16000 : rate)
    XCTAssertEqual(file.fileFormat.channelCount, compact ? 1 : 2)
    let output = try XCTUnwrap(
      AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: 4096))
    var read: Int64 = 0
    while read < expectedFrames {
      try file.read(into: output, frameCount: AVAudioFrameCount(min(4096, expectedFrames - read)))
      guard output.frameLength > 0 else {
        XCTFail("unexpected end of CAF")
        return
      }
      let values = try XCTUnwrap(output.floatChannelData)
      for index in 0..<Int(output.frameLength) {
        if compact {
          XCTAssertEqual(values[0][index], -0.125, accuracy: 1.0 / 32768)
        } else {
          XCTAssertEqual(values[0][index], 0.25)
          XCTAssertEqual(values[1][index], -0.5)
        }
      }
      read += Int64(output.frameLength)
    }
  }

  func testLivePrefixIsIndependentCAFBeforeStopAndFinalTailIsChainedWithoutPadding() throws {
    for interleaved in [false, true] {
      let root = try directory()
      defer { try? FileManager.default.removeItem(at: root) }
      let writer = try RawRecordingSegments(output: root, owner: owner, lane: "microphone")
      try append(writer, interleaved: interleaved)
      try waitForPrefix(writer)
      let tip = try value(root)
      XCTAssertEqual(tip["sealedFrames"] as? Int64, 64_000)
      XCTAssertEqual(tip["producerClosed"] as? Bool, false)
      XCTAssertEqual(tip["ownerId"] as? String, owner)
      let firstName = "raw-microphone-00000001.caf"
      try verifyAudio(root.appendingPathComponent(firstName), frames: 32_000)
      let firstData = try Data(contentsOf: root.appendingPathComponent(firstName))
      let first = try value(root, "raw-microphone-00000001.json")
      XCTAssertEqual(first["audioSHA256"] as? String, hex(SHA256.hash(data: firstData)))
      XCTAssertEqual(first["sourceFrameStart"] as? Int, 0)
      let maps = try XCTUnwrap(first["clockSegments"] as? [[String: Any]])
      XCTAssertEqual(maps.count, 1)
      XCTAssertEqual(maps[0]["sourceFrameEnd"] as? Int, 64_000)
      XCTAssertEqual(maps[0]["timelineEndUs"] as? Int, 4_000_000)
      let firstEntry = try Data(
        contentsOf: root.appendingPathComponent("raw-microphone-00000001.json"))
      XCTAssertEqual(tip["lastEntrySHA256"] as? String, hex(SHA256.hash(data: firstEntry)))
      try writer.finish()
      XCTAssertEqual(writer.snapshot().sealedSegments, 2)
      XCTAssertEqual(writer.snapshot().sealedFrames, 68_000)
      XCTAssertTrue(writer.snapshot().producerClosed)
      try writer.finish()
      XCTAssertEqual(try Data(contentsOf: root.appendingPathComponent(firstName)), firstData)
      try verifyAudio(root.appendingPathComponent("raw-microphone-00000002.caf"), frames: 2000)
      let second = try value(root, "raw-microphone-00000002.json")
      XCTAssertEqual(second["sourceFrameStart"] as? Int, 64_000)
      XCTAssertEqual(second["previousEntrySHA256"] as? String, hex(SHA256.hash(data: firstEntry)))
      XCTAssertEqual(second["captureOriginNanoseconds"] as? Int64, origin + 4_000_000_000)
      XCTAssertNil(second["acquisitionClosed"])
      XCTAssertEqual(try value(root)["producerClosed"] as? Bool, true)
    }
  }

  func testPendingWindowAllowsTwentySecondsAndRejectsBeforeDiscardingPCM() throws {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    let gate = RawSegmentGate()
    let writer = try RawRecordingSegments(
      output: root, owner: owner, lane: "microphone", beforeAppend: gate.waitOnce)
    defer { gate.released.signal() }
    let quarter = try buffer()
    try writer.enqueue(quarter, observation: .timestamp(origin), sourceFrame: 0)
    XCTAssertEqual(gate.entered.wait(timeout: .now() + 2), .success)
    for index in 1..<80 {
      try writer.enqueue(
        quarter, observation: .timestamp(origin + Int64(index) * 250_000_000),
        sourceFrame: Int64(index) * 2000)
    }
    XCTAssertThrowsError(
      try writer.enqueue(
        buffer(frames: 1),
        observation: .timestamp(origin + 20_000_000_000), sourceFrame: 160_000))
    XCTAssertEqual(writer.snapshot().sealedFrames, 0)
    XCTAssertFalse(writer.snapshot().producerClosed)
    gate.released.signal()
    XCTAssertThrowsError(try writer.drainAcceptedPCM())
    XCTAssertThrowsError(try writer.finish())
    let tip = try value(root)
    XCTAssertEqual(tip["sealedFrames"] as? Int64, 320_000)
    XCTAssertEqual(tip["sealedSegments"] as? Int, 5)
    XCTAssertEqual(tip["producerClosed"] as? Bool, false)
  }

  func testSegmentBoundaryUsesMeasuredNextTimestampAndNeverChangesPublishedClock() throws {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    let writer = try RawRecordingSegments(output: root, owner: owner, lane: "microphone")
    for index in 0..<17 {
      try writer.enqueue(
        buffer(), observation: .timestamp(origin + Int64(index) * 250_012_500),
        sourceFrame: Int64(index) * 2000)
      try writer.drainAcceptedPCM()
    }
    try waitForPrefix(writer)
    let maps = try XCTUnwrap(
      try value(root, "raw-microphone-00000001.json")["clockSegments"] as? [[String: Any]])
    XCTAssertEqual(maps.last?["timelineEndUs"] as? Int, 4_000_200)
    let original = try Data(contentsOf: root.appendingPathComponent("raw-microphone-00000001.json"))
    try writer.enqueue(
      buffer(frames: 100), observation: .timestamp(origin + 8_000_000_000),
      sourceFrame: 0, sourceEpoch: 1)
    try writer.finish()
    XCTAssertEqual(
      try Data(contentsOf: root.appendingPathComponent("raw-microphone-00000001.json")), original)
    let tail = try XCTUnwrap(
      try value(root, "raw-microphone-00000002.json")["clockSegments"] as? [[String: Any]])
    XCTAssertEqual(tail.count, 2)
    XCTAssertEqual(tail[0]["sourceFrameEnd"] as? Int, 4000)
    XCTAssertEqual(tail[1]["sourceFrameStart"] as? Int, 4000)
    XCTAssertGreaterThan(
      tail[1]["timelineStartUs"] as? Int ?? 0, tail[0]["timelineEndUs"] as? Int ?? 0)
  }

  func testCollisionSymlinkAndHardlinkCannotBeOverwrittenOrAdvertised() throws {
    for kind in ["existing", "symlink", "hardlink"] {
      let root = try directory()
      defer { try? FileManager.default.removeItem(at: root) }
      let foreign = root.appendingPathComponent("foreign")
      let leaf = root.appendingPathComponent("raw-microphone-00000001.caf")
      let content = Data("retain other user's content".utf8)
      try content.write(to: foreign)
      if kind == "symlink" {
        try FileManager.default.createSymbolicLink(at: leaf, withDestinationURL: foreign)
      } else if kind == "hardlink" {
        try FileManager.default.linkItem(at: foreign, to: leaf)
      } else {
        try content.write(to: leaf)
      }
      let writer = try RawRecordingSegments(output: root, owner: owner, lane: "microphone")
      try writer.enqueue(buffer(), observation: .timestamp(origin), sourceFrame: 0)
      XCTAssertThrowsError(try writer.finish())
      XCTAssertEqual(writer.snapshot().sealedSegments, 0)
      XCTAssertFalse(writer.snapshot().producerClosed)
      XCTAssertEqual(try Data(contentsOf: foreign), content)
      XCTAssertEqual(try Data(contentsOf: leaf), content)
      XCTAssertEqual(try value(root)["sealedSegments"] as? Int, 0)
    }
  }

  func testReplacedPrefixPointerCannotAuthorizeFurtherSegmentsOrRemoveReplacement() throws {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    let writer = try RawRecordingSegments(output: root, owner: owner, lane: "microphone")
    try append(writer)
    try waitForPrefix(writer)
    let prefix = root.appendingPathComponent("raw-microphone.json")
    let saved = root.appendingPathComponent("original-pointer")
    try FileManager.default.moveItem(at: prefix, to: saved)
    let foreign = Data("unowned pointer".utf8)
    try foreign.write(to: prefix)
    XCTAssertThrowsError(try writer.finish())
    XCTAssertEqual(writer.snapshot().sealedSegments, 1)
    XCTAssertFalse(writer.snapshot().producerClosed)
    XCTAssertEqual(try Data(contentsOf: prefix), foreign)
    try verifyAudio(root.appendingPathComponent("raw-microphone-00000001.caf"), frames: 32_000)
    XCTAssertEqual(try value(root, "original-pointer")["sealedSegments"] as? Int, 1)
  }

  func testChangedAncestorIsPinnedAndNeverWritesIntoReplacementDirectory() throws {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    let audio = root.appendingPathComponent("audio")
    try FileManager.default.createDirectory(at: audio, withIntermediateDirectories: false)
    let writer = try RawRecordingSegments(output: audio, owner: owner, lane: "microphone")
    let moved = root.appendingPathComponent("original")
    try FileManager.default.moveItem(at: audio, to: moved)
    try FileManager.default.createDirectory(at: audio, withIntermediateDirectories: false)
    try append(writer)
    try writer.finish()
    XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: audio.path), [])
    try verifyAudio(moved.appendingPathComponent("raw-microphone-00000001.caf"), frames: 32_000)
    XCTAssertEqual(try value(moved)["sealedFrames"] as? Int, 68_000)
  }

  func testProductionSystemPCMWriterStoresSameRealSamplesInLiveSegments() async throws {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    let raw = try RawRecordingSegments(output: root, owner: owner, lane: "system")
    let writer = SystemPCMWriter(
      outputURL: root.appendingPathComponent("system.caf"), rawSegments: raw)
    let fixture = CaptureClockFixtureTests()
    for index in 0..<17 {
      let sample = try fixture.sample(
        frames: 12000, value: 0.25, presentation: CMTime(value: Int64(index), timescale: 4),
        rightValue: -0.5)
      try writer.append(
        sample, observation: .timestamp(origin + Int64(index) * 250_000_000),
        sourceEpoch: 0, levelsEnabled: false, receivedAt: nil)
      try raw.drainAcceptedPCM()
    }
    try waitForPrefix(raw)
    XCTAssertEqual(writer.stats.rawSegments?.sealedFrames, 64_000)
    try await writer.finish()
    XCTAssertEqual(writer.stats.frames, 68_000)
    XCTAssertEqual(writer.stats.rawSegments?.sealedFrames, 68_000)
    XCTAssertEqual(writer.stats.rawSegments?.producerClosed, true)
    XCTAssertFalse(
      FileManager.default.fileExists(atPath: root.appendingPathComponent("system.caf").path))
    try raw.verifyClosed(expected: writer.stats)
    try verifyAudio(
      root.appendingPathComponent("raw-system-00000001.caf"), frames: 192_000, rate: 48000)
    try verifyAudio(
      root.appendingPathComponent("raw-system-00000002.caf"), frames: 12000, rate: 48000)
  }

  func testOneBatchedCAFWritePerSegmentAndNoContinuousDuplicate() throws {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    let log = RawCommitLog()
    let native = RawRecordingSegments.StorageIO()
    let commits = RawSegmentCommitWorker()
    let writer = try RawRecordingSegments(
      output: root, owner: owner, lane: "microphone",
      storageIO: .init(
        write: { data, fd in
          log.audio(data)
          try native.write(data, fd)
        }, synchronize: native.synchronize), commits: commits)
    try append(writer)
    XCTAssertEqual(log.audioWrites, [128_068])
    XCTAssertFalse(
      FileManager.default.fileExists(atPath: root.appendingPathComponent("microphone.caf").path))
    try writer.finish()
    XCTAssertEqual(log.audioWrites, [128_068, 8_068])
    XCTAssertEqual(commits.pending(lane: "microphone").nanoseconds, 0)
    XCTAssertEqual(commits.pending(lane: "microphone").bytes, 0)
    XCTAssertEqual(commits.pending(lane: "microphone").jobs, 0)
    try writer.verifyClosed(expected: writer.applyingSavedAudio(to: TrackStats()))
    try verifyAudio(root.appendingPathComponent("raw-microphone-00000001.caf"), frames: 32_000)
    try verifyAudio(root.appendingPathComponent("raw-microphone-00000002.caf"), frames: 2_000)
  }

  func testDiskSyncPauseDoesNotRetainNativePCMAndRecoversOneHundredSecondsWithoutLoss() throws {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    let gate = RawSegmentGate()
    defer { gate.released.signal() }
    let native = RawRecordingSegments.StorageIO()
    let commits = RawSegmentCommitWorker()
    let writer = try RawRecordingSegments(
      output: root, owner: owner, lane: "microphone",
      storageIO: .init(
        write: native.write,
        synchronize: { fd in
          gate.holdOnce()
          try native.synchronize(fd)
        }), commits: commits)
    let quarter = try buffer()
    for index in 0..<400 {
      try writer.enqueue(
        quarter,
        observation: .timestamp(origin + Int64(index) * 250_000_000),
        sourceFrame: Int64(index) * 2000)
      try writer.drainNormalization()
      if index == 16 { XCTAssertEqual(gate.entered.wait(timeout: .now() + 2), .success) }
    }
    XCTAssertEqual(writer.snapshot().sealedFrames, 0)
    XCTAssertEqual(commits.pending(lane: "microphone").nanoseconds, 100_000_000_000)
    XCTAssertLessThanOrEqual(
      commits.pending(lane: "microphone").bytes,
      25 * RawSegmentCommitWorker.segmentReserveBytes)
    XCTAssertNil(writer.snapshot().failure)
    gate.released.signal()
    try writer.finish()
    XCTAssertEqual(writer.snapshot().sealedFrames, 1_600_000)
    XCTAssertEqual(commits.pending(lane: "microphone").nanoseconds, 0)
    try writer.verifyClosed(expected: writer.applyingSavedAudio(to: TrackStats()))
    for index in 1...25 {
      try verifyAudio(
        root.appendingPathComponent(String(format: "raw-microphone-%08d.caf", index)),
        frames: 32_000)
    }
  }

  func testTwoMinuteBudgetCountsActiveAndInflightAudioAndFailsWithoutClaimingClosure() throws {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    let gate = RawSegmentGate()
    defer { gate.released.signal() }
    let native = RawRecordingSegments.StorageIO()
    let commits = RawSegmentCommitWorker()
    let writer = try RawRecordingSegments(
      output: root, owner: owner, lane: "microphone",
      storageIO: .init(
        write: native.write,
        synchronize: { fd in
          gate.holdOnce()
          try native.synchronize(fd)
        }), commits: commits)
    let quarter = try buffer()
    for index in 0..<480 {
      try writer.enqueue(
        quarter,
        observation: .timestamp(origin + Int64(index) * 250_000_000),
        sourceFrame: Int64(index) * 2000)
      try writer.drainNormalization()
      if index == 16 { XCTAssertEqual(gate.entered.wait(timeout: .now() + 2), .success) }
    }
    XCTAssertEqual(commits.pending(lane: "microphone").nanoseconds, 120_000_000_000)
    XCTAssertThrowsError(
      try writer.enqueue(
        buffer(frames: 1),
        observation: .timestamp(origin + 120_000_000_000), sourceFrame: 960_000))
    XCTAssertEqual(writer.snapshot().sealedFrames, 0)
    XCTAssertFalse(writer.snapshot().producerClosed)
    gate.released.signal()
    XCTAssertThrowsError(try writer.finish())
    XCTAssertEqual(writer.snapshot().failure, "raw_segment_callback_rejected")
    XCTAssertFalse(writer.snapshot().producerClosed)
    XCTAssertEqual(writer.snapshot().sealedFrames, 1_920_000)
    XCTAssertEqual(writer.snapshot().sealedSegments, 30)
    XCTAssertEqual(commits.pending(lane: "microphone").nanoseconds, 0)
    XCTAssertEqual(commits.pending(lane: "microphone").bytes, 0)
    XCTAssertEqual(commits.pending(lane: "microphone").jobs, 0)
    for index in 1...30 {
      try verifyAudio(
        root.appendingPathComponent(String(format: "raw-microphone-%08d.caf", index)),
        frames: 32_000)
    }
    let rescued = try Data(contentsOf: root.appendingPathComponent("raw-microphone.json"))
    XCTAssertEqual(try value(root)["producerClosed"] as? Bool, false)
    XCTAssertThrowsError(try writer.finish())
    XCTAssertThrowsError(
      try writer.verifyClosed(expected: writer.applyingSavedAudio(to: TrackStats())))
    XCTAssertEqual(
      try Data(contentsOf: root.appendingPathComponent("raw-microphone.json")), rescued)
  }

  func testSessionCommitWorkerAlternatesReadyLanesAndPreservesPerLaneOrder() throws {
    let worker = RawSegmentCommitWorker()
    let gate = RawSegmentGate()
    defer { gate.released.signal() }
    let log = RawCommitLog()
    try worker.beginSegment()
    worker.submit(lane: "microphone", bytes: RawSegmentCommitWorker.segmentReserveBytes) {
      gate.holdOnce()
      log.turn("microphone-1")
    }
    XCTAssertEqual(gate.entered.wait(timeout: .now() + 2), .success)
    for value in ["microphone-2", "microphone-3", "system-1"] {
      let lane = value.hasPrefix("system") ? "system" : "microphone"
      try worker.beginSegment()
      worker.submit(lane: lane, bytes: RawSegmentCommitWorker.segmentReserveBytes) {
        log.turn(value)
      }
    }
    gate.released.signal()
    worker.drain(lane: "microphone")
    worker.drain(lane: "system")
    XCTAssertEqual(log.order, ["microphone-1", "system-1", "microphone-2", "microphone-3"])
  }

  func testSlowStorageCannotExceedSixteenMiBOfUncommittedPCM() throws {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    let gate = RawSegmentGate()
    let writer = try RawRecordingSegments(
      output: root, owner: owner, lane: "microphone", beforeAppend: gate.waitOnce)
    defer { gate.released.signal() }
    let large = try buffer(frames: 96_000, rate: 384_000)
    try writer.enqueue(large, observation: .timestamp(origin), sourceFrame: 0)
    XCTAssertEqual(gate.entered.wait(timeout: .now() + 2), .success)
    for index in 1..<21 {
      try writer.enqueue(
        large, observation: .timestamp(origin + Int64(index) * 250_000_000),
        sourceFrame: Int64(index) * 96_000)
    }
    XCTAssertThrowsError(
      try writer.enqueue(
        large,
        observation: .timestamp(origin + 5_250_000_000), sourceFrame: 21 * 96_000))
    gate.released.signal()
    XCTAssertThrowsError(try writer.finish())
    XCTAssertEqual(writer.snapshot().failure, "raw_segment_callback_rejected")
    XCTAssertEqual(try value(root)["sealedSegments"] as? Int, 2)
    XCTAssertEqual(try value(root)["sealedFrames"] as? Int64, 84_000)
    XCTAssertFalse(writer.snapshot().producerClosed)
  }

  func testAggregateRawClockLimitNeverAdvertisesAnUnreadablePrefix() throws {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    let writer = try RawRecordingSegments(
      output: root, owner: owner, lane: "microphone", maximumTotalClockSegments: 200)
    for index in 0..<201 {
      try writer.enqueue(
        buffer(frames: 8), observation: .timestamp(origin + Int64(index) * 2_000_000),
        sourceFrame: Int64(index) * 16)
      try writer.drainAcceptedPCM()
    }
    let confirmed = writer.snapshot()
    XCTAssertGreaterThan(confirmed.sealedFrames, 0)
    let tip = try Data(contentsOf: root.appendingPathComponent("raw-microphone.json"))
    XCTAssertThrowsError(try writer.finish())
    XCTAssertEqual(writer.snapshot().sealedFrames, confirmed.sealedFrames)
    XCTAssertEqual(try Data(contentsOf: root.appendingPathComponent("raw-microphone.json")), tip)
    XCTAssertFalse(writer.snapshot().producerClosed)
    XCTAssertNotNil(writer.snapshot().failure)
    XCTAssertThrowsError(
      try writer.enqueue(
        buffer(frames: 8), observation: .timestamp(origin + 402_000_000), sourceFrame: 1608))
  }

  func testFinalizationCapacityFailureStillCommitsPreviouslyValidatedQueuedSegments() throws {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    let gate = RawSegmentGate()
    defer { gate.released.signal() }
    let native = RawRecordingSegments.StorageIO()
    let commits = RawSegmentCommitWorker()
    let writer = try RawRecordingSegments(
      output: root, owner: owner, lane: "microphone",
      storageIO: .init(
        write: native.write,
        synchronize: { descriptor in
          gate.holdOnce()
          try native.synchronize(descriptor)
        }), maximumTotalClockSegments: 2, commits: commits)
    let quarter = try buffer()
    for index in 0..<33 {
      try writer.enqueue(
        quarter,
        observation: .timestamp(origin + Int64(index) * 250_000_000),
        sourceFrame: Int64(index) * 2000)
      try writer.drainNormalization()
      if index == 16 { XCTAssertEqual(gate.entered.wait(timeout: .now() + 2), .success) }
    }
    // Two immutable full segments are already prevalidated, while the first
    // durable commit is held and a third segment is only being assembled.
    XCTAssertEqual(commits.pending(lane: "microphone").jobs, 3)
    XCTAssertEqual(writer.snapshot().sealedFrames, 0)
    XCTAssertThrowsError(
      try writer.enqueue(
        quarter,
        observation: .timestamp(origin + 9_250_000_000), sourceFrame: 0, sourceEpoch: 0))
    try writer.drainNormalization()
    try writer.enqueue(
      quarter,
      observation: .timestamp(origin + 10_250_000_000), sourceFrame: 0, sourceEpoch: 2)
    try writer.retireSource()
    gate.released.signal()
    XCTAssertThrowsError(try writer.finish())
    XCTAssertNotNil(writer.snapshot().failure)
    XCTAssertEqual(writer.snapshot().sealedSegments, 2)
    XCTAssertEqual(writer.snapshot().sealedFrames, 128_000)
    XCTAssertFalse(writer.snapshot().producerClosed)
    XCTAssertEqual(try value(root)["producerClosed"] as? Bool, false)
    for index in 1...2 {
      try verifyAudio(
        root.appendingPathComponent(String(format: "raw-microphone-%08d.caf", index)),
        frames: 32_000)
    }
    // An invalid, unsaved normalizer tail remains unknown. It is never padded,
    // advertised or used to turn the failed producer into a closed recording.
    XCTAssertFalse(
      FileManager.default.fileExists(
        atPath:
          root.appendingPathComponent("raw-microphone-00000003.json").path))
  }

  func testStorageFullQuotaAndSyncFailureKeepConfirmedPrefixAndRejectFurtherWrites() throws {
    for code in [POSIXErrorCode.ENOSPC, .EDQUOT, .EIO] {
      // One batched CAF write, entry write, then audio/entry/directory fsync.
      for (sync, skip) in [
        (false, 0), (false, 1), (true, 0), (true, 1), (true, 2),
      ] {
        let root = try directory()
        defer { try? FileManager.default.removeItem(at: root) }
        let fault = StorageFault()
        let writer = try RawRecordingSegments(
          output: root, owner: owner, lane: "microphone",
          storageIO: fault.io(code: code, sync: sync))
        try append(writer)
        let names = [
          "raw-microphone.json", "raw-microphone-00000001.json", "raw-microphone-00000001.caf",
        ]
        let originals = try names.map { try Data(contentsOf: root.appendingPathComponent($0)) }
        fault.arm(skipping: skip)
        XCTAssertThrowsError(try writer.finish())
        XCTAssertEqual(writer.snapshot().sealedFrames, 64_000)
        XCTAssertFalse(writer.snapshot().producerClosed)
        XCTAssertEqual(
          writer.snapshot().failure,
          code == .EIO ? "raw_segment_storage_io_failed" : "raw_segment_storage_full")
        let active = root.appendingPathComponent("raw-microphone-00000002.caf")
        let failedBytes = try Data(contentsOf: active)
        XCTAssertThrowsError(
          try writer.enqueue(
            buffer(), observation: .timestamp(origin + 4_500_000_000), sourceFrame: 36_000))
        XCTAssertThrowsError(try writer.finish())
        XCTAssertEqual(try Data(contentsOf: active), failedBytes)
        for (name, original) in zip(names, originals) {
          XCTAssertEqual(try Data(contentsOf: root.appendingPathComponent(name)), original)
        }
        var stats = TrackStats(frames: 34_000, sampleRate: 8000, channels: 2)
        stats.rawSegments = writer.snapshot()
        XCTAssertThrowsError(try stats.requireWritable())
      }
    }
  }

  func testTipStorageFaultsAtSegmentAndClosureKeepAudioAndTerminalStateDespiteVisibleRename() throws
  {
    for code in [POSIXErrorCode.ENOSPC, .EDQUOT, .EIO] {
      for point in TipStorageFault.Point.allCases {
        for closure in [false, true] {
          let root = try directory()
          defer { try? FileManager.default.removeItem(at: root) }
          let fault = TipStorageFault()
          let writer = try RawRecordingSegments(
            output: root, owner: owner, lane: "microphone", tipStorageIO: fault.io())
          try append(writer)
          let tipPath = root.appendingPathComponent("raw-microphone.json")
          let beforeTip = try Data(contentsOf: tipPath)
          let confirmed = ["raw-microphone-00000001.caf", "raw-microphone-00000001.json"]
          let beforeAudio = try confirmed.map {
            try Data(contentsOf: root.appendingPathComponent($0))
          }
          // finish first seals the tail with producerClosed=false, then writes
          // the closed tip. Skip exactly one matched operation for closure.
          fault.arm(point, code: code, skipping: closure ? 1 : 0)
          let failureCode =
            code == .EIO ? "raw_segment_storage_io_failed" : "raw_segment_storage_full"
          XCTAssertThrowsError(try writer.finish()) {
            XCTAssertEqual(String(describing: $0), CaptureFailureGuide.message(failureCode))
          }
          let snapshot = writer.snapshot()
          XCTAssertEqual(snapshot.failure, failureCode)
          XCTAssertFalse(snapshot.producerClosed)
          XCTAssertEqual(snapshot.sealedFrames, closure ? 68_000 : 64_000)
          let visible = try value(root)
          XCTAssertEqual(
            visible["sealedFrames"] as? Int, closure || point == .directorySync ? 68_000 : 64_000)
          XCTAssertEqual(visible["producerClosed"] as? Bool, closure && point == .directorySync)
          if !closure && point != .directorySync {
            XCTAssertEqual(try Data(contentsOf: tipPath), beforeTip)
          }
          for (name, bytes) in zip(confirmed, beforeAudio) {
            XCTAssertEqual(try Data(contentsOf: root.appendingPathComponent(name)), bytes)
          }
          try verifyAudio(
            root.appendingPathComponent("raw-microphone-00000001.caf"), frames: 32_000)
          // Clearing an injected syscall failure must not clear the native
          // writer's terminal failure or retroactively confirm the visible tip.
          let retained = try FileManager.default.contentsOfDirectory(atPath: root.path).sorted()
          let retainedBytes = try retained.map {
            try Data(contentsOf: root.appendingPathComponent($0))
          }
          fault.disarm()
          XCTAssertThrowsError(try writer.finish())
          XCTAssertThrowsError(
            try writer.enqueue(
              buffer(), observation: .timestamp(origin + 4_250_000_000), sourceFrame: 34_000))
          XCTAssertEqual(writer.snapshot().failure, failureCode)
          XCTAssertFalse(writer.snapshot().producerClosed)
          XCTAssertEqual(
            try FileManager.default.contentsOfDirectory(atPath: root.path).sorted(), retained)
          for (name, bytes) in zip(retained, retainedBytes) {
            XCTAssertEqual(try Data(contentsOf: root.appendingPathComponent(name)), bytes)
          }
        }
      }
    }
  }

  func testInitialTipStorageCauseIsLocalizedWithoutCreatingRecoveryOrOverwritingExistingAudio()
    throws
  {
    for code in [POSIXErrorCode.ENOSPC, .EDQUOT, .EIO, .EACCES, .EEXIST] {
      let root = try directory()
      defer { try? FileManager.default.removeItem(at: root) }
      let marker = Data("existing audio is not a cache".utf8)
      let audio = root.appendingPathComponent("unowned.caf")
      try marker.write(to: audio)
      let fault = TipStorageFault()
      fault.arm(.create, code: code)
      XCTAssertThrowsError(
        try RawRecordingSegments(
          output: root, owner: owner, lane: "microphone", tipStorageIO: fault.io())
      ) {
        if [.ENOSPC, .EDQUOT, .EIO].contains(code) {
          let failureCode =
            code == .EIO ? "raw_segment_storage_io_failed" : "raw_segment_storage_full"
          XCTAssertEqual(String(describing: $0), CaptureFailureGuide.message(failureCode))
        } else {
          XCTAssertEqual(($0 as? POSIXError)?.code, code)
        }
      }
      XCTAssertEqual(try Data(contentsOf: audio), marker)
      XCTAssertEqual(
        try FileManager.default.contentsOfDirectory(atPath: root.path), ["unowned.caf"])
    }
  }

  func testSystemCAFStopsAfterAsynchronousStorageFailureWithoutGrowingConfirmedPrefix() async throws
  {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    let fault = StorageFault()
    let raw = try RawRecordingSegments(
      output: root, owner: owner, lane: "system", storageIO: fault.io(code: .ENOSPC))
    let writer = SystemPCMWriter(
      outputURL: root.appendingPathComponent("system.caf"), rawSegments: raw)
    let fixture = CaptureClockFixtureTests()
    for index in 0..<33 {
      if index == 32 { fault.arm() }
      try writer.append(
        fixture.sample(
          frames: 12000, value: 0.25, presentation: .zero, rightValue: -0.5),
        observation: .timestamp(origin + Int64(index) * 250_000_000), sourceEpoch: 0,
        levelsEnabled: true, receivedAt: origin)
      if index < 32 { try raw.drainAcceptedPCM() }
    }
    XCTAssertThrowsError(try raw.drainAcceptedPCM())
    let frames = writer.stats.frames
    XCTAssertGreaterThanOrEqual(frames, 128_000)
    XCTAssertLessThan(frames, 132_000)
    XCTAssertEqual(writer.stats.rawSegments?.sealedFrames, 64_000)
    XCTAssertEqual(writer.stats.captureFailure, "raw_segment_storage_full")
    XCTAssertFalse(writer.ready)
    for _ in 0..<3 {
      XCTAssertThrowsError(
        try writer.append(
          fixture.sample(frames: 12000, value: 0.75, presentation: .zero),
          observation: .timestamp(origin + 4_500_000_000), sourceEpoch: 0,
          levelsEnabled: true, receivedAt: origin))
    }
    XCTAssertEqual(writer.stats.frames, frames)
    do {
      try await writer.finish()
      XCTFail("storage failure cannot finalize successfully")
    } catch {}
    XCTAssertFalse(try value(root, "raw-system.json")["producerClosed"] as? Bool ?? true)
    try verifyAudio(
      root.appendingPathComponent("raw-system-00000001.caf"), frames: 192_000, rate: 48000)
  }

  func testSystemCAFRejectsClockCapacityBeforeUnmappedPCMAndCannotResume() async throws {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    let writer = SystemPCMWriter(outputURL: root.appendingPathComponent("system.caf"))
    let fixture = CaptureClockFixtureTests()
    for index in 0..<1024 {
      try writer.append(
        fixture.sample(frames: 480, value: 0.25, presentation: .zero, rightValue: -0.5),
        observation: .timestamp(origin + Int64(index) * 20_000_000), sourceEpoch: 0,
        levelsEnabled: false, receivedAt: nil)
    }
    let frames = writer.stats.frames
    XCTAssertEqual(frames, 491_520)
    for index in 1024..<1027 {
      XCTAssertThrowsError(
        try writer.append(
          fixture.sample(frames: 480, value: 0.75, presentation: .zero),
          observation: .timestamp(origin + Int64(index) * 20_000_000), sourceEpoch: 0,
          levelsEnabled: false, receivedAt: nil))
      XCTAssertEqual(writer.stats.frames, frames)
    }
    XCTAssertNotNil(writer.stats.clockFailure)
    XCTAssertFalse(writer.ready)
    var stats = writer.stats
    XCTAssertThrowsError(try stats.finishClock(originNanoseconds: origin))
    XCTAssertNil(stats.clockSegments)
    do {
      try await writer.finish()
      XCTFail("clock failure cannot finalize successfully")
    } catch {}
    try verifyAudio(root.appendingPathComponent("system.caf"), frames: frames, rate: 48000)
  }

  func testSIGKILLKeepsOnlyAdvertisedPrefixWithoutClaimingProducerOrAcquisitionClosed() throws {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    let child = Process()
    child.executableURL = URL(fileURLWithPath: "/usr/bin/xcrun")
    child.arguments = [
      "xctest", "-XCTest", "MONMacOSCaptureTests.RawRecordingSegmentTests/testCrashChild",
      Bundle(for: RawRecordingSegmentTests.self).bundleURL.path,
    ]
    child.environment = ProcessInfo.processInfo.environment.merging([
      "MON_RAW_SEGMENT_CRASH_CHILD": root.path
    ]) { _, child in child }
    child.standardOutput = FileHandle.nullDevice
    child.standardError = FileHandle.nullDevice
    try child.run()
    defer {
      if child.isRunning { _ = kill(child.processIdentifier, SIGKILL) }
      child.waitUntilExit()
    }
    let ready = root.appendingPathComponent("ready")
    let deadline = ContinuousClock.now.advanced(by: .seconds(10))
    while child.isRunning, !FileManager.default.fileExists(atPath: ready.path),
      ContinuousClock.now < deadline
    { Thread.sleep(forTimeInterval: 0.01) }
    guard child.isRunning, FileManager.default.fileExists(atPath: ready.path) else {
      XCTFail("Synthetic raw writer did not reach its committed prefix")
      return
    }
    let committed = try Data(contentsOf: root.appendingPathComponent("raw-microphone.json"))
    XCTAssertEqual(kill(child.processIdentifier, SIGKILL), 0)
    child.waitUntilExit()
    XCTAssertEqual(child.terminationReason, .uncaughtSignal)
    XCTAssertEqual(child.terminationStatus, SIGKILL)
    XCTAssertEqual(
      try Data(contentsOf: root.appendingPathComponent("raw-microphone.json")), committed)
    XCTAssertEqual(try value(root)["sealedSegments"] as? Int, 1)
    XCTAssertEqual(try value(root)["sealedFrames"] as? Int, 64_000)
    XCTAssertEqual(try value(root)["producerClosed"] as? Bool, false)
    XCTAssertNil(try value(root)["acquisitionClosed"])
    try verifyAudio(root.appendingPathComponent("raw-microphone-00000001.caf"), frames: 32_000)
    XCTAssertFalse(
      FileManager.default.fileExists(
        atPath: root.appendingPathComponent("raw-microphone-00000002.json").path))
  }

  func testCrashChild() throws {
    guard let path = ProcessInfo.processInfo.environment["MON_RAW_SEGMENT_CRASH_CHILD"] else {
      throw XCTSkip("Only the parent executes this synthetic PCM child; no capture devices")
    }
    let root = URL(fileURLWithPath: path, isDirectory: true)
    let writer = try RawRecordingSegments(output: root, owner: owner, lane: "microphone")
    try append(writer)
    try waitForPrefix(writer)
    try Data("sealed 32000; active 2000; not closed".utf8).write(
      to: root.appendingPathComponent("ready"))
    while true { Thread.sleep(forTimeInterval: 0.05) }
  }

  func testNativePrefixFixtureExportsBoundManifestForRustWithoutCaptureDevices() async throws {
    try await nativePrefixFixture(
      rebased: ProcessInfo.processInfo.environment["MON_RAW_SEGMENT_FIXTURE_REBASE"] == "1",
      exports: true)
  }

  func testRebasedNativePrefixFixtureKeepsManifestAndRawReceiptsCompatible() async throws {
    try await nativePrefixFixture(rebased: true, exports: false)
  }

  private func nativePrefixFixture(rebased: Bool, exports: Bool) async throws {
    let environment = ProcessInfo.processInfo.environment
    let external = exports ? environment["MON_RAW_SEGMENT_FIXTURE_DIR"] : nil
    let root = try external.map { URL(fileURLWithPath: $0, isDirectory: true) } ?? directory()
    defer { if external == nil { try? FileManager.default.removeItem(at: root) } }
    let recordingOwner = environment["MON_RAW_SEGMENT_FIXTURE_OWNER"] ?? owner
    let dense = environment["MON_RAW_SEGMENT_FIXTURE_DENSE"] == "1"
    let reference = { (time: Int64) in
      CaptureClockReading(
        hostBeforeNs: time, continuousNs: time + 10,
        utcMs: 1_700_000_000_000, hostAfterNs: time + 20)
    }
    let readings = ReadingBox(reference(origin))
    let clock = try CaptureClockDomain(units: MachTimeUnits(numerator: 1, denominator: 1)) {
      readings.read()
    }
    readings.set(reference(origin + (dense ? 42_000_000_000 : 5_000_000_000)))
    let fixture = CaptureClockFixtureTests()
    var tracks: [TrackStats] = []
    for lane in ["microphone", "system"] {
      let raw = try RawRecordingSegments(
        output: root, owner: recordingOwner, lane: lane, allowsClockRebasing: rebased)
      let writer = SystemPCMWriter(
        outputURL: root.appendingPathComponent("\(lane).caf"), rawSegments: raw)
      for index in 0..<(dense ? 2048 : 17) {
        let sample = try fixture.sample(
          frames: dense ? 480 : 12000, value: 0.25,
          presentation: CMTime(value: Int64(index), timescale: 4), rightValue: -0.5)
        try writer.append(
          sample,
          observation: clock.observe(
            hostTicks: UInt64(
              origin + Int64(index) * (dense ? 20_000_000 : 250_000_000)
                - (rebased && index >= 8 ? (dense ? 100_000_000 : 500_000_000) : 0))),
          sourceEpoch: 0, levelsEnabled: false, receivedAt: nil)
        try raw.drainAcceptedPCM()
      }
      try await writer.finish()
      tracks.append(writer.stats)
    }
    let micClosure = try clock.close(.microphone)
    let systemClosure = try clock.close(.system)
    var manifest = Manifest(
      schemaVersion: 4, scenario: "synthetic-raw-prefix", captureMode: "system",
      backend: "screencapturekit", targetPID: nil, targetPIDs: [], startedAtUTC: "synthetic",
      requestedDurationSeconds: nil, osVersion: "synthetic", architecture: "arm64", state: "saved",
      microphone: tracks[0], system: tracks[1], errors: [], recordingOwnerId: recordingOwner)
    try manifest.finalizeStoppedClock(
      clock, microphoneClosure: micClosure, systemClosure: systemClosure)
    try manifest.encodedData().write(to: root.appendingPathComponent("manifest.json"))
    XCTAssertEqual(manifest.schemaVersion, 4)
    XCTAssertEqual(manifest.timingEstimated, rebased ? true : nil)
    XCTAssertEqual(manifest.microphone.rawSegments?.sealedFrames, dense ? 327_680 : 68_000)
    XCTAssertNotNil(manifest.microphone.rawSegments?.lastEntrySHA256)
    if dense {
      XCTAssertEqual(manifest.microphone.clockSegments?.count, 2048)
      XCTAssertGreaterThan(try manifest.encodedData().count, 256 * 1024)
      XCTAssertGreaterThan(manifest.microphone.rawSegments?.sealedSegments ?? 0, 6)
      for lane in ["microphone", "system"] {
        for sequence in 1...(tracks[lane == "microphone" ? 0 : 1].rawSegments?.sealedSegments ?? 0)
        {
          let name = String(format: "raw-%@-%08d.json", lane, sequence)
          let entry = try value(root, name)
          XCTAssertLessThanOrEqual(
            (entry["clockSegments"] as? [Any])?.count ?? Int.max,
            RawRecordingSegments.maximumClockSegments)
          XCTAssertLessThanOrEqual(
            try Data(contentsOf: root.appendingPathComponent(name)).count, 65 * 1024)
        }
      }
    }
  }

  private func hex<D: Sequence>(_ digest: D) -> String where D.Element == UInt8 {
    digest.map { String(format: "%02x", $0) }.joined()
  }
}
