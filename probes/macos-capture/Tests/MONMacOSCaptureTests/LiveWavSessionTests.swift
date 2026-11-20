import AVFoundation
import CryptoKit
import Darwin
import Foundation
import XCTest

@testable import MONMacOSCapture

private final class WavClockReading: @unchecked Sendable {
  private let lock = NSLock()
  private var time: Int64 = 1_000_000_000
  func advance() { lock.withLock { time = 3_000_000_000 } }
  func read() -> CaptureClockReading {
    lock.withLock {
      CaptureClockReading(
        hostBeforeNs: time, continuousNs: time, utcMs: 1_800_000_000_000,
        hostAfterNs: time)
    }
  }
}

final class LiveWavSessionTests: XCTestCase {
  private func directory() throws -> URL {
    let root = FileManager.default.temporaryDirectory
      .appendingPathComponent(UUID().uuidString.lowercased())
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false)
    let path = try XCTUnwrap(realpath(root.path, nil))
    defer { free(path) }
    return URL(fileURLWithPath: String(cString: path), isDirectory: true)
  }
  private func append(
    _ writer: LiveWavSession, lane: String, value: Int16, frames: Int, start: Int64
  ) throws {
    for first in stride(from: 0, to: frames, by: 4000) {
      let count = min(4000, frames - first)
      try writer.append(
        lane: lane, epoch: 0, pcm: [Int16](repeating: value, count: count),
        start: start + Int64(first) * 1_000_000_000 / 16000,
        end: start + Int64(first + count) * 1_000_000_000 / 16000, acquiredNanoseconds: 0)
    }
  }

  func testSingleWavPublishesVerifiedMixedPrefixAndTrailingGap() throws {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    let owner = UUID().uuidString.lowercased()
    let writer = try LiveWavSession(output: root, owner: owner, commits: RawSegmentCommitWorker())
    try append(writer, lane: "microphone", value: 3200, frames: 16000, start: 1_000_000_000)
    try append(writer, lane: "system", value: 9600, frames: 16000, start: 1_000_000_000)
    try writer.closeLane("microphone")
    try writer.closeLane("system")
    try writer.finish(endNanoseconds: 3_000_000_000)
    let saved = writer.snapshot()
    XCTAssertEqual(saved.frames, 32000)
    XCTAssertEqual(saved.checkpoints, 1)
    XCTAssertTrue(saved.producerClosed)
    let data = try Data(contentsOf: root.appendingPathComponent("recording.wav"))
    XCTAssertEqual(data.count, 80 + 32000 * 2)
    XCTAssertEqual(data.prefix(4), Data("RIFF".utf8))
    XCTAssertEqual(
      Int16(littleEndian: data[80..<82].withUnsafeBytes { $0.loadUnaligned(as: Int16.self) }), 6400)
    XCTAssertTrue(data[(80 + 16000 * 2)...].allSatisfy { $0 == 0 })
    let entryData = try Data(contentsOf: root.appendingPathComponent("wav-00000001.json"))
    let entry = try XCTUnwrap(JSONSerialization.jsonObject(with: entryData) as? [String: Any])
    XCTAssertEqual(entry["missingRanges"] as? [[Int]], [[16000, 32000]])
    XCTAssertEqual(
      saved.lastEntrySHA256,
      SHA256.hash(data: entryData).map { String(format: "%02x", $0) }.joined())
    let names = try FileManager.default.contentsOfDirectory(atPath: root.path)
    XCTAssertFalse(names.contains { $0.hasSuffix(".caf") || $0.hasSuffix(".flac") })
    XCTAssertTrue(names.contains("single-wav-owner.json"))
  }

  func testTestInputUsesOnlyItsShortWavAndNoRecoveryJournal() throws {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    let writer = try LiveWavSession(
      output: root, owner: UUID().uuidString.lowercased(), commits: RawSegmentCommitWorker(),
      fileName: "microphone.wav", onlyLane: 0)
    try append(writer, lane: "microphone", value: -1200, frames: 8000, start: 2_000_000_000)
    try writer.closeLane("microphone")
    try writer.finish()
    let names = try FileManager.default.contentsOfDirectory(atPath: root.path)
    XCTAssertEqual(names, ["microphone.wav"])
    let data = try Data(contentsOf: root.appendingPathComponent("microphone.wav"))
    XCTAssertEqual(data.count, 80 + 8000 * 2)
    XCTAssertEqual(
      Int16(littleEndian: data[80..<82].withUnsafeBytes { $0.loadUnaligned(as: Int16.self) }), -1200
    )
  }

  func testReplacingDestinationDoesNotOverwriteReplacement() throws {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    let writer = try LiveWavSession(
      output: root, owner: UUID().uuidString.lowercased(), commits: RawSegmentCommitWorker())
    let destination = root.appendingPathComponent("recording.wav")
    try FileManager.default.moveItem(
      at: destination, to: root.appendingPathComponent("original.wav"))
    let replacement = Data("retain unrelated replacement".utf8)
    try replacement.write(to: destination)
    try writer.append(
      lane: "microphone", epoch: 0, pcm: [Int16](repeating: 10, count: 1600),
      start: 1_000_000_000, end: 1_100_000_000, acquiredNanoseconds: 0)
    try writer.closeLane("microphone")
    try writer.closeLane("system")
    XCTAssertThrowsError(try writer.finish())
    XCTAssertEqual(try Data(contentsOf: destination), replacement)
  }

  func testUnsupportedRequestedCaptureFormatFailsDuringParsing() {
    XCTAssertThrowsError(try Options.parse(["--output", "/tmp/unused", "--capture-format", "caf"]))
  }

  func testNormalizersWriteOneWavWithoutRawAudioOrRawTips() throws {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    let owner = UUID().uuidString.lowercased()
    let commits = RawSegmentCommitWorker()
    let writer = try LiveWavSession(output: root, owner: owner, commits: commits)
    let mic = try RawRecordingSegments(
      output: root, owner: owner, lane: "microphone", commits: commits, wavSession: writer)
    let pc = try RawRecordingSegments(
      output: root, owner: owner, lane: "system", commits: commits, wavSession: writer)
    let format = try XCTUnwrap(
      AVAudioFormat(
        commonFormat: .pcmFormatFloat32, sampleRate: 48000, channels: 1, interleaved: false))
    let buffer = try XCTUnwrap(AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 48000))
    buffer.frameLength = 48000
    let samples = try XCTUnwrap(buffer.floatChannelData)
    for i in 0..<48000 { samples[0][i] = 0.25 }
    try mic.enqueue(buffer, observation: .timestamp(1_000_000_000), sourceFrame: 0)
    try pc.enqueue(buffer, observation: .timestamp(1_000_000_000), sourceFrame: nil)
    try mic.finish()
    try pc.finish()
    try writer.finish(endNanoseconds: 2_000_000_000)
    let micStats = mic.applyingSavedAudio(to: TrackStats())
    let pcStats = pc.applyingSavedAudio(to: TrackStats())
    XCTAssertEqual(micStats.frames, 16000)
    XCTAssertEqual(pcStats.frames, 16000)
    XCTAssertNil(micStats.rawSegments)
    XCTAssertNil(pcStats.rawSegments)
    XCTAssertEqual(writer.snapshot().frames, 16000)
    XCTAssertTrue(writer.snapshot().producerClosed)
    let names = try FileManager.default.contentsOfDirectory(atPath: root.path)
    XCTAssertFalse(names.contains { $0.hasPrefix("raw-") || $0.hasSuffix(".caf") })
  }

  func testIdleObservationCannotOvertakeAdmittedNormalization() throws {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    let owner = UUID().uuidString.lowercased()
    let commits = RawSegmentCommitWorker()
    let writer = try LiveWavSession(output: root, owner: owner, commits: commits)
    let entered = DispatchSemaphore(value: 0)
    let released = DispatchSemaphore(value: 0)
    defer { released.signal() }
    let mic = try RawRecordingSegments(
      output: root, owner: owner, lane: "microphone",
      beforeAppend: {
        entered.signal()
        _ = released.wait(timeout: .now() + 5)
      }, commits: commits, wavSession: writer)
    let pc = try RawRecordingSegments(
      output: root, owner: owner, lane: "system", commits: commits, wavSession: writer)
    let format = try XCTUnwrap(
      AVAudioFormat(
        commonFormat: .pcmFormatFloat32, sampleRate: 16000, channels: 1, interleaved: false))
    let buffer = try XCTUnwrap(AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 16000))
    buffer.frameLength = 16000
    let samples = try XCTUnwrap(buffer.floatChannelData)
    for index in 0..<16000 { samples[0][index] = 0.25 }
    let start = try XCTUnwrap(PCMInputLevel.nowNanoseconds())
    try mic.enqueue(buffer, observation: .timestamp(start), sourceFrame: 0)
    XCTAssertEqual(entered.wait(timeout: .now() + 2), .success)
    mic.observeUnreceived(at: start + 2_000_000_000)
    pc.observeUnreceived(at: start + 2_000_000_000)
    try pc.drainNormalization()
    XCTAssertEqual(writer.snapshot().frames, 0)
    released.signal()
    try mic.finish()
    try pc.finish()
    try writer.finish(endNanoseconds: start + 2_000_000_000)
    XCTAssertEqual(writer.snapshot().frames, 32000)
    let data = try Data(contentsOf: root.appendingPathComponent("recording.wav"))
    let value = data[80..<82].withUnsafeBytes { $0.loadUnaligned(as: Int16.self) }
    XCTAssertEqual(Int16(littleEndian: value), 8192)
  }

  /// Root Rust tests provide the already-owned recording directory. Keep the
  /// exact inode and birth time; copying this fixture would invalidate proof.
  func testExportSingleWavFixture() throws {
    let environment = ProcessInfo.processInfo.environment
    guard let path = environment["MON_SINGLE_WAV_FIXTURE_DIR"],
      let owner = environment["MON_SINGLE_WAV_OWNER"]
    else { throw XCTSkip("requires a Rust-owned single-WAV recording directory") }
    let root = URL(fileURLWithPath: path, isDirectory: true)
    let commits = RawSegmentCommitWorker()
    let writer = try LiveWavSession(output: root, owner: owner, commits: commits)
    let readings = WavClockReading()
    let clock = try CaptureClockDomain(units: MachTimeUnits(numerator: 1, denominator: 1)) {
      readings.read()
    }
    var tracks: [TrackStats] = []
    for lane in ["microphone", "system"] {
      let source = try RawRecordingSegments(
        output: root, owner: owner, lane: lane, commits: commits, wavSession: writer)
      let format = try XCTUnwrap(
        AVAudioFormat(
          commonFormat: .pcmFormatFloat32, sampleRate: 16000, channels: 1, interleaved: false))
      let buffer = try XCTUnwrap(AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 16000))
      buffer.frameLength = 16000
      let samples = try XCTUnwrap(buffer.floatChannelData)
      for index in 0..<16000 { samples[0][index] = lane == "microphone" ? 0.25 : 0.5 }
      try source.enqueue(buffer, observation: .timestamp(1_000_000_000), sourceFrame: nil)
      try source.finish()
      var original = TrackStats()
      original.callbackCount = 1
      var stats = source.applyingSavedAudio(to: original)
      let stopped = MicrophoneRecovery.Status(
        state: "stopped", sourceEpoch: 0, interruptions: 0, reason: nil)
      if lane == "microphone" {
        stats.microphoneRecovery = stopped
      } else {
        stats.systemRecovery = stopped
      }
      tracks.append(stats)
    }
    readings.advance()
    let micClosure = try clock.close(.microphone)
    let systemClosure = try clock.close(.system)
    var manifest = Manifest(
      schemaVersion: 5, scenario: "synthetic-single-wav", captureMode: "system",
      backend: environment["MON_SINGLE_WAV_BACKEND"] ?? "coreaudio", targetPID: nil,
      targetPIDs: [], startedAtUTC: "synthetic", requestedDurationSeconds: nil,
      osVersion: "synthetic", architecture: "arm64", state: "saved", microphone: tracks[0],
      system: tracks[1], errors: [], recordingOwnerId: owner)
    try manifest.finalizeStoppedClock(
      clock, microphoneClosure: micClosure, systemClosure: systemClosure)
    try writer.finish(endNanoseconds: 3_000_000_000)
    manifest.singleWav = writer.snapshot()
    let manifestFile = try CaptureSession.createManifest(manifest, output: root)
    try manifestFile.verifyCurrent(manifest.encodedData())
    for name in [
      "recording.wav", "manifest.json", "single-wav-owner.json", "single-wav.json",
      "wav-00000001.json",
    ] {
      var metadata = stat()
      XCTAssertEqual(lstat(root.appendingPathComponent(name).path, &metadata), 0)
      XCTAssertEqual(metadata.st_mode & 0o777, 0o600, name)
    }
    XCTAssertEqual(writer.snapshot().frames, 32000)
    XCTAssertTrue(writer.snapshot().producerClosed)
  }
}
