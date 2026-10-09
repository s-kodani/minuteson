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
  func wait() {
    entered.signal()
    _ = released.wait(timeout: .now() + 5)
  }
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
    XCTAssertEqual(file.length, frames)
    XCTAssertEqual(file.fileFormat.sampleRate, rate)
    XCTAssertEqual(file.fileFormat.channelCount, 2)
    let output = try XCTUnwrap(
      AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: 4096))
    var read: Int64 = 0
    while read < frames {
      try file.read(into: output, frameCount: AVAudioFrameCount(min(4096, frames - read)))
      XCTAssertGreaterThan(output.frameLength, 0)
      let values = try XCTUnwrap(output.floatChannelData)
      for index in 0..<Int(output.frameLength) {
        XCTAssertEqual(values[0][index], 0.25)
        XCTAssertEqual(values[1][index], -0.5)
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
      XCTAssertEqual(tip["sealedFrames"] as? Int64, 32_000)
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
      XCTAssertEqual(maps[0]["sourceFrameEnd"] as? Int, 32_000)
      XCTAssertEqual(maps[0]["timelineEndUs"] as? Int, 4_000_000)
      let firstEntry = try Data(
        contentsOf: root.appendingPathComponent("raw-microphone-00000001.json"))
      XCTAssertEqual(tip["lastEntrySHA256"] as? String, hex(SHA256.hash(data: firstEntry)))
      try writer.finish()
      XCTAssertEqual(writer.snapshot().sealedSegments, 2)
      XCTAssertEqual(writer.snapshot().sealedFrames, 34_000)
      XCTAssertTrue(writer.snapshot().producerClosed)
      try writer.finish()
      XCTAssertEqual(try Data(contentsOf: root.appendingPathComponent(firstName)), firstData)
      try verifyAudio(root.appendingPathComponent("raw-microphone-00000002.caf"), frames: 2000)
      let second = try value(root, "raw-microphone-00000002.json")
      XCTAssertEqual(second["sourceFrameStart"] as? Int, 32_000)
      XCTAssertEqual(second["previousEntrySHA256"] as? String, hex(SHA256.hash(data: firstEntry)))
      XCTAssertEqual(second["captureOriginNanoseconds"] as? Int64, origin + 4_000_000_000)
      XCTAssertNil(second["acquisitionClosed"])
      XCTAssertEqual(try value(root)["producerClosed"] as? Bool, true)
    }
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
    XCTAssertEqual(tail[0]["sourceFrameEnd"] as? Int, 2000)
    XCTAssertEqual(tail[1]["sourceFrameStart"] as? Int, 2000)
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
    XCTAssertEqual(try value(moved)["sealedFrames"] as? Int, 34_000)
  }

  func testSourceCounterResetAndFormatChangePreserveConfirmedPrefixAndRejectSuccess() throws {
    for format in [false, true] {
      let root = try directory()
      defer { try? FileManager.default.removeItem(at: root) }
      let writer = try RawRecordingSegments(output: root, owner: owner, lane: "microphone")
      try append(writer)
      try waitForPrefix(writer)
      let original = try Data(contentsOf: root.appendingPathComponent("raw-microphone.json"))
      try writer.enqueue(
        buffer(rate: format ? 16000 : 8000), observation: .timestamp(origin + 4_250_000_000),
        sourceFrame: format ? 34_000 : 0)
      XCTAssertThrowsError(try writer.finish())
      XCTAssertEqual(
        try Data(contentsOf: root.appendingPathComponent("raw-microphone.json")), original)
      XCTAssertFalse(writer.snapshot().producerClosed)
      XCTAssertNotNil(writer.snapshot().failure)
    }
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
    XCTAssertEqual(writer.stats.rawSegments?.sealedFrames, 192_000)
    try await writer.finish()
    XCTAssertEqual(writer.stats.frames, 204_000)
    XCTAssertEqual(writer.stats.rawSegments?.sealedFrames, 204_000)
    XCTAssertEqual(writer.stats.rawSegments?.producerClosed, true)
    try verifyAudio(root.appendingPathComponent("system.caf"), frames: 204_000, rate: 48000)
    try verifyAudio(
      root.appendingPathComponent("raw-system-00000001.caf"), frames: 192_000, rate: 48000)
    try verifyAudio(
      root.appendingPathComponent("raw-system-00000002.caf"), frames: 12000, rate: 48000)
  }

  func testSlowStorageCannotAccumulateMoreThanHalfASecondOfUncommittedPCM() throws {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    let gate = RawSegmentGate()
    let writer = try RawRecordingSegments(
      output: root, owner: owner, lane: "microphone", beforeAppend: { gate.wait() })
    try writer.enqueue(buffer(), observation: .timestamp(origin), sourceFrame: 0)
    XCTAssertEqual(gate.entered.wait(timeout: .now() + 2), .success)
    try writer.enqueue(
      buffer(), observation: .timestamp(origin + 250_000_000), sourceFrame: 2000)
    XCTAssertThrowsError(
      try writer.enqueue(
        buffer(), observation: .timestamp(origin + 500_000_000), sourceFrame: 4000))
    gate.released.signal()
    gate.released.signal()
    XCTAssertThrowsError(try writer.finish())
    XCTAssertEqual(writer.snapshot().failure, "raw_segment_callback_rejected")
    XCTAssertEqual(try value(root)["sealedSegments"] as? Int, 0)
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

  func testStorageFullQuotaAndSyncFailureKeepConfirmedPrefixAndRejectFurtherWrites() throws {
    for code in [POSIXErrorCode.ENOSPC, .EDQUOT, .EIO] {
      // PCM/header/entry writes, followed by audio/entry/directory fsync.
      for (sync, skip, appendPCM) in [
        (false, 0, true), (false, 0, false), (false, 1, false),
        (true, 0, false), (true, 1, false), (true, 2, false),
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
        if appendPCM {
          try writer.enqueue(
            buffer(), observation: .timestamp(origin + 4_250_000_000), sourceFrame: 34_000)
          XCTAssertThrowsError(try writer.drainAcceptedPCM())
        }
        XCTAssertThrowsError(try writer.finish())
        XCTAssertEqual(writer.snapshot().sealedFrames, 32_000)
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
    for index in 0..<18 {
      if index == 17 { fault.arm() }
      try writer.append(
        fixture.sample(
          frames: 12000, value: 0.25, presentation: .zero, rightValue: -0.5),
        observation: .timestamp(origin + Int64(index) * 250_000_000), sourceEpoch: 0,
        levelsEnabled: true, receivedAt: origin)
      if index < 17 { try raw.drainAcceptedPCM() }
    }
    XCTAssertThrowsError(try raw.drainAcceptedPCM())
    let frames = writer.stats.frames
    XCTAssertEqual(frames, 216_000)
    XCTAssertEqual(writer.stats.rawSegments?.sealedFrames, 192_000)
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
    try verifyAudio(root.appendingPathComponent("system.caf"), frames: frames, rate: 48000)
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
    XCTAssertEqual(try value(root)["sealedFrames"] as? Int, 32_000)
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
    let environment = ProcessInfo.processInfo.environment
    let external = environment["MON_RAW_SEGMENT_FIXTURE_DIR"]
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
      let raw = try RawRecordingSegments(output: root, owner: recordingOwner, lane: lane)
      let writer = SystemPCMWriter(
        outputURL: root.appendingPathComponent("\(lane).caf"), rawSegments: raw)
      for index in 0..<(dense ? 2048 : 17) {
        let sample = try fixture.sample(
          frames: dense ? 480 : 12000, value: 0.25,
          presentation: CMTime(value: Int64(index), timescale: 4), rightValue: -0.5)
        try writer.append(
          sample,
          observation: clock.observe(
            hostTicks: UInt64(origin + Int64(index) * (dense ? 20_000_000 : 250_000_000))),
          sourceEpoch: 0, levelsEnabled: false, receivedAt: nil)
        try raw.drainAcceptedPCM()
      }
      try await writer.finish()
      tracks.append(writer.stats)
    }
    let micClosure = try clock.close(.microphone)
    let systemClosure = try clock.close(.system)
    var manifest = Manifest(
      schemaVersion: 2, scenario: "synthetic-raw-prefix", captureMode: "system",
      backend: "screencapturekit", targetPID: nil, targetPIDs: [], startedAtUTC: "synthetic",
      requestedDurationSeconds: nil, osVersion: "synthetic", architecture: "arm64", state: "saved",
      microphone: tracks[0], system: tracks[1], errors: [], recordingOwnerId: recordingOwner)
    try manifest.finalizeStoppedClock(
      clock, microphoneClosure: micClosure, systemClosure: systemClosure)
    try manifest.encodedData().write(to: root.appendingPathComponent("manifest.json"))
    XCTAssertEqual(manifest.schemaVersion, 3)
    XCTAssertEqual(manifest.microphone.rawSegments?.sealedFrames, dense ? 983_040 : 204_000)
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
