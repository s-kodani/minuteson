import XCTest

@testable import MONMacOSCapture

final class CapturePCMAlignmentTests: XCTestCase {
  func testLatePacketsKeepOneOffsetAndRealGapsAndNewBindingReset() throws {
    var alignment = CapturePCMAlignment()
    let first = try alignment.map(
      start: 1000, duration: 100, frames: 10, sourceEpoch: 1, emittedThrough: nil)
    XCTAssertEqual(first.start, 1000)
    let late = try alignment.map(
      start: 500, duration: 100, frames: 10, sourceEpoch: 1, emittedThrough: 1100)
    XCTAssertEqual(late.start, 1100)
    XCTAssertEqual(late.end, 1200)
    let next = try alignment.map(
      start: 600, duration: 100, frames: 10, sourceEpoch: 1, emittedThrough: 1200)
    XCTAssertEqual(next.start, 1200)
    let gap = try alignment.map(
      start: 900, duration: 100, frames: 10, sourceEpoch: 1, emittedThrough: 1300)
    XCTAssertEqual(gap.start, 1500)
    let rebound = try alignment.map(
      start: 1700, duration: 100, frames: 10, sourceEpoch: 2, emittedThrough: 1600)
    XCTAssertEqual(rebound.start, 1700)
    XCTAssertEqual(alignment.diagnostics?.clockAdjustments, 1)
    XCTAssertEqual(alignment.diagnostics?.correctionNanoseconds, 600)
    XCTAssertEqual(alignment.diagnostics?.retimedPackets, 3)
    XCTAssertThrowsError(
      try alignment.map(
        start: 1800, duration: 100, frames: 10, sourceEpoch: 1, emittedThrough: 1800))
  }

  func testUnemittedHaloCanFollowMeasuredDriftWithoutInventingACorrection() throws {
    var alignment = CapturePCMAlignment()
    _ = try alignment.map(
      start: 1000, duration: 100, frames: 10, sourceEpoch: 1, emittedThrough: nil)
    let next = try alignment.map(
      start: 1099, duration: 100, frames: 10, sourceEpoch: 1, emittedThrough: 1090)
    XCTAssertEqual(next.start, 1099)
    XCTAssertNil(alignment.diagnostics)
  }

  func testAnomalyHistoryIsBoundedAndOverflowRemainsAnError() throws {
    var alignment = CapturePCMAlignment()
    for index in 0..<100 {
      _ = try alignment.map(
        start: 1000, duration: 100, frames: 10, sourceEpoch: 1,
        emittedThrough: index == 0 ? nil : Int64(1000 + index * 100))
    }
    XCTAssertEqual(alignment.diagnostics?.firstAnomalies.count, 16)
    XCTAssertEqual(alignment.diagnostics?.truncated, true)
    XCTAssertEqual(alignment.diagnostics?.clockAdjustments, 99)
    XCTAssertThrowsError(
      try alignment.map(
        start: Int64.max, duration: 1, frames: 1, sourceEpoch: 1, emittedThrough: nil))
  }
}
