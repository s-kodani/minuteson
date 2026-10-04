import Foundation
import IOKit.pwr_mgt
import XCTest

@testable import MONMacOSCapture

private final class FakePower: RecordingPowerBackend {
  var creates = 0
  var releases: [IOPMAssertionID] = []
  var failCreate = false
  var failRelease = false
  func acquire() throws -> IOPMAssertionID {
    creates += 1
    if failCreate { throw RecordingPowerError(operation: "acquire", status: kIOReturnError) }
    return 42
  }
  func release(_ id: IOPMAssertionID) throws {
    releases.append(id)
    if failRelease { throw RecordingPowerError(operation: "release", status: kIOReturnError) }
  }
}

final class RecordingPowerTests: XCTestCase {
  func testIdleAcquisitionAndReleaseAreScopedAndIdempotent() throws {
    let backend = FakePower()
    let guardValue = RecordingPowerAssertion(backend: backend)
    XCTAssertNil(guardValue.id)
    XCTAssertEqual(backend.creates, 0)
    try guardValue.release()
    try guardValue.acquire()
    try guardValue.acquire()
    XCTAssertEqual(backend.creates, 1)
    XCTAssertEqual(guardValue.id, 42)
    try guardValue.release()
    try guardValue.release()
    XCTAssertNil(guardValue.id)
    XCTAssertEqual(backend.releases, [42])
  }

  func testAcquisitionFailureNeverClaimsOwnershipAndCanRetry() throws {
    let backend = FakePower()
    let guardValue = RecordingPowerAssertion(backend: backend)
    backend.failCreate = true
    XCTAssertThrowsError(try guardValue.acquire())
    XCTAssertNil(guardValue.id)
    try guardValue.release()
    XCTAssertTrue(backend.releases.isEmpty)
    backend.failCreate = false
    try guardValue.acquire()
    try guardValue.release()
    XCTAssertEqual(backend.creates, 2)
    XCTAssertEqual(backend.releases, [42])
  }

  func testFailedReleasePreservesOwnershipUntilCleanupRetries() throws {
    let backend = FakePower()
    var guardValue: RecordingPowerAssertion? = RecordingPowerAssertion(backend: backend)
    try guardValue!.acquire()
    backend.failRelease = true
    XCTAssertThrowsError(try guardValue!.release())
    XCTAssertEqual(guardValue!.id, 42)
    backend.failRelease = false
    guardValue = nil
    XCTAssertEqual(backend.releases, [42, 42])
  }

  func testUnexpectedScopeExitReleasesOwnedAssertion() throws {
    let backend = FakePower()
    func operation() throws {
      let guardValue = RecordingPowerAssertion(backend: backend)
      try guardValue.acquire()
      defer { try? guardValue.release() }
      throw RecordingPowerError(operation: "fixture recording start", status: kIOReturnError)
    }
    XCTAssertThrowsError(try operation())
    XCTAssertEqual(backend.creates, 1)
    XCTAssertEqual(backend.releases, [42])
  }

  func testNativeAssertionIsSystemIdleOnlyAndDisappearsAfterRelease() throws {
    let guardValue = RecordingPowerAssertion()
    try guardValue.acquire()
    defer { try? guardValue.release() }
    let id = try XCTUnwrap(guardValue.id)
    let copied = try XCTUnwrap(IOPMAssertionCopyProperties(id)?.takeRetainedValue())
    let properties = copied as NSDictionary
    XCTAssertEqual(properties[kIOPMAssertionTypeKey] as? String, "PreventUserIdleSystemSleep")
    XCTAssertEqual(properties[kIOPMAssertionLevelKey] as? UInt32, UInt32(kIOPMAssertionLevelOn))
    XCTAssertEqual(properties[kIOPMAssertionNameKey] as? String, "MON audio recording")
    try guardValue.release()
    XCTAssertNil(IOPMAssertionCopyProperties(id))
  }
}
