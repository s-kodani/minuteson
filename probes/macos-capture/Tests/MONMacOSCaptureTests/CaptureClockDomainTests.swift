import AVFoundation
import CoreMedia
import Foundation
import XCTest

@testable import MONMacOSCapture

final class ReadingBox: @unchecked Sendable {
  let lock = NSLock()
  var value: CaptureClockReading
  init(_ value: CaptureClockReading) { self.value = value }
  func read() -> CaptureClockReading { lock.withLock { value } }
  func set(_ value: CaptureClockReading) { lock.withLock { self.value = value } }
}

final class CaptureClockDomainTests: XCTestCase {
  private func reading(_ host: Int64, offset: Int64 = 500, utc: Int64 = 1_700_000_000_000)
    -> CaptureClockReading
  {
    CaptureClockReading(
      hostBeforeNs: host, continuousNs: host + offset + 10, utcMs: utc,
      hostAfterNs: host + 20)
  }

  func testMachConversionUsesFullWidthIntegersAndRejectsInvalidBounds() throws {
    XCTAssertEqual(try MachTimeUnits(numerator: 125, denominator: 3).nanoseconds(24), 1000)
    XCTAssertEqual(
      try MachTimeUnits(numerator: 2, denominator: 4).nanoseconds(UInt64.max), Int64.max)
    XCTAssertThrowsError(try MachTimeUnits(numerator: 0, denominator: 1).nanoseconds(0))
    XCTAssertThrowsError(try MachTimeUnits(numerator: 1, denominator: 0).nanoseconds(0))
    XCTAssertThrowsError(try MachTimeUnits(numerator: 2, denominator: 1).nanoseconds(UInt64.max))
    XCTAssertThrowsError(try MachTimeUnits(numerator: 1, denominator: 1).nanoseconds(UInt64.max))
  }

  func testCalibrationConvertsCaptureTimeNotArrivalAndKeepsFixedUTCReference() throws {
    let box = ReadingBox(reading(1_000_000))
    let clock = try CaptureClockDomain(units: MachTimeUnits(numerator: 1, denominator: 1)) {
      box.read()
    }
    box.set(reading(9_000_000, utc: 1_800_000_000_000))
    guard case .timestamp(let value) = clock.observe(hostTicks: 2_000_000) else {
      return XCTFail("capture timestamp was refused")
    }
    XCTAssertEqual(value, 2_000_500)
    let metadata = try clock.metadata(originNanoseconds: value)
    XCTAssertEqual(metadata.originUtcMs, 1_700_000_000_000)
    XCTAssertEqual(metadata.uncertaintyNs, 1_000_024)
    XCTAssertEqual(metadata.domain, "mach_continuous_time")
    // No device uptime or raw host ticks are exported.
    let json =
      try JSONSerialization.jsonObject(with: JSONEncoder().encode(metadata)) as! [String: Any]
    XCTAssertEqual(Set(json.keys), ["domain", "originUtcMs", "uncertaintyNs"])
  }

  func testSleepClockBridgeChangeIsStickyAndNeverRemapsQueuedObservations() throws {
    let box = ReadingBox(reading(1_000_000))
    let clock = try CaptureClockDomain(units: MachTimeUnits(numerator: 1, denominator: 1)) {
      box.read()
    }
    box.set(reading(2_000_000))
    let queued = clock.observe(hostTicks: 1_500_000)
    box.set(reading(3_000_000, offset: 10_000_500))
    guard case .invalid = clock.observe(hostTicks: 2_500_000) else {
      return XCTFail("changed clock bridge must not be accepted")
    }
    box.set(reading(4_000_000))
    guard case .invalid = clock.observe(hostTicks: 3_500_000) else {
      return XCTFail("clock failure must remain sticky")
    }
    guard case .timestamp(let value) = queued else { return XCTFail("old observation lost") }
    XCTAssertEqual(value, 1_500_500)
    XCTAssertThrowsError(try clock.metadata(originNanoseconds: value))
  }

  func testMissingPredomainFutureAndInvalidCalibrationAreRefused() throws {
    for ticks: UInt64? in [nil, 999, 1021] {
      let initial = reading(1000)
      let clock = try CaptureClockDomain(units: MachTimeUnits(numerator: 1, denominator: 1)) {
        initial
      }
      guard case .invalid = clock.observe(hostTicks: ticks) else {
        return XCTFail("invalid time accepted")
      }
      XCTAssertThrowsError(try clock.metadata(originNanoseconds: 1000))
    }
    XCTAssertThrowsError(
      try CaptureClockDomain(units: MachTimeUnits(numerator: 1, denominator: 1)) {
        CaptureClockReading(hostBeforeNs: 10, continuousNs: 0, utcMs: 1, hostAfterNs: 9)
      })
    XCTAssertThrowsError(
      try CaptureClockDomain(units: MachTimeUnits(numerator: 1, denominator: 1)) {
        CaptureClockReading(hostBeforeNs: 0, continuousNs: 0, utcMs: 1, hostAfterNs: Int64.max - 3)
      })
    let initial = reading(1000, offset: 0)
    let noSleep = try CaptureClockDomain(units: MachTimeUnits(numerator: 1, denominator: 1)) {
      initial
    }
    guard case .timestamp(let value) = noSleep.observe(hostTicks: 1000) else {
      return XCTFail("zero offset with signed calibration bounds must work")
    }
    XCTAssertEqual(value, 1000)
    XCTAssertThrowsError(try noSleep.metadata(originNanoseconds: -1))
  }

