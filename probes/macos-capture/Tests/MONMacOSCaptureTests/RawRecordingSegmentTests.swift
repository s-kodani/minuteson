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

  private func hex<D: Sequence>(_ digest: D) -> String where D.Element == UInt8 {
    digest.map { String(format: "%02x", $0) }.joined()
  }
}
