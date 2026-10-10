import AVFoundation
import Foundation
import XCTest

@testable import MONMacOSCapture

final class CaptureClockRecoveryTests: XCTestCase {
  private func reading(_ host: Int64, offset: Int64 = 500, utc: Int64 = 1_700_000_000_000)
    -> CaptureClockReading
  {
    CaptureClockReading(
      hostBeforeNs: host, continuousNs: host + offset + 10,
      utcMs: utc, hostAfterNs: host + 20)
  }

  private func clock(_ box: ReadingBox) throws -> CaptureClockDomain {
    try CaptureClockDomain(
      units: MachTimeUnits(numerator: 1, denominator: 1), allowsRecovery: true
    ) { box.read() }
  }

  private func request(_ clock: CaptureClockDomain) throws -> CaptureClockRecoveryRequest {
    try XCTUnwrap(clock.recoveryRequest())
  }

  private func resume(_ clock: CaptureClockDomain, _ request: CaptureClockRecoveryRequest) throws {
    try clock.resume(
      request, microphone: clock.quiesced(.microphone, for: request),
      system: clock.quiesced(.system, for: request))
  }

  func testMicrophoneCannotMintQuiescenceBeforeStartingItsConfirmedAcquisition() throws {
    let output = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    let box = ReadingBox(reading(1_000_000))
    let clock = try clock(box)
    let recorder = MicrophoneRecorder(
      outputURL: output.appendingPathComponent("microphone.caf"), clock: clock,
      microphoneUID: "confirmed-only", mayContinue: { false })
    box.set(reading(2_000_000, offset: 10_000_500))
    let ticket = try request(clock)
    XCTAssertThrowsError(try recorder.suspendForClockRecovery(ticket))
    recorder.maintain()
    XCTAssertEqual(recorder.snapshot().frames, 0)
    XCTAssertEqual(recorder.snapshot().appendFailures, 0)
    XCTAssertFalse(FileManager.default.fileExists(atPath: output.path))
  }

  func testPositiveBridgeChangeSuspendsAllPCMAndRequiresBothInputsFromTheSameClock() throws {
    let box = ReadingBox(reading(1_000_000))
    let first = try clock(box)
    let second = try clock(box)
    XCTAssertNil(try first.recoveryRequest())
    box.set(reading(2_000_000, offset: 10_000_500))
    guard case .suspended = first.observe(hostTicks: 1_500_000) else {
      return XCTFail("bridge change admitted audio")
    }
    let ticket = try request(first)
    XCTAssertEqual(try request(first), ticket)
    // During draining even an obsolete callback lacking host time is discarded.
    guard case .suspended = first.observe(hostTicks: nil) else { return XCTFail("not suspended") }
    XCTAssertThrowsError(try first.metadata(originNanoseconds: 1_000_500))
    let mic = try first.quiesced(.microphone, for: ticket)
    XCTAssertEqual(try first.quiesced(.microphone, for: ticket), mic)
    XCTAssertThrowsError(try first.resume(ticket, microphone: mic, system: mic))
    let otherTicket = try request(second)
    let otherSystem = try second.quiesced(.system, for: otherTicket)
    XCTAssertThrowsError(try first.quiesced(.system, for: otherTicket))
    XCTAssertThrowsError(try first.resume(ticket, microphone: mic, system: otherSystem))
    let system = try first.quiesced(.system, for: ticket)
    box.set(reading(3_000_000, offset: 10_000_500, utc: 1_600_000_000_000))
    try first.resume(ticket, microphone: mic, system: system)
    XCTAssertNil(try first.recoveryRequest())
    box.set(reading(4_000_000, offset: 10_000_500))
    guard case .timestamp(let time) = first.observe(hostTicks: 3_500_000) else {
      return XCTFail("new epoch refused")
    }
    XCTAssertEqual(time, 13_500_500)
    XCTAssertEqual(try first.metadata(originNanoseconds: 1_000_500).originUtcMs, 1_699_999_999_999)
    XCTAssertThrowsError(try first.resume(ticket, microphone: mic, system: system))
    XCTAssertThrowsError(try first.quiesced(.microphone, for: ticket))
  }

