import Foundation
import XCTest

@testable import MONMacOSCapture

final class CapturedClockMapTests: XCTestCase {
  func testFourHourFragmentedTrackExceedsLegacyCapacityWithoutLosingMeasuredGaps() throws {
    var clock = try CapturedClockMap(sampleRate: 48000, maximumSegments: 65_536)
    let origin: Int64 = 123_000_000_000
    let count = 675_000
    for index in 0..<count {
      let gap = Int64(index / 100) * 1_000_000
      let start = origin + Int64(index) * 21_333_333 + gap
      try clock.append(
        frames: 1024, captureStartNanoseconds: start, captureEndNanoseconds: start + 21_333_333,
        discontinuity: index > 0 && index % 100 == 0)
    }
    let maps = try clock.finish(originNanoseconds: origin)
    XCTAssertEqual(maps.count, 6750)
    XCTAssertEqual(maps.last?.sourceFrameEnd, 691_200_000)
    for pair in zip(maps, maps.dropFirst()) {
      XCTAssertEqual(pair.0.sourceFrameEnd, pair.1.sourceFrameStart)
      XCTAssertEqual(pair.1.timelineStartUs - pair.0.timelineEndUs, 1000)
    }
    XCTAssertLessThan(try JSONEncoder().encode(maps).count, 2 * 1024 * 1024)
  }

  func testTrackCapacityIsFiniteAndDoesNotWeakenDefaultRawLimit() throws {
    for limit in [1024, CapturedClockMap.maximumTrackSegments] {
      var clock = try CapturedClockMap(sampleRate: 8000, maximumSegments: limit)
      for index in 0..<limit {
        try clock.append(
          frames: 8, captureStartNanoseconds: Int64(index) * 2_000_000,
          captureEndNanoseconds: Int64(index) * 2_000_000 + 1_000_000, discontinuity: index > 0)
      }
      XCTAssertThrowsError(
        try clock.append(
          frames: 8, captureStartNanoseconds: Int64(limit) * 2_000_000,
          captureEndNanoseconds: Int64(limit) * 2_000_000 + 1_000_000, discontinuity: true))
      XCTAssertEqual(clock.frames, Int64(limit) * 8)
      XCTAssertThrowsError(try clock.finish(originNanoseconds: 0))
    }
  }
  func testConstantAndDriftingClocksCompressAndKeepSharedNonzeroOrigin() throws {
    for multiplier in [1.0, 1.0001, 0.9999] {
      var clock = try CapturedClockMap(sampleRate: 48000)
      let origin: Int64 = 9_000_000_000_000
      let delay: Int64 = 123_456_000
      for index in 0..<1000 {
        let start =
          origin + delay + Int64((Double(index * 1024) * 1e9 / 48000 * multiplier).rounded())
        let end =
          origin + delay + Int64((Double((index + 1) * 1024) * 1e9 / 48000 * multiplier).rounded())
        try clock.append(frames: 1024, captureStartNanoseconds: start, captureEndNanoseconds: end)
      }
      let segments = try clock.finish(originNanoseconds: origin)
      XCTAssertEqual(segments.count, 1)
      XCTAssertEqual(segments[0].sourceFrameStart, 0)
      XCTAssertEqual(segments[0].sourceFrameEnd, 1_024_000)
      XCTAssertEqual(segments[0].timelineStartUs, 123456)
      XCTAssertEqual(clock.frames, 1_024_000)
      let json =
        try JSONSerialization.jsonObject(with: JSONEncoder().encode(segments[0])) as? [String: Any]
      XCTAssertEqual(
        json?["mappingJson"] as? String, "{\"schema_version\":1,\"method\":\"linear\"}")
    }
  }

  func testFourHoursOfCallbacksStayBoundedAndPreserveEveryObservation() throws {
    var clock = try CapturedClockMap(sampleRate: 48000)
    let count = 675_000
    let origin: Int64 = 123_000_000_000
    for index in 0..<count {
      let start = origin + Int64(index) * 21_335_466
      try clock.append(
        frames: 1024, captureStartNanoseconds: start, captureEndNanoseconds: start + 21_335_466)
    }
    let segments = try clock.finish(originNanoseconds: origin)
    XCTAssertEqual(segments.count, 1)
    XCTAssertEqual(segments[0].sourceFrameEnd, 691_200_000)
    XCTAssertLessThan(try JSONEncoder().encode(segments).count, 256)
    for index in 0...count {
      let estimated =
        Double(index * 1024) * Double(segments[0].timelineEndUs) * 1000 / Double(clock.frames)
      let observed = Double(index) * 21_335_466
      XCTAssertLessThanOrEqual(
        abs(estimated - observed), Double(clock.maximumErrorNanoseconds + 1000))
    }
  }

