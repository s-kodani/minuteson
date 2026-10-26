import CoreAudio
import Foundation
import XCTest

@testable import MONMacOSCapture

final class FamilyCatalog: @unchecked Sendable {
  private let lock = NSLock()
  var root = CaptureApplicationIdentity(
    pid: 123, bundleIdentifier: "com.example.app", launchTimeUs: 100)
  private var rows: [CaptureProcess] = []
  private var allowed = true
  func set(_ rows: [CaptureProcess]) { lock.withLock { self.rows = rows } }
  func denyRoot() { lock.withLock { allowed = false } }
  func list() -> [CaptureProcess] { lock.withLock { rows } }
  func read(_ pid: pid_t) throws -> CaptureProcess {
    try lock.withLock {
      guard let row = rows.first(where: { $0.pid == pid }) else {
        throw ProbeError.capture("exited")
      }
      return row
    }
  }
  func require(_ identity: CaptureApplicationIdentity) throws {
    try lock.withLock {
      guard allowed, identity == root else { throw ProbeError.capture("root changed") }
    }
  }
  func family() -> CaptureProcessFamily {
    CaptureProcessFamily(root: root, list: list, read: read, requireRoot: require)
  }
}

final class CaptureProcessFamilyTests: XCTestCase {
  private func row(_ pid: pid_t, _ parent: pid_t, _ start: Int64) -> CaptureProcess {
    CaptureProcess(pid: pid, parentPID: parent, launchTimeUs: start)
  }
  func testDescendantsReparentingExitAndPIDReuseDoNotAdoptUnrelatedProcesses() throws {
    let catalog = FamilyCatalog()
    catalog.set([row(789, 456, 102), row(999, 1, 103), row(456, 123, 101), row(123, 1, 100)])
    let family = catalog.family()
    XCTAssertEqual(try family.snapshot(), [123: 100, 456: 101, 789: 102])
    catalog.set([row(123, 1, 100), row(456, 1, 101), row(789, 456, 102), row(998, 456, 104)])
    XCTAssertEqual(try family.snapshot(), [123: 100, 456: 101, 789: 102, 998: 104])
    catalog.set([row(123, 1, 100), row(456, 1, 200), row(789, 1, 102)])
    XCTAssertEqual(try family.snapshot(), [123: 100, 789: 102])
    catalog.set([row(123, 1, 100)])
    XCTAssertEqual(try family.snapshot(), [123: 100])
  }
  func testRootIdentityAndAmbiguousOrRacingCatalogFailClosed() throws {
    let catalog = FamilyCatalog()
    let family = catalog.family()
    for rows in [
      [row(123, 1, 101)], [row(123, 1, 100), row(123, 1, 100)],
      [row(123, 1, 100), row(-1, 123, 101)],
    ] {
      catalog.set(rows)
      XCTAssertThrowsError(try family.snapshot())
    }
    catalog.set([row(123, 1, 100), row(456, 123, 101)])
    let members = try family.snapshot()
    catalog.set([row(123, 1, 100), row(456, 123, 102)])
    XCTAssertThrowsError(try family.verify(members))
    catalog.denyRoot()
    XCTAssertThrowsError(try family.snapshot())
  }
  func testChildOnlyAudioScopeNeverBecomesGlobal() throws {
    let catalog = FamilyCatalog()
    let scope = CoreAudioTapScope.application(catalog.root)
    let description = try scope.description(
      objects: [(456, 22), (999, 23)], ownPID: 888, family: [123: 100, 456: 101])
    XCTAssertEqual(description.processes, [22])
    XCTAssertFalse(description.isExclusive)
    XCTAssertThrowsError(
      try scope.description(objects: [(999, 23)], ownPID: 888, family: [123: 100]))
    XCTAssertThrowsError(
      try scope.description(objects: [(456, 22)], ownPID: 888, family: [456: 101]))
  }
}