  func testOldHostTimestampCannotBeRemappedIntoResumedEpochAndFailureStaysTerminal() throws {
    let box = ReadingBox(reading(1_000_000))
    let clock = try clock(box)
    box.set(reading(2_000_000, offset: 20_000_500))
    let ticket = try request(clock)
    box.set(reading(3_000_000, offset: 20_000_500))
    try resume(clock, ticket)
    // This timestamp was acquired before the quiescent calibration bracket ended.
    guard case .invalid = clock.observe(hostTicks: 3_000_019) else {
      return XCTFail("old queued audio was assigned the new offset")
    }
    box.set(reading(4_000_000, offset: 20_000_500))
    guard case .invalid = clock.observe(hostTicks: 4_000_000) else { return XCTFail("not sticky") }
    XCTAssertThrowsError(try clock.recoveryRequest())
    XCTAssertThrowsError(try clock.close(.microphone))
  }

  func testEachSleepCycleRequiresFreshQuiescenceAndCalibration() throws {
    let box = ReadingBox(reading(1_000_000))
    let clock = try clock(box)
    box.set(reading(2_000_000, offset: 10_000_500))
    let first = try request(clock)
    let oldMic = try clock.quiesced(.microphone, for: first)
    let oldSystem = try clock.quiesced(.system, for: first)
    try clock.resume(first, microphone: oldMic, system: oldSystem)
    box.set(reading(3_000_000, offset: 30_000_500))
    let next = try request(clock)
    XCTAssertNotEqual(first, next)
    XCTAssertThrowsError(try clock.resume(next, microphone: oldMic, system: oldSystem))
    try resume(clock, next)
    box.set(reading(4_000_000, offset: 30_000_500))
    guard case .timestamp(let time) = clock.observe(hostTicks: 4_000_000) else {
      return XCTFail("second epoch failed")
    }
    XCTAssertEqual(time, 34_000_500)
  }

  func testSecondBridgeChangeOrBackwardsTimeDuringQuiescenceIsNotRecovered() throws {
    for next in [
      reading(3_000_000, offset: 20_000_500),
      reading(1_000_000, offset: 10_000_500),
      reading(3_000_000, offset: 500),
    ] {
      let box = ReadingBox(reading(1_000_000))
      let clock = try clock(box)
      box.set(reading(2_000_000, offset: 10_000_500))
      let ticket = try request(clock)
      let mic = try clock.quiesced(.microphone, for: ticket)
      let system = try clock.quiesced(.system, for: ticket)
      box.set(next)
      XCTAssertThrowsError(try clock.resume(ticket, microphone: mic, system: system))
      XCTAssertThrowsError(try clock.recoveryRequest())
      box.set(reading(4_000_000, offset: 10_000_500))
      XCTAssertThrowsError(try clock.resume(ticket, microphone: mic, system: system))
    }
  }

  func testRecoveryDoesNotAdoptNegativeBridgeChangesBadTimestampsOrClosedAcquisitions() throws {
    for next in [reading(2_000_000, offset: 0), reading(500_000)] {
      let box = ReadingBox(reading(1_000_000))
      let clock = try clock(box)
      box.set(next)
      XCTAssertThrowsError(try clock.recoveryRequest())
      box.set(reading(3_000_000, offset: 10_000_500))
      XCTAssertThrowsError(try clock.recoveryRequest())
    }
    for ticks: UInt64? in [nil, 999_999, 1_000_021] {
      let box = ReadingBox(reading(1_000_000))
      let clock = try clock(box)
      guard case .invalid = clock.observe(hostTicks: ticks) else {
        return XCTFail("bad host admitted")
      }
      box.set(reading(2_000_000, offset: 10_000_500))
      XCTAssertThrowsError(try clock.recoveryRequest())
    }
    let box = ReadingBox(reading(1_000_000))
    let clock = try clock(box)
    _ = try clock.close(.microphone)
    box.set(reading(2_000_000, offset: 10_000_500))
    XCTAssertThrowsError(try clock.recoveryRequest())
    XCTAssertThrowsError(try clock.close(.system))
  }

  func testSuspendedEpochCannotMintSavedEndingOrBecomeNormalByReturningToOldOffset() throws {
    let box = ReadingBox(reading(1_000_000))
    let clock = try clock(box)
    box.set(reading(2_000_000, offset: 10_000_500))
    let ticket = try request(clock)
    box.set(reading(3_000_000))
    guard case .suspended = clock.observe(hostTicks: 3_000_000) else {
      return XCTFail("changed bridge recovered without quiescence")
    }
    XCTAssertEqual(try request(clock), ticket)
    XCTAssertThrowsError(try clock.close(.microphone))
    XCTAssertThrowsError(try clock.recoveryRequest())
    XCTAssertThrowsError(try clock.quiesced(.system, for: ticket))
    guard case .invalid = clock.observe(hostTicks: 3_000_000) else {
      return XCTFail("terminal ending failure was hidden as a recoverable pause")
    }
  }