  func testChangingSlopeKeepsObservedEndpointsAndBoundsInteriorError() throws {
    var clock = try CapturedClockMap(sampleRate: 48000)
    let origin: Int64 = 1_000_000
    func stamp(_ frame: Int64) -> Int64 {
      origin + frame * 20_833 + max(0, frame - 51200) * 10
    }
    for index in 0..<100 {
      let frame = Int64(index) * 1024
      try clock.append(
        frames: 1024, captureStartNanoseconds: stamp(frame),
        captureEndNanoseconds: stamp(frame + 1024))
    }
    let segments = try clock.finish(originNanoseconds: origin)
    XCTAssertGreaterThan(segments.count, 1)
    XCTAssertEqual(segments.first?.sourceFrameStart, 0)
    XCTAssertEqual(segments.last?.sourceFrameEnd, 102400)
    for index in 0...100 {
      let frame = Int64(index) * 1024
      let segment = try XCTUnwrap(
        segments.first { frame >= $0.sourceFrameStart && frame <= $0.sourceFrameEnd })
      let fraction =
        Double(frame - segment.sourceFrameStart)
        / Double(segment.sourceFrameEnd - segment.sourceFrameStart)
      let estimated =
        Double(segment.timelineStartUs) * 1000 + fraction
        * Double(segment.timelineEndUs - segment.timelineStartUs) * 1000
      XCTAssertLessThanOrEqual(
        abs(estimated - Double(stamp(frame) - origin)), Double(clock.maximumErrorNanoseconds + 1000)
      )
    }
  }

  func testExplicitDiscontinuityPreservesMissingTimeWithoutFabricatedFrames() throws {
    var clock = try CapturedClockMap(sampleRate: 48000)
    try clock.append(
      frames: 48000, captureStartNanoseconds: 1_000_000_000, captureEndNanoseconds: 2_000_000_000)
    try clock.append(
      frames: 48000, captureStartNanoseconds: 5_000_000_000, captureEndNanoseconds: 6_000_000_000,
      discontinuity: true)
    let segments = try clock.finish(originNanoseconds: 1_000_000_000)
    XCTAssertEqual(segments.count, 2)
    XCTAssertEqual(segments[0].sourceFrameEnd, segments[1].sourceFrameStart)
    XCTAssertEqual(segments[0].timelineEndUs, 1_000_000)
    XCTAssertEqual(segments[1].timelineStartUs, 4_000_000)
    XCTAssertEqual(segments[1].timelineEndUs, 5_000_000)
    XCTAssertEqual(clock.frames, 96000)
  }

  func testInvalidInputAndFinalizationNeverPublishPartialMappings() throws {
    XCTAssertThrowsError(try CapturedClockMap(sampleRate: 7999))
    XCTAssertThrowsError(try CapturedClockMap(sampleRate: 48000, maximumSegments: 65_537))
    for (frames, start, end) in [
      (Int64(0), Int64(0), Int64(1)), (1, -1, 1), (1, 1, 1), (Int64.max, 0, 1),
    ] {
      var clock = try CapturedClockMap(sampleRate: 48000)
      XCTAssertThrowsError(
        try clock.append(frames: frames, captureStartNanoseconds: start, captureEndNanoseconds: end)
      )
      XCTAssertThrowsError(try clock.finish(originNanoseconds: 0))
    }
    var clock = try CapturedClockMap(sampleRate: 48000)
    try clock.append(
      frames: 48000, captureStartNanoseconds: 1000, captureEndNanoseconds: 1_000_001_000)
    XCTAssertThrowsError(try clock.finish(originNanoseconds: 1001))
    let segments = try clock.finish(originNanoseconds: 1000)
    XCTAssertEqual(segments.count, 1)
    XCTAssertThrowsError(try clock.finish(originNanoseconds: 1000))
    XCTAssertThrowsError(
      try clock.append(
        frames: 1, captureStartNanoseconds: 2_000_000_000, captureEndNanoseconds: 2_000_030_000))
  }

  func testCapacityAndReversedDiscontinuityRefusePartialClockProof() throws {
    var clock = try CapturedClockMap(sampleRate: 48000, maximumSegments: 1)
    try clock.append(
      frames: 48000, captureStartNanoseconds: 0, captureEndNanoseconds: 1_000_000_000)
    XCTAssertThrowsError(
      try clock.append(
        frames: 48000, captureStartNanoseconds: 2_000_000_000,
        captureEndNanoseconds: 3_000_000_000, discontinuity: true))
    XCTAssertThrowsError(try clock.finish(originNanoseconds: 0))
    var reversed = try CapturedClockMap(sampleRate: 48000)
    try reversed.append(
      frames: 48000, captureStartNanoseconds: 0, captureEndNanoseconds: 1_000_000_000)
    XCTAssertThrowsError(
      try reversed.append(
        frames: 48000, captureStartNanoseconds: 999_999_999, captureEndNanoseconds: 2_000_000_000,
        discontinuity: true))
    XCTAssertThrowsError(try reversed.finish(originNanoseconds: 0))
  }
}
