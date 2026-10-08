import AVFoundation
import CoreMedia
import Foundation
import XCTest

@testable import MONMacOSCapture

final class CaptureTestWindowTests: XCTestCase, @unchecked Sendable {
  private let owner = "735027d4-a03e-43d3-830a-21e767dfaf88"
  private let origin: Int64 = 10_000_000_000

  private func window() throws -> CaptureTestWindow {
    let window = try CaptureTestWindow(ownerId: owner)
    try window.open(at: origin)
    return window
  }

  private func selection(
    _ window: CaptureTestWindow, at timestamp: Int64, frames: Int = 9600,
    rate: Double = 48000, saved: Int64 = 0
  ) throws -> CaptureTestSelection? {
    try window.selection(
      observation: .timestamp(timestamp), frames: frames, sampleRate: rate, savedFrames: saved)
  }

  func testSingleUseWindowDiscardsInitializationAndCannotReopenAfterCancellation() throws {
    let waiting = try CaptureTestWindow(ownerId: owner)
    XCTAssertFalse(waiting.accepting)
    XCTAssertNil(waiting.metadata().windowStartNs)
    XCTAssertNil(
      try waiting.selection(
        observation: .invalid("not initialized"), frames: 0,
        sampleRate: .nan, savedFrames: -1))
    try waiting.open(at: origin)
    XCTAssertTrue(waiting.accepting)
    XCTAssertThrowsError(try waiting.open(at: origin))
    waiting.finish(at: origin + 1)
    XCTAssertFalse(waiting.accepting)
    XCTAssertFalse(waiting.metadata().completed)
    waiting.finish(at: origin + CaptureTestWindow.durationNs)
    XCTAssertFalse(waiting.metadata().completed)  // A cancelled test never becomes complete.
    XCTAssertThrowsError(try waiting.open(at: origin))
    XCTAssertNil(try selection(waiting, at: origin))
    let cancelled = try CaptureTestWindow(ownerId: owner)
    cancelled.finish(at: origin)
    XCTAssertThrowsError(try cancelled.open(at: origin))
    for id in ["", owner.uppercased(), "../../../audio", "735027d4-a03e-53d3-830a-21e767dfaf88"] {
      XCTAssertThrowsError(try CaptureTestWindow(ownerId: id))
    }
    for timestamp: Int64? in [nil, -1, Int64.max] {
      XCTAssertThrowsError(try CaptureTestWindow(ownerId: owner).open(at: timestamp))
    }
  }

  func testCaptureTimestampsExcludeQueuedInitializationAndTrimBothBoundaries() throws {
    let window = try window()
    XCTAssertNil(try selection(window, at: 0))
    XCTAssertNil(try selection(window, at: origin - 200_000_000))
    let first = try XCTUnwrap(selection(window, at: origin - 100_000_000))
    XCTAssertEqual(first.frames, 4800..<9600)
    if case .timestamp(let timestamp) = first.observation {
      XCTAssertEqual(timestamp, origin)
    } else {
      XCTFail("lost capture timestamp")
    }
    XCTAssertEqual(try selection(window, at: origin)?.frames, 0..<9600)
    XCTAssertEqual(try selection(window, at: origin + 4_900_000_000)?.frames, 0..<4800)
    XCTAssertNil(try selection(window, at: origin + CaptureTestWindow.durationNs))
    XCTAssertNil(try selection(window, at: Int64.max))
  }

  func testFractionalFrameBoundariesAndBothTrackBudgetsAreConservative() throws {
    let window = try window()
    XCTAssertEqual(try selection(window, at: origin - 1, frames: 3, rate: 8000)?.frames, 1..<3)
    XCTAssertNil(try selection(window, at: origin + 4_999_999_999, frames: 3, rate: 8000))
    for rate: Double in [8000, 44100, 48000, 384000] {
      let maximum = Int64(rate) * 5
      // Independent saved counts, not a shared five-second frame pool.
      XCTAssertEqual(
        try selection(window, at: origin, frames: 10, rate: rate, saved: maximum - 3)?.frames, 0..<3
      )
      XCTAssertEqual(
        try selection(window, at: origin, frames: 10, rate: rate, saved: 0)?.frames, 0..<10)
      XCTAssertNil(try selection(window, at: origin, frames: 10, rate: rate, saved: maximum))
      XCTAssertThrowsError(try selection(window, at: origin, rate: rate, saved: maximum + 1))
    }
    window.finish(at: origin + CaptureTestWindow.durationNs)
    XCTAssertTrue(window.metadata().completed)
    XCTAssertEqual(window.metadata().windowEndNs, origin + CaptureTestWindow.durationNs)
  }

