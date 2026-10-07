import AVFoundation
import CoreMedia
import Foundation
import XCTest

@testable import MONMacOSCapture

final class CaptureClockFixtureTests: XCTestCase, @unchecked Sendable {
  private func sample(frames: Int, value: Float, presentation: CMTime) throws -> CMSampleBuffer {
    var format = AudioStreamBasicDescription(
      mSampleRate: 48000, mFormatID: kAudioFormatLinearPCM,
      mFormatFlags: kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked,
      mBytesPerPacket: 8, mFramesPerPacket: 1, mBytesPerFrame: 8,
      mChannelsPerFrame: 2, mBitsPerChannel: 32, mReserved: 0)
    var description: CMAudioFormatDescription?
    XCTAssertEqual(
      CMAudioFormatDescriptionCreate(
        allocator: kCFAllocatorDefault, asbd: &format, layoutSize: 0, layout: nil,
        magicCookieSize: 0, magicCookie: nil, extensions: nil,
        formatDescriptionOut: &description), noErr)
    let samples = (0..<frames * 2).map { $0 % 2 == 0 ? value : -value }
    var block: CMBlockBuffer?
    XCTAssertEqual(
      CMBlockBufferCreateWithMemoryBlock(
        allocator: kCFAllocatorDefault, memoryBlock: nil, blockLength: samples.count * 4,
        blockAllocator: kCFAllocatorDefault, customBlockSource: nil, offsetToData: 0,
        dataLength: samples.count * 4, flags: 0, blockBufferOut: &block), noErr)
    let data = try XCTUnwrap(block)
    samples.withUnsafeBytes {
      XCTAssertEqual(
        CMBlockBufferReplaceDataBytes(
          with: $0.baseAddress!, blockBuffer: data, offsetIntoDestination: 0,
          dataLength: $0.count), noErr)
    }
    var sample: CMSampleBuffer?
    XCTAssertEqual(
      CMAudioSampleBufferCreateReadyWithPacketDescriptions(
        allocator: kCFAllocatorDefault, dataBuffer: data,
        formatDescription: try XCTUnwrap(description), sampleCount: frames,
        presentationTimeStamp: presentation, packetDescriptions: nil,
        sampleBufferOut: &sample), noErr)
    return try XCTUnwrap(sample)
  }

  private func writeCAF(_ samples: [CMSampleBuffer], to url: URL) async throws -> TrackStats {
    let writer = try AVAssetWriter(outputURL: url, fileType: .caf)
    let settings: [String: Any] = [
      AVFormatIDKey: kAudioFormatLinearPCM, AVSampleRateKey: 48000,
      AVNumberOfChannelsKey: 2, AVLinearPCMBitDepthKey: 32, AVLinearPCMIsFloatKey: true,
      AVLinearPCMIsNonInterleaved: false, AVLinearPCMIsBigEndianKey: false,
    ]
    let input = AVAssetWriterInput(mediaType: .audio, outputSettings: settings)
    writer.add(input)
    XCTAssertTrue(writer.startWriting())
    writer.startSession(atSourceTime: .zero)
    var stats = TrackStats(sampleRate: 48000, channels: 2)
    for sample in samples {
      let contiguous = try continuousPCMSample(sample, frameOffset: stats.frames, sampleRate: 48000)
      XCTAssertEqual(contiguous.numSamples, sample.numSamples)
      XCTAssertEqual(
        contiguous.presentationTimeStamp, CMTime(value: stats.frames, timescale: 48000))
      var originalLength = 0
      var originalBytes: UnsafeMutablePointer<Int8>?
      var copiedLength = 0
      var copiedBytes: UnsafeMutablePointer<Int8>?
      XCTAssertEqual(
        CMBlockBufferGetDataPointer(
          try XCTUnwrap(sample.dataBuffer), atOffset: 0, lengthAtOffsetOut: nil,
          totalLengthOut: &originalLength, dataPointerOut: &originalBytes), noErr)
      XCTAssertEqual(
        CMBlockBufferGetDataPointer(
          try XCTUnwrap(contiguous.dataBuffer), atOffset: 0, lengthAtOffsetOut: nil,
          totalLengthOut: &copiedLength, dataPointerOut: &copiedBytes), noErr)
      XCTAssertEqual(originalLength, copiedLength)
      XCTAssertEqual(memcmp(originalBytes!, copiedBytes!, originalLength), 0)
      // Bounded wait, not an unbounded test hang if the writer fails.
      let deadline = ContinuousClock.now.advanced(by: .seconds(5))
      while !input.isReadyForMoreMediaData, ContinuousClock.now < deadline {
        try await Task.sleep(for: .milliseconds(1))
      }
      guard input.isReadyForMoreMediaData, input.append(contiguous) else {
        throw ProbeError.capture("synthetic CAF append failed")
      }
      stats.frames += Int64(sample.numSamples)
      stats.callbackCount += 1
    }
    writer.endSession(atSourceTime: CMTime(value: stats.frames, timescale: 48000))
    input.markAsFinished()
    await writer.finishWriting()
    XCTAssertEqual(writer.status, .completed)
    try verifyRecordedCAF(url, expected: stats)
    return stats
  }

