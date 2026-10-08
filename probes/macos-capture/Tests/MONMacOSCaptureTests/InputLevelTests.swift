import AVFoundation
import CoreMedia
import Foundation
import XCTest

@testable import MONMacOSCapture

final class InputLevelTests: XCTestCase {
  private let owner = "735027d4-a03e-43d3-830a-21e767dfaf88"
  private func buffer(_ values: [[Float]], interleaved: Bool = false) throws -> AVAudioPCMBuffer {
    let format = try XCTUnwrap(
      AVAudioFormat(
        commonFormat: .pcmFormatFloat32, sampleRate: 48000,
        channels: AVAudioChannelCount(values.count), interleaved: interleaved))
    let buffer = try XCTUnwrap(
      AVAudioPCMBuffer(
        pcmFormat: format, frameCapacity: AVAudioFrameCount(values[0].count)))
    buffer.frameLength = buffer.frameCapacity
    let data = try XCTUnwrap(buffer.floatChannelData)
    for channel in values.indices {
      for frame in values[channel].indices {
        data[interleaved ? 0 : channel][interleaved ? frame * values.count + channel : frame] =
          values[channel][frame]
      }
    }
    return buffer
  }

  func testAntiPhaseStereoDoesNotDisappearInEitherLayout() throws {
    for interleaved in [true, false] {
      var level = PCMInputLevel()
      let pcm = try buffer([[0.25, -0.5], [-0.25, 0.5]], interleaved: interleaved)
      level.receive(pcm, at: 1_000_000_000)
      let result = level.snapshot(at: 1_123_000_000)
      XCTAssertEqual(result.state, "measured")
      XCTAssertEqual(result.windowFrames, 2)
      XCTAssertEqual(result.ageMs, 123)
      XCTAssertEqual(try XCTUnwrap(result.rms), sqrt(0.15625), accuracy: 1e-12)
      XCTAssertEqual(result.peak, 0.5)
      // Measuring does not mutate either channel's original audio.
      XCTAssertEqual(pcm.floatChannelData![0][0], 0.25)
      XCTAssertEqual(pcm.floatChannelData![interleaved ? 0 : 1][interleaved ? 1 : 0], -0.25)
    }
  }

  func testSilenceIsMeasuredButWaitingAndInvalidHaveNoNumericValues() throws {
    var level = PCMInputLevel()
    XCTAssertEqual(level.snapshot(at: 0).state, "waiting")
    level.receive(try buffer([[0, 0, 0]]), at: 0)
    XCTAssertEqual(level.snapshot(at: 0).rms, 0)
    XCTAssertEqual(level.snapshot(at: 0).peak, 0)
    level.invalidate()
    let invalid = level.snapshot(at: 1)
    XCTAssertEqual(invalid.state, "unavailable")
    XCTAssertNil(invalid.rms)
    XCTAssertNil(invalid.ageMs)
    let fields = try XCTUnwrap(
      JSONSerialization.jsonObject(with: JSONEncoder().encode(invalid)) as? [String: Any])
    XCTAssertEqual(fields.count, 5)
    XCTAssertTrue(fields["rms"] is NSNull)
  }

  func testFullScaleAndOverFullScaleRemainMeasurementsNotClampedEvidence() throws {
    var level = PCMInputLevel()
    level.receive(try buffer([[1, -2]]), at: 0)
    let result = level.snapshot(at: 0)
    XCTAssertEqual(result.peak, 2)
    XCTAssertEqual(try XCTUnwrap(result.rms), sqrt(2.5), accuracy: 1e-12)
  }