  func testInvalidTimestampsFormatsAndFrameCountsCannotBecomeTestAudio() throws {
    let window = try window()
    XCTAssertThrowsError(
      try window.selection(
        observation: .invalid("clock bridge changed"), frames: 1, sampleRate: 48000, savedFrames: 0)
    )
    XCTAssertThrowsError(try selection(window, at: -1))
    for rate in [Double.nan, .infinity, 7999, 384001, 44100.5] {
      XCTAssertThrowsError(try selection(window, at: origin, rate: rate))
    }
    for frames in [0, -1, 1_000_001] {
      XCTAssertThrowsError(try selection(window, at: origin, frames: frames))
    }
    XCTAssertThrowsError(try selection(window, at: origin, saved: -1))
    window.finish(at: nil)
    XCTAssertFalse(window.metadata().completed)
  }

  func testEverySelectedWholeFrameFitsTheSharedWindowAtFractionalRates() throws {
    let window = try window()
    for rate: Int64 in [8000, 11025, 16000, 22050, 44100, 48000, 96000, 192000, 384000] {
      for offset: Int64 in [
        -1_000_000_001, -100_000_001, -1, 0, 1, 4_900_000_001, 4_999_999_999, 5_000_000_000,
      ] {
        for frames in [1, 2, 1024, 9600, 1_000_000] {
          for saved in [Int64(0), rate * 5 - 1, rate * 5] {
            let timestamp = origin + offset
            if let selected = try selection(
              window, at: timestamp, frames: frames, rate: Double(rate), saved: saved)
            {
              XCTAssertGreaterThanOrEqual(selected.frames.lowerBound, 0)
              XCTAssertLessThanOrEqual(selected.frames.upperBound, frames)
              XCTAssertLessThanOrEqual(Int64(selected.frames.count) + saved, rate * 5)
              // Exact integer products verify fractional sample boundaries,
              // not a floating-point timestamp rounded into the window.
              XCTAssertGreaterThanOrEqual(
                timestamp * rate + Int64(selected.frames.lowerBound) * 1_000_000_000, origin * rate)
              XCTAssertLessThanOrEqual(
                timestamp * rate + Int64(selected.frames.upperBound) * 1_000_000_000,
                (origin + 5_000_000_000) * rate)
            }
          }
        }
      }
    }
  }

  private func pcm(
    formatType: AVAudioCommonFormat = .pcmFormatFloat32, interleaved: Bool = false,
    frames: Int = 8, rate: Double = 48000
  ) throws -> AVAudioPCMBuffer {
    let format = try XCTUnwrap(
      AVAudioFormat(
        commonFormat: formatType, sampleRate: rate, channels: 2, interleaved: interleaved))
    let buffer = try XCTUnwrap(
      AVAudioPCMBuffer(pcmFormat: format, frameCapacity: AVAudioFrameCount(frames)))
    buffer.frameLength = buffer.frameCapacity
    for channel in 0..<2 {
      for frame in 0..<frames {
        let c = interleaved ? 0 : channel
        let i = interleaved ? frame * 2 + channel : frame
        let sign = channel == 0 ? 1 : -1
        switch formatType {
        case .pcmFormatFloat32: buffer.floatChannelData![c][i] = Float(sign * (frame + 1)) / 16
        case .pcmFormatFloat64:
          let data = UnsafeMutableAudioBufferListPointer(buffer.mutableAudioBufferList)
          data[c].mData!.assumingMemoryBound(to: Double.self)[i] = Double(sign * (frame + 1)) / 16
        case .pcmFormatInt16: buffer.int16ChannelData![c][i] = Int16(sign * (frame + 1))
        case .pcmFormatInt32: buffer.int32ChannelData![c][i] = Int32(sign * (frame + 1))
        default: XCTFail("unexpected test format")
        }
      }
    }
    return buffer
  }

