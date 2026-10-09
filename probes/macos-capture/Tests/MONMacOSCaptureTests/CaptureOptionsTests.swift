import Foundation
import XCTest

@testable import MONMacOSCapture

final class CaptureOptionsTests: XCTestCase {
  private let output = ["--output", "/tmp/mon-capture-options-test"]
  private let controlled = ["--stop-on-file", "--parent-pid", "1234"]

  func testTestRunnerFlagsRemainUnknownCaptureArguments() {
    for arguments in [
      ["--test-bundle-path", "/tmp/mon-test-bundle"],
      ["--testing-library", "swift-testing"],
    ] {
      XCTAssertThrowsError(try Options.parse(output + arguments))
    }
  }

  func testDefaultAndExplicitProbeDurationRemainBounded() throws {
    let startedAt = ContinuousClock.now
    for (arguments, seconds) in [([], 30.0), (["--duration", "14400"], 14400.0)] {
      let options = try Options.parse(output + arguments)
      XCTAssertEqual(options.duration, seconds)
      XCTAssertFalse(options.stopOnFile)
      XCTAssertTrue(
        options.shouldContinueRecording(
          startedAt: startedAt, now: startedAt.advanced(by: .seconds(seconds - 1)),
          stopRequested: false, parentExited: false))
      XCTAssertFalse(
        options.shouldContinueRecording(
          startedAt: startedAt, now: startedAt.advanced(by: .seconds(seconds)),
          stopRequested: false, parentExited: false))
    }
  }

  func testBoundedFileStopStillHonorsDeadlineStopAndParentExit() throws {
    let options = try Options.parse(output + controlled + ["--duration", "900"])
    let startedAt = ContinuousClock.now
    XCTAssertEqual(options.duration, 900)
    XCTAssertTrue(options.stopOnFile)
    XCTAssertEqual(options.parentPID, 1234)
    for (seconds, stop, exited, expected) in [
      (899, false, false, true), (900, false, false, false),
      (1, true, false, false), (1, false, true, false), (1, true, true, false),
    ] {
      XCTAssertEqual(
        options.shouldContinueRecording(
          startedAt: startedAt, now: startedAt.advanced(by: .seconds(seconds)),
          stopRequested: stop, parentExited: exited), expected)
    }
  }

  func testUntilStoppedContinuesBeyondFourHoursButStopsOnEitherControl() throws {
    let options = try Options.parse(output + controlled + ["--until-stopped"])
    XCTAssertNil(options.duration)
    XCTAssertTrue(options.stopOnFile)
    XCTAssertEqual(options.parentPID, 1234)
    let startedAt = ContinuousClock.now
    for seconds in [0, 14400, 14401, 28800, 172800] {
      let now = startedAt.advanced(by: .seconds(seconds))
      XCTAssertTrue(
        options.shouldContinueRecording(
          startedAt: startedAt, now: now, stopRequested: false, parentExited: false))
      for (stop, exited) in [(true, false), (false, true), (true, true)] {
        XCTAssertFalse(
          options.shouldContinueRecording(
            startedAt: startedAt, now: now, stopRequested: stop, parentExited: exited))
      }
    }
  }

  func testUntilStoppedRequiresBothControlsAndRejectsDurationInEitherOrder() {
    for arguments in [
      ["--until-stopped"], ["--until-stopped", "--stop-on-file"],
      ["--until-stopped", "--parent-pid", "1234"],
      controlled + ["--until-stopped", "--duration", "30"],
      ["--duration", "30", "--until-stopped"] + controlled,
      ["--stop-on-file", "--parent-pid", "0", "--until-stopped"],
      ["--stop-on-file", "--parent-pid", "-1", "--until-stopped"],
    ] {
      XCTAssertThrowsError(try Options.parse(output + arguments))
    }
  }

  func testProbeDurationAndCaptureSelectionValidationRemainStrict() {
    for value in ["0", "-1", "14401", "nan", "inf", "-inf", "invalid"] {
      XCTAssertThrowsError(try Options.parse(output + ["--duration", value]))
    }
    XCTAssertThrowsError(try Options.parse(output + ["--parent-pid", "1234"]))
    for arguments in [
      ["--mode", "application"], ["--pid", "1234"],
      ["--backend", "coreaudio"],
      ["--mode", "application", "--pid", "1234", "--pid", "5678"],
      ["--mode", "application", "--backend", "coreaudio", "--pid", "1234", "--pid", "1234"],
    ] {
      XCTAssertThrowsError(try Options.parse(output + controlled + ["--until-stopped"] + arguments))
    }
    let selected =
      output + controlled + [
        "--until-stopped", "--mode", "application", "--backend", "coreaudio",
        "--pid", "1234", "--pid", "5678",
      ]
    XCTAssertNoThrow(try Options.parse(selected))
  }

  func testMicrophoneUIDRemainsOneBoundedArgumentAndDoesNotChangeStopControls() throws {
    let uid = "USB input/名前 with spaces"
    let selected = try Options.parse(
      output + controlled + ["--until-stopped", "--microphone-uid", uid])
    XCTAssertEqual(selected.microphoneUID, uid)
    XCTAssertNil(selected.duration)
    XCTAssertEqual(selected.parentPID, 1234)
    XCTAssertNil(try Options.parse(output).microphoneUID)
    for arguments in [
      ["--microphone-uid"], ["--microphone-uid", ""], ["--microphone-uid", "a\nb"],
      ["--microphone-uid", String(repeating: "x", count: 1025)],
      ["--microphone-uid", uid, "--microphone-uid", uid],
    ] {
      XCTAssertThrowsError(try Options.parse(output + arguments))
    }
  }

  func testApplicationIdentityRequiresTheExactTargetAndDoesNotAlterLegacyProbeOptions() throws {
    let identity = CaptureApplicationIdentity(
      pid: 123, bundleIdentifier: "com.example.browser", launchTimeUs: 1_791_100_000_000_001)
    let json = String(decoding: try JSONEncoder().encode(identity), as: UTF8.self)
    let args =
      output + controlled + [
        "--until-stopped", "--mode", "application", "--pid", "123", "--application-identity", json,
      ]
    let options = try Options.parse(args)
    XCTAssertEqual(options.applicationIdentity, identity)
    XCTAssertNil(options.duration)
    XCTAssertNil(
      try Options.parse(output + ["--mode", "application", "--pid", "123"]).applicationIdentity)
    for extra in [
      ["--application-identity", json],
      ["--mode", "application", "--pid", "456", "--application-identity", json],
      [
        "--mode", "application", "--pid", "123", "--backend", "coreaudio", "--application-identity",
        json,
      ],
      [
        "--mode", "application", "--pid", "123", "--application-identity", json,
        "--application-identity", json,
      ],
      ["--application-identity", "{}"],
      ["--application-identity", String(repeating: "x", count: 4097)],
    ] { XCTAssertThrowsError(try Options.parse(output + extra)) }
  }
}
