import Darwin
import Foundation
import XCTest

@testable import MONMacOSCapture

final class CaptureTestLeaseTests: XCTestCase {
  private let owner = "735027d4-a03e-43d3-830a-21e767dfaf88"

  private func fixture(_ test: (URL, URL, Int32) throws -> Void) throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(
      at: root, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
    defer { try? FileManager.default.removeItem(at: root) }
    let output = root.appendingPathComponent("audio-\(owner)")
    try FileManager.default.createDirectory(
      at: output, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
    let path = root.appendingPathComponent("lease-\(owner)")
    let descriptor = open(path.path, O_RDWR | O_CREAT | O_EXCL | O_CLOEXEC, 0o600)
    XCTAssertGreaterThanOrEqual(descriptor, 3)
    defer { close(descriptor) }
    XCTAssertEqual(flock(descriptor, LOCK_EX | LOCK_NB), 0)
    try test(output, path, descriptor)
  }

  func testValidationPreservesTheHeldDescriptionAndOnlySetsCloseOnExec() throws {
    try fixture { output, path, descriptor in
      try CaptureTestLease.requireInherited(descriptor: descriptor, output: output, owner: owner)
      XCTAssertNotEqual(fcntl(descriptor, F_GETFD) & FD_CLOEXEC, 0)
      let check = open(path.path, O_RDONLY | O_CLOEXEC)
      XCTAssertGreaterThanOrEqual(check, 3)
      defer { close(check) }
      XCTAssertEqual(flock(check, LOCK_EX | LOCK_NB), -1)
      XCTAssertEqual(errno, EWOULDBLOCK)
      // Rechecking never explicitly unlocks the inherited description.
      try CaptureTestLease.requireInherited(descriptor: descriptor, output: output, owner: owner)
      XCTAssertEqual(flock(check, LOCK_EX | LOCK_NB), -1)
    }
  }

  func testWrongTargetReplacementLinkAndUnheldLeaseCannotAuthorizeCapture() throws {
    for alteration in ["owner", "output", "replacement", "link", "hardlink", "unheld"] {
      try fixture { output, path, descriptor in
        var requestedOutput = output
        var requestedOwner = owner
        switch alteration {
        case "owner": requestedOwner = UUID().uuidString.lowercased()
        case "output": requestedOutput = output.deletingLastPathComponent()
        case "replacement", "link":
          let moved = path.appendingPathExtension("original")
          try FileManager.default.moveItem(at: path, to: moved)
          if alteration == "link" {
            try FileManager.default.createSymbolicLink(at: path, withDestinationURL: moved)
          } else {
            let replacement = open(path.path, O_RDWR | O_CREAT | O_EXCL | O_CLOEXEC, 0o600)
            XCTAssertGreaterThanOrEqual(replacement, 3)
            close(replacement)
          }
        case "hardlink":
          try FileManager.default.linkItem(at: path, to: path.appendingPathExtension("duplicate"))
        case "unheld": XCTAssertEqual(flock(descriptor, LOCK_UN), 0)
        default: XCTFail("unknown fixture")
        }
        XCTAssertThrowsError(
          try CaptureTestLease.requireInherited(
            descriptor: descriptor, output: requestedOutput, owner: requestedOwner))
        XCTAssertGreaterThanOrEqual(
          fcntl(descriptor, F_GETFD), 0, "failure must not close caller FD")
      }
    }
  }

  func testInvalidKindsPermissionsContentsAndLinkedOutputAreRejected() throws {
    for alteration in ["mode", "contents", "output-mode", "root-mode", "output-link", "directory"] {
      try fixture { output, path, descriptor in
        var supplied = descriptor
        switch alteration {
        case "mode": XCTAssertEqual(fchmod(descriptor, 0o644), 0)
        case "contents": XCTAssertEqual(ftruncate(descriptor, 1), 0)
        case "output-mode": XCTAssertEqual(chmod(output.path, 0o755), 0)
        case "root-mode": XCTAssertEqual(chmod(output.deletingLastPathComponent().path, 0o755), 0)
        case "output-link":
          let moved = output.appendingPathExtension("original")
          try FileManager.default.moveItem(at: output, to: moved)
          try FileManager.default.createSymbolicLink(at: output, withDestinationURL: moved)
        case "directory": supplied = open(output.path, O_RDONLY | O_DIRECTORY | O_CLOEXEC)
        default: XCTFail("unknown fixture")
        }
        defer { if supplied != descriptor { close(supplied) } }
        XCTAssertThrowsError(
          try CaptureTestLease.requireInherited(descriptor: supplied, output: output, owner: owner))
      }
    }
    XCTAssertThrowsError(
      try CaptureTestLease.requireInherited(
        descriptor: -1, output: URL(fileURLWithPath: "/tmp"), owner: owner))
  }

  func testLeaseArgumentRequiresOneCanonicalDescriptorAndFiveSecondControls() throws {
    let base = [
      "--output", "/tmp/test", "--stop-on-file", "--parent-pid", "1234",
      "--microphone-uid", "USB input", "--five-second-test", owner,
    ]
    XCTAssertEqual(
      try Options.parse(base + ["--capture-test-lease-fd", "17"]).captureTestLeaseFD, 17)
    XCTAssertNil(try Options.parse(base).captureTestLeaseFD)
    for arguments in [
      ["--capture-test-lease-fd"], ["--capture-test-lease-fd", "2"],
      ["--capture-test-lease-fd", "-1"], ["--capture-test-lease-fd", "+3"],
      ["--capture-test-lease-fd", "03"], ["--capture-test-lease-fd", "2147483648"],
      ["--capture-test-lease-fd", "3", "--capture-test-lease-fd", "4"],
    ] { XCTAssertThrowsError(try Options.parse(base + arguments)) }
    XCTAssertThrowsError(
      try Options.parse(["--output", "/tmp/test", "--capture-test-lease-fd", "3"]))
  }

  func testInheritedLeaseFixtureWithoutCapture() throws {
    let environment = ProcessInfo.processInfo.environment
    guard let root = environment["MON_CAPTURE_TEST_JOURNAL_FIXTURE"] else {
      throw XCTSkip("Rust-to-Swift inherited lease fixture only; no microphone or native app")
    }
    let descriptor = try XCTUnwrap(
      environment["MON_CAPTURE_TEST_JOURNAL_LEASE_FD"].flatMap(Int32.init))
    let output = URL(fileURLWithPath: try XCTUnwrap(environment["MON_CAPTURE_TEST_JOURNAL_OUTPUT"]))
    let owner = try XCTUnwrap(environment["MON_CAPTURE_TEST_JOURNAL_OWNER"])
    try CaptureTestLease.requireInherited(descriptor: descriptor, output: output, owner: owner)
    let fixture = URL(fileURLWithPath: root)
    try Data().write(to: fixture.appendingPathComponent("helper-ready"))
    let deadline = ContinuousClock.now.advanced(by: .seconds(20))
    while !FileManager.default.fileExists(
      atPath: fixture.appendingPathComponent("helper-stop").path)
    {
      guard ContinuousClock.now < deadline else {
        throw ProbeError.capture("fixture stop timed out")
      }
      Thread.sleep(forTimeInterval: 0.02)
    }
    // Production keeps the inherited lease through process exit. This fixture
    // does the same; the Rust collector cannot proceed just on a ready marker.
  }
}