  func testMicrophoneSlicesPreserveEveryChannelInEverySupportedPCMLayout() throws {
    for type in [
      AVAudioCommonFormat.pcmFormatFloat32, .pcmFormatFloat64, .pcmFormatInt16, .pcmFormatInt32,
    ] {
      for interleaved in [false, true] {
        let source = try pcm(formatType: type, interleaved: interleaved)
        let copy = try testPCMFrames(source, range: 2..<5)
        XCTAssertEqual(copy.frameLength, 3)
        let input = UnsafeMutableAudioBufferListPointer(
          UnsafeMutablePointer(mutating: source.audioBufferList))
        let output = UnsafeMutableAudioBufferListPointer(copy.mutableAudioBufferList)
        let width = type == .pcmFormatFloat64 ? 8 : type == .pcmFormatInt16 ? 2 : 4
        for index in input.indices {
          let stride = width * Int(input[index].mNumberChannels)
          XCTAssertEqual(
            memcmp(input[index].mData!.advanced(by: 2 * stride), output[index].mData!, 3 * stride),
            0)
        }
        XCTAssertEqual(source.frameLength, 8)
        for range in [-1..<1, 0..<0, 7..<9] {
          XCTAssertThrowsError(try testPCMFrames(source, range: range))
        }
      }
    }
  }

  private func sample(_ pcm: AVAudioPCMBuffer, presentation: CMTime) throws -> CMSampleBuffer {
    var timing = CMSampleTimingInfo(
      duration: CMTime(value: 1, timescale: Int32(pcm.format.sampleRate)),
      presentationTimeStamp: presentation, decodeTimeStamp: .invalid)
    var result: CMSampleBuffer?
    XCTAssertEqual(
      CMSampleBufferCreate(
        allocator: kCFAllocatorDefault, dataBuffer: nil, dataReady: false,
        makeDataReadyCallback: nil, refcon: nil, formatDescription: pcm.format.formatDescription,
        sampleCount: Int(pcm.frameLength), sampleTimingEntryCount: 1, sampleTimingArray: &timing,
        sampleSizeEntryCount: 0, sampleSizeArray: nil, sampleBufferOut: &result), noErr)
    let output = try XCTUnwrap(result)
    XCTAssertEqual(
      CMSampleBufferSetDataBufferFromAudioBufferList(
        output, blockBufferAllocator: kCFAllocatorDefault,
        blockBufferMemoryAllocator: kCFAllocatorDefault, flags: 0, bufferList: pcm.audioBufferList),
      noErr)
    XCTAssertEqual(CMSampleBufferSetDataReady(output), noErr)
    return output
  }

  func testSystemSlicesPreservePlanarAndInterleavedSamplesAndTheirCaptureTime() throws {
    for interleaved in [false, true] {
      let input = try pcm(interleaved: interleaved)
      let source = try sample(input, presentation: CMTime(value: 10, timescale: 1))
      let copy = try testPCMFrames(source, range: 2..<5)
      XCTAssertEqual(copy.numSamples, 3)
      XCTAssertEqual(copy.presentationTimeStamp, CMTime(value: 480002, timescale: 48000))
      XCTAssertEqual(source.numSamples, 8)
      XCTAssertEqual(source.presentationTimeStamp, CMTime(value: 10, timescale: 1))
      let output = try XCTUnwrap(AVAudioPCMBuffer(pcmFormat: input.format, frameCapacity: 3))
      output.frameLength = 3
      XCTAssertEqual(
        CMSampleBufferCopyPCMDataIntoAudioBufferList(
          copy, at: 0, frameCount: 3, into: output.mutableAudioBufferList), noErr)
      for channel in 0..<2 {
        for frame in 0..<3 {
          let c = interleaved ? 0 : channel
          let i = interleaved ? frame * 2 + channel : frame
          XCTAssertEqual(
            output.floatChannelData![c][i], Float((channel == 0 ? 1 : -1) * (frame + 3)) / 16)
        }
      }
    }
  }