  func testRealCAFAppendsAcrossSleepEpochWithoutFillingGapOrChangingSavedPCM() throws {
    let output = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: output, withIntermediateDirectories: false)
    defer { try? FileManager.default.removeItem(at: output) }
    let format = try XCTUnwrap(AVAudioFormat(standardFormatWithSampleRate: 48000, channels: 1))
    let path = output.appendingPathComponent("microphone.caf")
    var file: AVAudioFile? = try AVAudioFile(
      forWriting: path, settings: format.settings, commonFormat: .pcmFormatFloat32,
      interleaved: false)
    let buffer = try XCTUnwrap(AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 480))
    buffer.frameLength = 480
    let samples = try XCTUnwrap(buffer.floatChannelData)[0]
    for index in 0..<480 { samples[index] = Float(index) / 480 }
    let box = ReadingBox(reading(1_000_000_000))
    let clock = try clock(box)
    var timing = try CaptureTrackTiming(sampleRate: 48000)
    let origin: Int64 = 1_010_000_500
    box.set(reading(1_020_000_000))
    try timing.recordSaved(
      frames: 480, observation: clock.observe(hostTicks: 1_010_000_000),
      sourceFrame: 0, sourceEpoch: 1)
    try file?.write(from: buffer)
    box.set(reading(1_030_000_000))
    try timing.recordSaved(
      frames: 480, observation: clock.observe(hostTicks: 1_020_000_000),
      sourceFrame: 480, sourceEpoch: 1)
    try file?.write(from: buffer)
    box.set(reading(1_040_000_000, offset: 10_000_000_500, utc: 1_600_000_000_000))
    let ticket = try request(clock)
    guard case .suspended = clock.observe(hostTicks: 1_030_000_000) else {
      return XCTFail("queued audio accepted while paused")
    }
    // A production owner must gate writes before passing suspended observations
    // to timing/writers. This harness models that gate, not AVAudioEngine/SCStream.
    box.set(reading(1_050_000_000, offset: 10_000_000_500))
    try resume(clock, ticket)
    box.set(reading(1_070_000_000, offset: 10_000_000_500))
    try timing.recordSaved(
      frames: 480, observation: clock.observe(hostTicks: 1_060_000_000),
      sourceFrame: 0, sourceEpoch: 2)
    try file?.write(from: buffer)
    file = nil
    let maps = try timing.finish(originNanoseconds: origin, expectedFrames: 1440)
    XCTAssertEqual(maps.count, 2)
    XCTAssertEqual(maps[0].timelineStartUs, 0)
    XCTAssertEqual(maps[0].timelineEndUs, 20_000)
    XCTAssertEqual(maps[1].timelineStartUs, 10_050_000)
    XCTAssertEqual(maps[1].timelineEndUs, 10_060_000)
    XCTAssertEqual(maps[1].sourceFrameStart, 960)
    XCTAssertEqual(maps[1].sourceFrameEnd, 1440)
    let saved = try AVAudioFile(forReading: path)
    XCTAssertEqual(saved.length, 1440)
    let pcm = try XCTUnwrap(
      AVAudioPCMBuffer(pcmFormat: saved.processingFormat, frameCapacity: 1440))
    try saved.read(into: pcm)
    XCTAssertEqual(pcm.frameLength, 1440)
    let actual = try XCTUnwrap(pcm.floatChannelData)[0]
    for index in 0..<1440 { XCTAssertEqual(actual[index], Float(index % 480) / 480) }
    let mic = try clock.close(.microphone)
    box.set(reading(1_080_000_000, offset: 10_000_000_500, utc: 1_900_000_000_000))
    let system = try clock.close(.system)
    let (metadata, ending) = try clock.ending(
      originNanoseconds: origin, microphone: mic, system: system)
    XCTAssertEqual(metadata.originUtcMs, 1_700_000_000_009)
    XCTAssertEqual(ending.endUs, 10_070_000)
    XCTAssertEqual(ending.microphoneClosedUs, 10_060_000)
  }
}
