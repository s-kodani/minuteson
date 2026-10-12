import Foundation
import XCTest

@testable import MONMacOSCapture

final class CaptureFailureDetailTests: XCTestCase {
  func testOnlyPrivateAudioOnlyTestChangesInputPreparationOrder() throws {
    let owner = "735027d4-a03e-43d3-830a-21e767dfaf88"
    let base = [
      "--output", "/tmp/unused-synthetic-startup", "--stop-on-file", "--parent-pid", "123",
      "--microphone-uid", "frozen-mic", "--backend", "coreaudio",
    ]
    let test = try Options.parse(base + ["--five-second-test", owner])
    XCTAssertTrue(test.startsSystemBeforeMicrophone)
    let normal = try Options.parse(base + ["--until-stopped", "--input-level-owner", owner])
    XCTAssertFalse(normal.startsSystemBeforeMicrophone)
    let measurement = try Options.parse([
      "--output", "/tmp/unused-synthetic-startup", "--duration", "5", "--backend", "coreaudio",
    ])
    XCTAssertFalse(measurement.startsSystemBeforeMicrophone)
    let screenTest = try Options.parse(base.dropLast(2) + ["--five-second-test", owner])
    XCTAssertFalse(screenTest.startsSystemBeforeMicrophone)
  }

  func testFirstFailureIsBoundedRetainedAndDoesNotChangeTrackSchemaOrInventFailure() throws {
    var stats = TrackStats()
    stats.retainCaptureFailure(ProbeError.capture(" \n"))
    XCTAssertNil(stats.captureFailureDetail)
    let reason = "synthetic original: " + String(repeating: "詳", count: 2000)
    stats.retainCaptureFailure(ProbeError.capture(reason))
    XCTAssertEqual(stats.captureFailureDetail, String(reason.prefix(1200)))
    XCTAssertNoThrow(try stats.requireWritable())
    stats.appendFailures = 1
    stats.retainCaptureFailure(ProbeError.capture("later cleanup failure"))
    XCTAssertThrowsError(try stats.requireWritable()) { error in
      let message = String(describing: error)
      XCTAssertTrue(message.contains("capture_audio_write_failed"))
      XCTAssertTrue(message.contains("synthetic original"))
      XCTAssertFalse(message.contains("later cleanup failure"))
    }
    let encoded = try JSONEncoder().encode(stats)
    let fields = try XCTUnwrap(JSONSerialization.jsonObject(with: encoded) as? [String: Any])
    XCTAssertNil(fields["captureFailureDetail"])
    XCTAssertEqual(fields["appendFailures"] as? Int, 1)
  }

  func testWriterRetainsReasonWithoutDoubleCountingAndRejectsFurtherPCM() throws {
    let writer = SystemPCMWriter(
      outputURL: FileManager.default.temporaryDirectory.appendingPathComponent(
        "unused-synthetic-\(UUID()).caf"))
    writer.recordFailure(ProbeError.capture("synthetic first callback failure"))
    writer.retainFailure(ProbeError.capture("secondary write failure"))
    XCTAssertEqual(writer.stats.appendFailures, 1)
    XCTAssertEqual(writer.stats.captureFailureDetail, "synthetic first callback failure")
    XCTAssertFalse(writer.ready)
    XCTAssertThrowsError(try writer.stats.requireWritable()) { error in
      XCTAssertTrue(String(describing: error).contains("synthetic first callback failure"))
    }
  }
}