  func testFiveSecondOptionsRequireOwnedStopControlsAndDoNotAlterOrdinaryProbes() throws {
    let output = ["--output", "/tmp/mon-test-window-options"]
    let controls = [
      "--stop-on-file", "--parent-pid", "1234", "--microphone-uid", "USB input/名前 with spaces",
    ]
    let flag = ["--five-second-test", owner]
    let options = try Options.parse(output + controls + flag)
    XCTAssertEqual(options.duration, 5)
    XCTAssertEqual(options.captureTestOwner, owner)
    XCTAssertEqual(options.inputLevelOwner, owner)
    XCTAssertEqual(options.microphoneUID, controls.last)
    let ordinary = try Options.parse(output + controls + ["--duration", "5"])
    XCTAssertNil(ordinary.captureTestOwner)
    XCTAssertNil(ordinary.inputLevelOwner)
    XCTAssertEqual(ordinary.duration, 5)
    for args in [
      flag, flag + ["--stop-on-file"], flag + ["--parent-pid", "1234"],
      ["--stop-on-file", "--parent-pid", "1234"] + flag,
    ] {
      XCTAssertThrowsError(try Options.parse(output + args))
    }
    for extra in [
      ["--duration", "5"], ["--until-stopped"], ["--input-level-owner", owner], flag,
      ["--mode", "application", "--pid", "1234", "--backend", "coreaudio"],
    ] {
      XCTAssertThrowsError(try Options.parse(output + controls + flag + extra))
    }
    let start = ContinuousClock.now
    XCTAssertFalse(
      options.shouldContinueRecording(
        startedAt: start, now: start.advanced(by: .seconds(5)), stopRequested: false,
        parentExited: false))
    XCTAssertFalse(
      options.shouldContinueRecording(
        startedAt: start, now: start, stopRequested: true, parentExited: false))
    XCTAssertFalse(
      options.shouldContinueRecording(
        startedAt: start, now: start, stopRequested: false, parentExited: true))
  }

