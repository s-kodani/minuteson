import Foundation
import XCTest

@testable import MONMacOSCapture

final class CaptureApplicationsTests: XCTestCase {
  private let identity = CaptureApplicationIdentity(
    pid: 123, bundleIdentifier: "com.example.browser", launchTimeUs: 1_791_100_000_000_001)
  func testProcessStartIsExactAndBoundedWithoutLiveEnumeration() throws {
    XCTAssertEqual(
      try CaptureApplications.startTime(seconds: 1_791_100_000, microseconds: 1),
      identity.launchTimeUs)
    for (seconds, micros) in [
      (UInt64(0), UInt64(0)), (1, 1_000_000), (UInt64.max, 1), (9_007_199_255, 0),
    ] {
      XCTAssertThrowsError(
        try CaptureApplications.startTime(seconds: seconds, microseconds: micros))
    }
  }
  func testReusedPIDAndSameNamedAppsNeverSatisfyAnOldIdentity() throws {
    let app = CaptureApplication(
      pid: identity.pid, bundleIdentifier: identity.bundleIdentifier,
      launchTimeUs: identity.launchTimeUs, name: "同じ名前")
    XCTAssertNoThrow(try CaptureApplications.require(identity, from: [app]))
    for alternative in [
      CaptureApplication(
        pid: 123, bundleIdentifier: identity.bundleIdentifier,
        launchTimeUs: identity.launchTimeUs + 1, name: app.name),
      CaptureApplication(
        pid: 123, bundleIdentifier: "com.example.other", launchTimeUs: identity.launchTimeUs,
        name: app.name),
      CaptureApplication(
        pid: 456, bundleIdentifier: identity.bundleIdentifier, launchTimeUs: identity.launchTimeUs,
        name: app.name),
    ] { XCTAssertThrowsError(try CaptureApplications.require(identity, from: [alternative])) }
    XCTAssertThrowsError(try CaptureApplications.require(identity, from: []))
    XCTAssertThrowsError(try CaptureApplications.require(identity, from: [app, app]))
  }
  func testInvalidIdentitiesAreRejectedAndReportHasOnlyBoundedPublicMetadata() throws {
    for i in [
      CaptureApplicationIdentity(pid: 0, bundleIdentifier: "a", launchTimeUs: 1),
      CaptureApplicationIdentity(pid: 1, bundleIdentifier: "a\nb", launchTimeUs: 1),
      CaptureApplicationIdentity(
        pid: 1, bundleIdentifier: "a", launchTimeUs: 9_007_199_254_740_992),
    ] { XCTAssertFalse(i.valid) }
    let row = CaptureApplication(
      pid: identity.pid, bundleIdentifier: identity.bundleIdentifier,
      launchTimeUs: identity.launchTimeUs, name: "ブラウザ")
    let v =
      try JSONSerialization.jsonObject(
        with: JSONEncoder().encode(
          CaptureApplicationsReport(systemPermission: "permissionRequired", applications: [row])))
      as! [String: Any]
    XCTAssertEqual(v["schemaVersion"] as? Int, 1)
    let rows = v["applications"] as! [[String: Any]]
    XCTAssertEqual(rows[0]["launchTimeUs"] as? Int64, identity.launchTimeUs)
    XCTAssertNil(rows[0]["identity"])
  }
}