  func testNonFiniteUnsupportedEmptyAndClockErrorsDoNotFabricateSilence() throws {
    for value in [Float.nan, Float.infinity, -Float.infinity, 17] {
      var level = PCMInputLevel()
      level.receive(try buffer([[value]]), at: 0)
      XCTAssertEqual(level.snapshot(at: 0).state, "unavailable")
    }
    var level = PCMInputLevel()
    let empty = try buffer([[0]])
    empty.frameLength = 0
    level.receive(empty, at: 0)
    XCTAssertEqual(level.snapshot(at: 0).state, "unavailable")
    let pcm = try buffer([[0.5]])
    level.receive(pcm, at: 100)
    XCTAssertEqual(level.snapshot(at: nil).state, "unavailable")
    XCTAssertEqual(level.snapshot(at: 99).state, "unavailable")
    level.receive(pcm, at: 99)
    XCTAssertEqual(level.snapshot(at: 101).state, "unavailable")
    let format = try XCTUnwrap(
      AVAudioFormat(
        commonFormat: .pcmFormatFloat64, sampleRate: 48000, channels: 1, interleaved: false))
    let unsupported = try XCTUnwrap(AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 1))
    unsupported.frameLength = 1
    level.receive(unsupported, at: 200)
    XCTAssertEqual(level.snapshot(at: 200).state, "unavailable")
  }

  func testIntegerFullScaleNormalizationInBothLayouts() throws {
    for formatType in [AVAudioCommonFormat.pcmFormatInt16, .pcmFormatInt32] {
      for interleaved in [true, false] {
        let format = try XCTUnwrap(
          AVAudioFormat(
            commonFormat: formatType, sampleRate: 48000, channels: 2, interleaved: interleaved))
        let pcm = try XCTUnwrap(AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 2))
        pcm.frameLength = 2
        for channel in 0..<2 {
          for frame in 0..<2 {
            let c = interleaved ? 0 : channel
            let i = interleaved ? frame * 2 + channel : frame
            if formatType == .pcmFormatInt16 {
              pcm.int16ChannelData![c][i] = frame == 0 ? Int16.min : 16384
            } else {
              pcm.int32ChannelData![c][i] = frame == 0 ? Int32.min : 1_073_741_824
            }
          }
        }
        var level = PCMInputLevel()
        level.receive(pcm, at: 0)
        let result = level.snapshot(at: 0)
        XCTAssertEqual(result.peak, 1)
        XCTAssertEqual(try XCTUnwrap(result.rms), sqrt(0.625), accuracy: 1e-12)
      }
    }
  }

  func testScreenCaptureSampleUsesActualPCMNotPresentationOrConnection() throws {
    var asbd = AudioStreamBasicDescription(
      mSampleRate: 48000, mFormatID: kAudioFormatLinearPCM,
      mFormatFlags: kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked,
      mBytesPerPacket: 8, mFramesPerPacket: 1, mBytesPerFrame: 8,
      mChannelsPerFrame: 2, mBitsPerChannel: 32, mReserved: 0)
    var description: CMAudioFormatDescription?
    XCTAssertEqual(
      CMAudioFormatDescriptionCreate(
        allocator: kCFAllocatorDefault, asbd: &asbd, layoutSize: 0, layout: nil,
        magicCookieSize: 0, magicCookie: nil, extensions: nil, formatDescriptionOut: &description),
      noErr)
    let values: [Float] = [0.25, -0.25, 0.5, -0.5]
    var block: CMBlockBuffer?
    XCTAssertEqual(
      CMBlockBufferCreateWithMemoryBlock(
        allocator: kCFAllocatorDefault, memoryBlock: nil, blockLength: 16,
        blockAllocator: kCFAllocatorDefault, customBlockSource: nil, offsetToData: 0,
        dataLength: 16, flags: 0, blockBufferOut: &block), noErr)
    let bytes = try XCTUnwrap(block)
    values.withUnsafeBytes { data in
      XCTAssertEqual(
        CMBlockBufferReplaceDataBytes(
          with: data.baseAddress!, blockBuffer: bytes,
          offsetIntoDestination: 0, dataLength: data.count), noErr)
    }
    var sample: CMSampleBuffer?
    XCTAssertEqual(
      CMAudioSampleBufferCreateReadyWithPacketDescriptions(
        allocator: kCFAllocatorDefault, dataBuffer: bytes,
        formatDescription: try XCTUnwrap(description),
        sampleCount: 2, presentationTimeStamp: CMTime(value: 900, timescale: 1),
        packetDescriptions: nil,
        sampleBufferOut: &sample), noErr)
    var level = PCMInputLevel()
    level.receive(try XCTUnwrap(sample), at: 1_000_000_000)
    let result = level.snapshot(at: 1_020_000_000)
    XCTAssertEqual(result.ageMs, 20)
    XCTAssertEqual(result.peak, 0.5)
    XCTAssertEqual(try XCTUnwrap(result.rms), sqrt(0.15625), accuracy: 1e-12)
  }

  func testTelemetryOwnerRequiresCanonicalV4AndParentControls() throws {
    let args = ["--output", "/tmp/mon-level-options", "--input-level-owner", owner]
    XCTAssertThrowsError(try Options.parse(args))
    let controls = ["--stop-on-file", "--parent-pid", "1234"]
    XCTAssertEqual(try Options.parse(args + controls + ["--duration", "5"]).inputLevelOwner, owner)
    XCTAssertNil(try Options.parse(["--output", "/tmp/mon-level-options"]).inputLevelOwner)
    for invalid in [
      owner.uppercased(), "735027d4-a03e-13d3-830a-21e767dfaf88",
      "735027d4-a03e-43d3-030a-21e767dfaf88", "", "../outside",
    ] {
      XCTAssertThrowsError(
        try Options.parse(
          ["--output", "/tmp/mon-level-options"] + controls + ["--input-level-owner", invalid]))
    }
    XCTAssertThrowsError(try Options.parse(args + controls + ["--input-level-owner", owner]))
  }

  func testPlanarScreenCaptureSamplesCopyEveryChannelAndConservativelyAge() throws {
    let pcm = try buffer([[0.25, -0.5], [-0.25, 0.5]])
    var sample: CMSampleBuffer?
    var timing = CMSampleTimingInfo(
      duration: CMTime(value: 1, timescale: 48000),
      presentationTimeStamp: .zero, decodeTimeStamp: .invalid)
    XCTAssertEqual(
      CMSampleBufferCreate(
        allocator: kCFAllocatorDefault, dataBuffer: nil, dataReady: false,
        makeDataReadyCallback: nil, refcon: nil,
        formatDescription: pcm.format.formatDescription, sampleCount: 2,
        sampleTimingEntryCount: 1, sampleTimingArray: &timing,
        sampleSizeEntryCount: 0, sampleSizeArray: nil, sampleBufferOut: &sample), noErr)
    let sampleBuffer = try XCTUnwrap(sample)
    XCTAssertEqual(
      CMSampleBufferSetDataBufferFromAudioBufferList(
        sampleBuffer, blockBufferAllocator: kCFAllocatorDefault,
        blockBufferMemoryAllocator: kCFAllocatorDefault, flags: 0,
        bufferList: pcm.audioBufferList), noErr)
    XCTAssertEqual(CMSampleBufferSetDataReady(sampleBuffer), noErr)
    var level = PCMInputLevel()
    level.receive(sampleBuffer, at: 100)
    let result = level.snapshot(at: 101)
    XCTAssertEqual(result.state, "measured")
    XCTAssertEqual(result.ageMs, 1)
    XCTAssertEqual(result.peak, 0.5)
    XCTAssertEqual(try XCTUnwrap(result.rms), sqrt(0.15625), accuracy: 1e-12)
  }

  func testSwiftLevelPublicationAndManifestExclusion() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(
      "mon-input-level-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
    defer { try? FileManager.default.removeItem(at: directory) }
    var microphone = PCMInputLevel()
    var system = PCMInputLevel()
    microphone.receive(try buffer([[0.25, -0.5]]), at: 100_000_000)
    system.receive(try buffer([[0, 0]]), at: 100_000_000)
    let report = InputLevelReport(
      ownerId: owner, revision: 1,
      microphone: microphone.snapshot(at: 110_000_000), system: system.snapshot(at: 110_000_000))
    try report.publish(to: directory)
    let data = try Data(contentsOf: directory.appendingPathComponent("input-levels.json"))
    let fields = try XCTUnwrap(JSONSerialization.jsonObject(with: data) as? [String: Any])
    XCTAssertEqual(fields.count, 5)
    XCTAssertEqual(fields["ownerId"] as? String, owner)
    var stats = TrackStats()
    stats.inputLevel = microphone
    let saved = try XCTUnwrap(
      JSONSerialization.jsonObject(with: JSONEncoder().encode(stats)) as? [String: Any])
    XCTAssertNil(saved["inputLevel"])
    if let path = ProcessInfo.processInfo.environment["MON_INPUT_LEVEL_FIXTURE_DIR"] {
      try report.publish(to: URL(fileURLWithPath: path, isDirectory: true))
    }
  }
}
