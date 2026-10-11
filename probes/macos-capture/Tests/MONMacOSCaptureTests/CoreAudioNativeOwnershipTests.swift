import AVFoundation
import CoreAudio
import Foundation
import XCTest

@testable import MONMacOSCapture

private final class SilentTapHAL: CoreAudioTapHAL, @unchecked Sendable {
  private let lock = NSLock()
  private var failedPhase: String?
  private var events: [String] = []
  private var resources: Set<String> = []
  private var selectedPID: pid_t = 123
  private var changedFormat = false
  private var startResult: OSStatus = noErr
  private var stopResult: OSStatus = noErr
  var changePIDDuringIOCreation = false
  func fail(_ phase: String?) { lock.withLock { failedPhase = phase } }
  func setStartStatus(_ value: OSStatus) { lock.withLock { startResult = value } }
  func setStopStatus(_ value: OSStatus) { lock.withLock { stopResult = value } }
  func changeFormat() { lock.withLock { changedFormat = true } }
  var trace: [String] { lock.withLock { events } }
  var owned: Set<String> { lock.withLock { resources } }

  private func phase(_ name: String) throws {
    try lock.withLock {
      events.append(name)
      if failedPhase == name { throw ProbeError.capture("injected native \(name) failure") }
    }
  }
  func processes() throws -> [(pid: pid_t, objectID: AudioObjectID)] { [(123, 21), (456, 22)] }
  func createTap(_ description: CATapDescription) throws -> AudioObjectID {
    try phase("tap")
    _ = lock.withLock { resources.insert("tap") }
    return 31
  }
  func tapUID(_ tap: AudioObjectID) throws -> String {
    try phase("uid")
    return "fixture-tap"
  }
  func createAggregate(tapUID: String) throws -> AudioObjectID {
    try phase("aggregate")
    _ = lock.withLock { resources.insert("aggregate") }
    return 41
  }
  func format(_ tap: AudioObjectID) throws -> AudioStreamBasicDescription {
    try phase("format")
    let format = AVAudioFormat(
      commonFormat: .pcmFormatFloat32,
      sampleRate: lock.withLock { changedFormat ? 44_100 : 48_000 }, channels: 2, interleaved: false
    )!
    return format.streamDescription.pointee
  }
  func processPID(_ object: AudioObjectID) throws -> pid_t { lock.withLock { selectedPID } }
  func alive(_ device: AudioObjectID) throws -> UInt32 { 1 }
  func createIO(
    _ device: AudioObjectID, queue: DispatchQueue,
    block: @escaping AudioDeviceIOBlock
  ) throws -> AudioDeviceIOProcID {
    try phase("io")
    lock.withLock {
      resources.insert("io")
      if changePIDDuringIOCreation { selectedPID = 456 }
    }
    return { _, _, _, _, _, _, _ in noErr }
  }
  func start(_ device: AudioObjectID, io: AudioDeviceIOProcID) -> OSStatus {
    lock.withLock {
      events.append("start")
      return failedPhase == "start" ? kAudioHardwareNotReadyError : startResult
    }
  }
  func stop(_ device: AudioObjectID, io: AudioDeviceIOProcID) -> OSStatus {
    lock.withLock {
      events.append("stop")
      return stopResult
    }
  }
  func destroyIO(_ device: AudioObjectID, io: AudioDeviceIOProcID) throws {
    try phase("destroyIO")
    _ = lock.withLock { resources.remove("io") }
  }
  func destroyAggregate(_ device: AudioObjectID) throws {
    try phase("destroyAggregate")
    _ = lock.withLock { resources.remove("aggregate") }
  }
  func destroyTap(_ tap: AudioObjectID) throws {
    try phase("destroyTap")
    _ = lock.withLock { resources.remove("tap") }
  }
}

final class CoreAudioNativeOwnershipTests: XCTestCase, @unchecked Sendable {
  private let identity = CaptureApplicationIdentity(
    pid: 123, bundleIdentifier: "com.example.confirmed", launchTimeUs: 1_791_100_000_000_001)

  func testEveryPartialInitializationIsCleanedInOwnershipOrderWithoutHardware() throws {
    for phase in ["tap", "uid", "aggregate", "format", "io", "start"] {
      let hal = SilentTapHAL()
      hal.fail(phase)
      let driver = NativeCoreAudioCaptureDriver(scope: .system, hal: hal)
      XCTAssertThrowsError(try driver.start(sample: { _ in }, failure: { _ in }), phase)
      try driver.stop()
      try driver.detach()
      XCTAssertTrue(hal.owned.isEmpty, phase)
      let destroyed = hal.trace.filter { $0.hasPrefix("destroy") }
      let expected: [String]
      switch phase {
      case "tap": expected = []
      case "uid", "aggregate": expected = ["destroyTap"]
      case "format", "io": expected = ["destroyAggregate", "destroyTap"]
      default: expected = ["destroyIO", "destroyAggregate", "destroyTap"]
      }
      XCTAssertEqual(destroyed, expected, phase)
      XCTAssertThrowsError(try driver.start(sample: { _ in }, failure: { _ in }))
    }
  }

