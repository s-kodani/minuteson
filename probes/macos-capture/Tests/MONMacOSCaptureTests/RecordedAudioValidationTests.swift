import AVFoundation
import Foundation
import XCTest

@testable import MONMacOSCapture

final class RecordedAudioValidationTests: XCTestCase {
  private func directory() throws -> URL {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false)
    return root
  }

  private func audio(_ root: URL, name: String = "track.caf", frames: UInt32 = 4801) throws
    -> URL
  {
    let url = root.appendingPathComponent(name)
    let format = try XCTUnwrap(
      AVAudioFormat(
        commonFormat: .pcmFormatFloat32, sampleRate: 48000, channels: 2, interleaved: false))
    let buffer = try XCTUnwrap(
      AVAudioPCMBuffer(pcmFormat: format, frameCapacity: max(frames, 1)))
    buffer.frameLength = frames
    if frames > 0 {
      let samples = try XCTUnwrap(buffer.floatChannelData)
      for frame in 0..<Int(frames) {
        samples[0][frame] = 0.25
        samples[1][frame] = -0.5
      }
    }
    let file = try AVAudioFile(
      forWriting: url, settings: format.settings, commonFormat: .pcmFormatFloat32,
      interleaved: false)
    if frames > 0 { try file.write(from: buffer) }
    return url
  }

  private var expected: TrackStats {
    TrackStats(callbackCount: 5, frames: 4801, sampleRate: 48000, channels: 2)
  }

  func testExactFinalizedFrameCountAndFormatAreAcceptedWithoutChangingAudio() throws {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    let url = try audio(root)
    let original = try Data(contentsOf: url)
    XCTAssertNoThrow(try verifyRecordedCAF(url, expected: expected))
    XCTAssertEqual(try Data(contentsOf: url), original)
  }

  func testMissingTailInsertedFramesAndFormatChangesAreRejectedWithoutChangingAudio() throws {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    let url = try audio(root)
    let original = try Data(contentsOf: url)
    var cases: [TrackStats] = []
    var short = expected
    short.frames -= 1
    cases.append(short)
    var long = expected
    long.frames += 1
    cases.append(long)
    var rate = expected
    rate.sampleRate = 44100
    cases.append(rate)
    var channels = expected
    channels.channels = 1
    cases.append(channels)
    for stats in cases {
      XCTAssertThrowsError(try verifyRecordedCAF(url, expected: stats))
      XCTAssertEqual(try Data(contentsOf: url), original)
    }
  }

  func testFailedWritesMissingAndInvalidStatisticsCannotBeReportedAsSaved() throws {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    let url = try audio(root)
    let original = try Data(contentsOf: url)
    var cases: [TrackStats] = [TrackStats()]
    var failures = expected
    failures.appendFailures = 1
    cases.append(failures)
    var empty = expected
    empty.frames = 0
    cases.append(empty)
    for rate in [Double.nan, Double.infinity, 7999, 384001, 48000.5] {
      var stats = expected
      stats.sampleRate = rate
      cases.append(stats)
    }
    for count: UInt32 in [0, 9] {
      var stats = expected
      stats.channels = count
      cases.append(stats)
    }
    for stats in cases {
      XCTAssertThrowsError(try verifyRecordedCAF(url, expected: stats))
      XCTAssertEqual(try Data(contentsOf: url), original)
    }
  }

  func testEmptyBrokenAndWrongContainerFilesAreRetainedAndRejected() throws {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    let empty = try audio(root, name: "empty.caf", frames: 0)
    let wav = try audio(root, name: "wrong.wav")
    let broken = root.appendingPathComponent("broken.caf")
    try Data("caff broken file".utf8).write(to: broken)
    for url in [empty, wav, broken] {
      let original = try Data(contentsOf: url)
      XCTAssertThrowsError(try verifyRecordedCAF(url, expected: expected))
      XCTAssertEqual(try Data(contentsOf: url), original)
    }
    XCTAssertThrowsError(
      try verifyRecordedCAF(root.appendingPathComponent("missing.caf"), expected: expected))
  }
}