  private func savedTestTrack(
    _ window: CaptureTestWindow, microphone: Bool, to url: URL
  ) async throws -> TrackStats {
    let format = try XCTUnwrap(
      AVAudioFormat(
        commonFormat: .pcmFormatFloat32, sampleRate: 8000, channels: 2, interleaved: !microphone))
    var file: AVAudioFile?
    var writer: AVAssetWriter?
    var input: AVAssetWriterInput?
    if microphone {
      file = try AVAudioFile(
        forWriting: url, settings: format.settings, commonFormat: .pcmFormatFloat32,
        interleaved: false)
    } else {
      let asset = try AVAssetWriter(outputURL: url, fileType: .caf)
      let settings: [String: Any] = [
        AVFormatIDKey: kAudioFormatLinearPCM, AVSampleRateKey: 8000, AVNumberOfChannelsKey: 2,
        AVLinearPCMBitDepthKey: 32, AVLinearPCMIsFloatKey: true, AVLinearPCMIsNonInterleaved: false,
        AVLinearPCMIsBigEndianKey: false,
      ]
      let sink = AVAssetWriterInput(mediaType: .audio, outputSettings: settings)
      asset.add(sink)
      XCTAssertTrue(asset.startWriting())
      asset.startSession(atSourceTime: .zero)
      writer = asset
      input = sink
    }
    var stats = TrackStats(sampleRate: 8000, channels: 2)
    // Early samples and a late tail exist in the source, not in either CAF.
    // PC audio has a measured 100 ms late start and a 1 s capture hole.
    let timestamps: [Int64] =
      [origin - 300_000_000, origin - 100_000_000]
      + (0..<25).map { origin + 100_000_000 + Int64($0) * 200_000_000 }
      + [origin + 5_100_000_000]
    for timestamp in timestamps {
      if !microphone,
        timestamp < origin + 100_000_000
          || (origin + 1_900_000_000..<origin + 2_900_000_000).contains(timestamp)
      {
        continue
      }
      let pcm = try XCTUnwrap(AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 1600))
      pcm.frameLength = 1600
      for channel in 0..<2 {
        for frame in 0..<1600 {
          let c = microphone ? channel : 0
          let i = microphone ? frame : frame * 2 + channel
          pcm.floatChannelData![c][i] = microphone ? (channel == 0 ? 0.125 : -0.125) : 0
        }
      }
      guard
        let selected = try selection(
          window, at: timestamp, frames: 1600, rate: 8000, saved: stats.frames)
      else { continue }
      if microphone {
        try XCTUnwrap(file).write(from: testPCMFrames(pcm, range: selected.frames))
      } else {
        let original = try sample(
          pcm, presentation: CMTime(value: timestamp, timescale: 1_000_000_000))
        let sliced = try testPCMFrames(original, range: selected.frames)
        let contiguous = try continuousPCMSample(
          sliced, frameOffset: stats.frames, sampleRate: 8000)
        let deadline = ContinuousClock.now.advanced(by: .seconds(5))
        while input?.isReadyForMoreMediaData != true, ContinuousClock.now < deadline {
          try await Task.sleep(for: .milliseconds(1))
        }
        guard let input, input.isReadyForMoreMediaData, input.append(contiguous) else {
          throw ProbeError.capture("test window fixture CAF append failed")
        }
      }
      stats.callbackCount += 1
      stats.frames += Int64(selected.frames.count)
      stats.recordSavedClock(
        selected.observation, frames: Int64(selected.frames.count), sourceFrame: nil)
    }
    file = nil
    if let writer, let input {
      writer.endSession(atSourceTime: CMTime(value: stats.frames, timescale: 8000))
      input.markAsFinished()
      await writer.finishWriting()
      XCTAssertEqual(writer.status, .completed)
    }
    try verifyRecordedCAF(url, expected: stats)
    return stats
  }

  func testSelectedApplicationTestRequiresTheExactProcessInstanceButLegacyProbesRemainValid() throws
  {
    let identity = CaptureApplicationIdentity(
      pid: 1234, bundleIdentifier: "com.example.browser", launchTimeUs: 1_791_100_000_000_001)
    let json = String(decoding: try JSONEncoder().encode(identity), as: UTF8.self)
    let selected = [
      "--output", "/tmp/mon-test-scope-options", "--stop-on-file", "--parent-pid", "2345",
      "--microphone-uid", "USB mic", "--mode", "application", "--pid", "1234",
    ]
    XCTAssertNoThrow(try Options.parse(selected))
    XCTAssertThrowsError(try Options.parse(selected + ["--five-second-test", owner]))
    let options = try Options.parse(
      selected + ["--five-second-test", owner, "--application-identity", json])
    XCTAssertEqual(options.duration, 5)
    XCTAssertEqual(options.applicationIdentity, identity)
    XCTAssertEqual(options.targetPIDs, [1234])
    let different = CaptureApplicationIdentity(
      pid: 1235, bundleIdentifier: identity.bundleIdentifier, launchTimeUs: identity.launchTimeUs)
    let wrong = String(decoding: try JSONEncoder().encode(different), as: UTF8.self)
    XCTAssertThrowsError(
      try Options.parse(selected + ["--five-second-test", owner, "--application-identity", wrong]))
  }

  func testSwiftFiveSecondWindowWritesOnlySelectedFramesAndExportsPrivateManifest() async throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(
      "mon-capture-test-fixture-\(UUID().uuidString)", isDirectory: true)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
    defer { try? FileManager.default.removeItem(at: directory) }
    let window = try window()
    let microphone = try await savedTestTrack(
      window, microphone: true, to: directory.appendingPathComponent("microphone.caf"))
    let system = try await savedTestTrack(
      window, microphone: false, to: directory.appendingPathComponent("system.caf"))
    XCTAssertEqual(microphone.frames, 40000)
    XCTAssertEqual(system.frames, 31200)
    window.finish(at: origin + CaptureTestWindow.durationNs)
    let reference = CaptureClockReading(
      hostBeforeNs: 1_000_000_000, continuousNs: 1_000_000_010, utcMs: 1_700_000_000_000,
      hostAfterNs: 1_000_000_020)
    let readings = ReadingBox(reference)
    let clock = try CaptureClockDomain(units: MachTimeUnits(numerator: 1, denominator: 1)) {
      readings.read()
    }
    readings.set(
      CaptureClockReading(
        hostBeforeNs: 20_000_000_000, continuousNs: 20_000_000_010, utcMs: 1_700_000_019_000,
        hostAfterNs: 20_000_000_020))
    var manifest = Manifest(
      schemaVersion: 2, scenario: "synthetic-five-second-test", captureMode: "system",
      backend: "screencapturekit", targetPID: nil, targetPIDs: [], targetApplicationName: nil,
      targetBundleIdentifier: nil, startedAtUTC: "2023-11-14T22:13:20Z",
      requestedDurationSeconds: 5, osVersion: "synthetic", architecture: "arm64", state: "saved",
      microphone: microphone, system: system, errors: [])
    manifest.microphoneDeviceUID = "fixture-microphone"
    manifest.captureTest = window.metadata(firstSavedCaptureNanoseconds: origin)
    try manifest.finalizeClock(clock)
    XCTAssertEqual(manifest.microphone.clockSegments?.first?.timelineStartUs, 0)
    XCTAssertEqual(manifest.microphone.clockSegments?.last?.timelineEndUs, 5_000_000)
    XCTAssertEqual(manifest.system.clockSegments?.first?.timelineStartUs, 100_000)
    XCTAssertEqual(manifest.system.clockSegments?.count, 2)
    XCTAssertEqual(manifest.system.clockSegments?[0].timelineEndUs, 1_900_000)
    XCTAssertEqual(manifest.system.clockSegments?[1].timelineStartUs, 2_900_000)
    for (name, stats, value) in [
      ("microphone.caf", microphone, Float(0.125)), ("system.caf", system, Float(0)),
    ] {
      let file = try AVAudioFile(forReading: directory.appendingPathComponent(name))
      XCTAssertEqual(file.length, stats.frames)
      let pcm = try XCTUnwrap(
        AVAudioPCMBuffer(
          pcmFormat: file.processingFormat, frameCapacity: AVAudioFrameCount(stats.frames)))
      try file.read(into: pcm)
      XCTAssertEqual(Int64(pcm.frameLength), stats.frames)
      for frame in 0..<Int(stats.frames) {
        XCTAssertEqual(pcm.floatChannelData![0][frame], value)
        XCTAssertEqual(pcm.floatChannelData![1][frame], -value)
      }
    }
    let bytes = try manifest.encodedData()
    let object = try XCTUnwrap(JSONSerialization.jsonObject(with: bytes) as? [String: Any])
    let metadata = try XCTUnwrap(object["captureTest"] as? [String: Any])
    XCTAssertEqual(metadata["ownerId"] as? String, owner)
    XCTAssertEqual(metadata["durationSeconds"] as? Int, 5)
    XCTAssertEqual(metadata["completed"] as? Bool, true)
    XCTAssertEqual(metadata["windowEndNs"] as? Int64, origin + 5_000_000_000)
    XCTAssertEqual(metadata["firstSavedCaptureNs"] as? Int64, origin)
    try bytes.write(to: directory.appendingPathComponent("manifest.json"), options: .atomic)
    if let destinationPath = ProcessInfo.processInfo.environment["MON_CAPTURE_TEST_FIXTURE_DIR"] {
      let destination = URL(fileURLWithPath: destinationPath, isDirectory: true)
      XCTAssertTrue(try FileManager.default.contentsOfDirectory(atPath: destinationPath).isEmpty)
      for name in ["manifest.json", "microphone.caf", "system.caf"] {
        try FileManager.default.copyItem(
          at: directory.appendingPathComponent(name), to: destination.appendingPathComponent(name))
      }
    }
    manifest.captureTest = nil
    let ordinary = try XCTUnwrap(
      JSONSerialization.jsonObject(with: manifest.encodedData()) as? [String: Any])
    XCTAssertNil(ordinary["captureTest"])
  }
}