  func testStopFailureRetainsLiveIOAndPreventsAggregateDestruction() throws {
    let hal = SilentTapHAL()
    let driver = NativeCoreAudioCaptureDriver(scope: .system, hal: hal)
    try driver.start(sample: { _ in }, failure: { _ in })
    hal.setStopStatus(kAudioHardwareUnspecifiedError)
    XCTAssertThrowsError(try driver.stop())
    XCTAssertThrowsError(try driver.detach())
    XCTAssertEqual(hal.owned, ["tap", "aggregate", "io"])
    XCTAssertFalse(hal.trace.contains("destroyIO"))
    hal.setStopStatus(noErr)
    try driver.stop()
    try driver.detach()
    XCTAssertTrue(hal.owned.isEmpty)
  }

  func testDestroyFailuresRetainExactResourceAndRetryWithoutDuplicateSuccessfulDestruction() throws
  {
    for phase in ["destroyIO", "destroyAggregate", "destroyTap"] {
      let hal = SilentTapHAL()
      let driver = NativeCoreAudioCaptureDriver(scope: .system, hal: hal)
      try driver.start(sample: { _ in }, failure: { _ in })
      hal.fail(phase)
      if phase == "destroyIO" {
        XCTAssertThrowsError(try driver.stop())
        XCTAssertThrowsError(try driver.detach())
        XCTAssertEqual(hal.owned, ["tap", "aggregate", "io"])
      } else {
        try driver.stop()
        XCTAssertThrowsError(try driver.detach())
        XCTAssertEqual(hal.owned, phase == "destroyAggregate" ? ["tap", "aggregate"] : ["tap"])
      }
      hal.fail(nil)
      try driver.stop()
      try driver.detach()
      try driver.detach()
      XCTAssertTrue(hal.owned.isEmpty)
      for item in ["destroyIO", "destroyAggregate", "destroyTap"] {
        XCTAssertEqual(hal.trace.filter { $0 == item }.count, item == phase ? 2 : 1)
      }
    }
  }

  func testExplicitPermissionFailureKeepsOriginalReasonAndStillCleansOnlyOwnedResources() throws {
    let hal = SilentTapHAL()
    hal.setStartStatus(kAudioDevicePermissionsError)
    let driver = NativeCoreAudioCaptureDriver(scope: .system, hal: hal)
    do {
      try driver.start(sample: { _ in }, failure: { _ in })
      XCTFail("permission rejection")
    } catch ProbeError.permission(let message) {
      XCTAssertTrue(message.contains("システムオーディオ録音のみ"))
      XCTAssertFalse(message.contains("ScreenCaptureKit"))
    }
    hal.setStopStatus(kAudioHardwareNotRunningError)
    try driver.stop()
    try driver.detach()
    XCTAssertTrue(hal.owned.isEmpty)
  }

  func testNotRunningErrorCannotProveStopForAStartedAcquisition() throws {
    let hal = SilentTapHAL()
    let driver = NativeCoreAudioCaptureDriver(scope: .system, hal: hal)
    try driver.start(sample: { _ in }, failure: { _ in })
    hal.setStopStatus(kAudioHardwareNotRunningError)
    XCTAssertThrowsError(try driver.stop())
    XCTAssertThrowsError(try driver.detach())
    hal.setStopStatus(noErr)
    try driver.stop()
    try driver.detach()
  }

  func testReusedAudioObjectCannotStartOrFallbackToGlobalCapture() throws {
    let hal = SilentTapHAL()
    hal.changePIDDuringIOCreation = true
    let driver = NativeCoreAudioCaptureDriver(
      scope: .application(identity), hal: hal,
      requireIdentity: { _ in })
    XCTAssertThrowsError(try driver.start(sample: { _ in }, failure: { _ in }))
    XCTAssertFalse(hal.trace.contains("start"))
    try driver.stop()
    try driver.detach()
    XCTAssertTrue(hal.owned.isEmpty)
  }

  func testCancelledPreparationNeverStartsNativeIOOrRequestsAudioPermission() throws {
    let hal = SilentTapHAL()
    let driver = NativeCoreAudioCaptureDriver(
      scope: .system, hal: hal,
      mayContinue: { !hal.trace.contains("io") })
    XCTAssertThrowsError(try driver.start(sample: { _ in }, failure: { _ in }))
    XCTAssertFalse(hal.trace.contains("start"))
    try driver.stop()
    try driver.detach()
    XCTAssertTrue(hal.owned.isEmpty)
  }

  func testFormatChangeNotifiesRecoveryWithoutReadingOrRequestingScreenPermission() async throws {
    let hal = SilentTapHAL()
    let driver = NativeCoreAudioCaptureDriver(scope: .system, hal: hal)
    let failed = expectation(description: "format change")
    try driver.start(
      sample: { _ in XCTFail("mock HAL supplies no actual PCM") },
      failure: { error in
        XCTAssertTrue(String(describing: error).contains("format changed"))
        failed.fulfill()
      })
    hal.changeFormat()
    await fulfillment(of: [failed], timeout: 2)
    try driver.stop()
    try driver.detach()
    XCTAssertTrue(hal.owned.isEmpty)
  }
}