  func testLiveClockCalibrationAndMediaClockConversionWithoutCapture() throws {
    let clock = try CaptureClockDomain.live()
    let host = CMClockGetTime(CMClockGetHostTimeClock())
    guard
      case .timestamp(let value) = clock.observe(
        presentation: host, clock: CMClockGetHostTimeClock())
    else { return XCTFail("live clock conversion failed") }
    let metadata = try clock.metadata(originNanoseconds: value)
    XCTAssertLessThanOrEqual(
      abs(metadata.originUtcMs - Int64(Date().timeIntervalSince1970 * 1000)), 100)
    guard case .invalid = clock.observe(presentation: .invalid, clock: CMClockGetHostTimeClock())
    else { return XCTFail("invalid media time accepted") }
    guard case .invalid = clock.observe(presentation: host, clock: nil) else {
      return XCTFail("missing synchronization clock accepted")
    }
  }

  func testSavedFrameCountersDistinguishDriftFromMissingTime() throws {
    for drift: Int64 in [-1000, 1000] {
      var timing = try CaptureTrackTiming(sampleRate: 48000)
      for index in 0..<20 {
        try timing.recordSaved(
          frames: 480, observation: .timestamp(1_000_000_000 + Int64(index) * (10_000_000 + drift)),
          sourceFrame: Int64(index) * 480 - 960)
      }
      let maps = try timing.finish(originNanoseconds: 1_000_000_000, expectedFrames: 9600)
      XCTAssertEqual(maps.count, 1)
      XCTAssertEqual(maps[0].sourceFrameEnd, 9600)
    }
    var gap = try CaptureTrackTiming(sampleRate: 48000)
    try gap.recordSaved(frames: 480, observation: .timestamp(1_000_000_000), sourceFrame: 0)
    try gap.recordSaved(frames: 480, observation: .timestamp(1_030_000_000), sourceFrame: 1440)
    let maps = try gap.finish(originNanoseconds: 1_000_000_000, expectedFrames: 960)
    XCTAssertEqual(maps.count, 2)
    XCTAssertEqual(maps[0].timelineEndUs, 10000)
    XCTAssertEqual(maps[1].timelineStartUs, 30000)
    XCTAssertEqual(maps[1].sourceFrameStart, 480)
  }

  func testFailedClockProofIsStickyInSavedStatisticsAndNeverExportsPartialMaps() throws {
    for invalid in [CaptureClockObservation.invalid("missing timestamp"), .timestamp(900_000_000)] {
      var stats = TrackStats(frames: 960, sampleRate: 48000, channels: 2)
      stats.recordSavedClock(.timestamp(1_000_000_000), frames: 480, sourceFrame: 0)
      stats.recordSavedClock(invalid, frames: 480, sourceFrame: -480)
      stats.recordSavedClock(.timestamp(1_030_000_000), frames: 480, sourceFrame: 1440)
      XCTAssertNotNil(stats.clockFailure)
      XCTAssertThrowsError(try stats.finishClock(originNanoseconds: 1_000_000_000))
      XCTAssertNil(stats.clockSegments)
      XCTAssertEqual(stats.frames, 960)
      XCTAssertEqual(stats.appendFailures, 0)
    }
    var mismatch = try CaptureTrackTiming(sampleRate: 48000)
    try mismatch.recordSaved(frames: 480, observation: .timestamp(1000), sourceFrame: nil)
    XCTAssertThrowsError(try mismatch.finish(originNanoseconds: 1000, expectedFrames: 481))
    var impossibleGap = try CaptureTrackTiming(sampleRate: 48000)
    try impossibleGap.recordSaved(frames: 480, observation: .timestamp(1000), sourceFrame: 0)
    XCTAssertThrowsError(
      try impossibleGap.recordSaved(frames: 480, observation: .timestamp(2000), sourceFrame: 960))
    for rate in [Double.nan, .infinity, 7999, 384001, 48000.5] {
      XCTAssertThrowsError(try CaptureTrackTiming(sampleRate: rate))
    }
  }
}