  func testSwiftCaptureClockFixtureRoundTripsRealPCMAndExportsActualManifest() async throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(
      "mon-clock-fixture-\(UUID().uuidString)", isDirectory: true)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
    defer { try? FileManager.default.removeItem(at: directory) }
    let origin: Int64 = 10_000_000_000
    let initial = CaptureClockReading(
      hostBeforeNs: 1_000_000_000, continuousNs: 1_000_000_010,
      utcMs: 1_700_000_000_000, hostAfterNs: 1_000_000_020)
    let readings = ReadingBox(initial)
    let clock = try CaptureClockDomain(units: MachTimeUnits(numerator: 1, denominator: 1)) {
      readings.read()
    }
    readings.set(
      CaptureClockReading(
        hostBeforeNs: 20_000_000_000, continuousNs: 20_000_000_010,
        utcMs: 1_900_000_000_000, hostAfterNs: 20_000_000_020))
    let micURL = directory.appendingPathComponent("microphone.caf")
    let systemURL = directory.appendingPathComponent("system.caf")
    var mic = try await writeCAF(
      [sample(frames: 4800, value: 0.125, presentation: CMTime(value: 10, timescale: 1))],
      to: micURL)
    var system = try await writeCAF(
      [
        sample(frames: 2400, value: 0.25, presentation: CMTime(value: 10005, timescale: 1000)),
        sample(frames: 2400, value: 0.5, presentation: CMTime(value: 10255, timescale: 1000)),
      ],
      to: systemURL)
    mic.recordSavedClock(clock.observe(hostTicks: UInt64(origin)), frames: 4800, sourceFrame: 0)
    system.recordSavedClock(
      clock.observe(hostTicks: UInt64(origin + 5_000_000)), frames: 2400, sourceFrame: nil)
    system.recordSavedClock(
      clock.observe(hostTicks: UInt64(origin + 255_000_000)), frames: 2400, sourceFrame: nil)
    var manifest = Manifest(
      schemaVersion: 2, scenario: "synthetic-clock-proof", captureMode: "system",
      backend: "screencapturekit", targetPID: nil, targetPIDs: [],
      startedAtUTC: "2023-11-14T22:13:20Z", requestedDurationSeconds: nil,
      osVersion: "synthetic", architecture: "arm64", state: "saved",
      microphone: mic, system: system, errors: [])
    try manifest.finalizeClock(clock)
    XCTAssertEqual(manifest.microphone.clockSegments?.first?.timelineStartUs, 0)
    XCTAssertEqual(manifest.system.clockSegments?.count, 2)
    XCTAssertEqual(manifest.system.clockSegments?[0].timelineEndUs, 55000)
    XCTAssertEqual(manifest.system.clockSegments?[1].timelineStartUs, 255000)
    XCTAssertEqual(manifest.captureClock?.originUtcMs, 1_700_000_008_999)
    let manifestData = try manifest.encodedData()
    let json = try JSONSerialization.jsonObject(with: manifestData) as! [String: Any]
    XCTAssertNil(json["requestedDurationSeconds"])
    for name in ["microphone", "system"] {
      let track = json[name] as! [String: Any]
      XCTAssertNil(track["timing"])
      XCTAssertNil(track["clockFailure"])
    }
    try manifestData.write(to: directory.appendingPathComponent("manifest.json"), options: .atomic)
    for (url, values) in [(micURL, [Float(0.125), 0.125]), (systemURL, [Float(0.25), 0.5])] {
      let file = try AVAudioFile(forReading: url)
      XCTAssertEqual(file.length, 4800)  // No 200 ms of writer-generated silence.
      let buffer = try XCTUnwrap(
        AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: 4800))
      try file.read(into: buffer)
      XCTAssertEqual(buffer.frameLength, 4800)
      for frame in 0..<4800 {
        XCTAssertEqual(buffer.floatChannelData![0][frame], values[frame / 2400])
        XCTAssertEqual(buffer.floatChannelData![1][frame], -values[frame / 2400])
      }
    }
    if let destinationPath = ProcessInfo.processInfo.environment["MON_CAPTURE_CLOCK_FIXTURE_DIR"] {
      let destination = URL(fileURLWithPath: destinationPath, isDirectory: true)
      XCTAssertTrue(try FileManager.default.contentsOfDirectory(atPath: destinationPath).isEmpty)
      for name in ["manifest.json", "microphone.caf", "system.caf"] {
        // Never overwrite a supplied destination's existing file.
        try FileManager.default.copyItem(
          at: directory.appendingPathComponent(name), to: destination.appendingPathComponent(name))
      }
    }
    // Capacity failure must not discard maps or silently publish schema 1.
    manifest.errors = [String(repeating: "x", count: 256 * 1024)]
    XCTAssertThrowsError(try manifest.encodedData())
  }

  func testRetimingRejectsBadRatesAndOffsetsWithoutChangingInput() throws {
    let input = try sample(frames: 48, value: 0.125, presentation: CMTime(value: 10, timescale: 1))
    for rate in [Double.nan, .infinity, 7999, 48000.5, 96000] {
      XCTAssertThrowsError(try continuousPCMSample(input, frameOffset: 0, sampleRate: rate))
    }
    XCTAssertThrowsError(try continuousPCMSample(input, frameOffset: -1, sampleRate: 48000))
    XCTAssertEqual(input.presentationTimeStamp, CMTime(value: 10, timescale: 1))
    XCTAssertEqual(input.numSamples, 48)
  }
}
