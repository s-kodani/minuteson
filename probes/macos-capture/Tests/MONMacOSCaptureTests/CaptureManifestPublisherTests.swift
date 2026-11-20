import Darwin
import Foundation
import XCTest

@testable import MONMacOSCapture

final class CaptureManifestPublisherTests: XCTestCase {
  private func directory() throws -> URL {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(
      at: root, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
    let canonical = try XCTUnwrap(realpath(root.path, nil))
    defer { free(canonical) }
    return URL(fileURLWithPath: String(cString: canonical), isDirectory: true)
  }

  private func manifest() -> Manifest {
    Manifest(
      schemaVersion: 5, scenario: "synthetic-manifest-publication", captureMode: "system",
      backend: "coreaudio", targetPID: nil, targetPIDs: [], startedAtUTC: "synthetic",
      requestedDurationSeconds: nil, osVersion: "synthetic", architecture: "arm64",
      state: "starting", microphone: TrackStats(), system: TrackStats(), errors: [])
  }

  func testProductionPublisherIsPrivateUnderPermissiveUmask() throws {
    if let childRoot = ProcessInfo.processInfo.environment["MON_CAPTURE_MANIFEST_CHILD"] {
      let root = URL(fileURLWithPath: childRoot, isDirectory: true)
      // Confirm the child inherited 022, without changing the test runner's
      // process-wide mask. The historical Foundation writer produces 0644.
      let ordinary = root.appendingPathComponent("ordinary.json")
      try Data("legacy manifest".utf8).write(to: ordinary, options: .atomic)
      var previous = stat()
      XCTAssertEqual(lstat(ordinary.path, &previous), 0)
      XCTAssertEqual(previous.st_mode & 0o777, 0o644)
      try FileManager.default.removeItem(at: ordinary)

      var value = manifest()
      let file = try CaptureSession.createManifest(value, output: root)
      let path = root.appendingPathComponent("manifest.json")
      XCTAssertEqual(lstat(path.path, &previous), 0)
      XCTAssertEqual(previous.st_mode & 0o777, 0o600)
      try file.verifyCurrent(value.encodedData())
      for state in ["recording", "saved"] {
        value.state = state
        value.errors = [String(repeating: "synthetic diagnostic ", count: 4000)]
        XCTAssertGreaterThan(try value.encodedData().count, 65 * 1024)
        try CaptureSession.write(value, using: file)
        try file.verifyCurrent(value.encodedData())
        var current = stat()
        XCTAssertEqual(lstat(path.path, &current), 0)
        XCTAssertEqual(current.st_mode & 0o777, 0o600)
        XCTAssertEqual(current.st_nlink, 1)
        XCTAssertNotEqual(current.st_ino, previous.st_ino)
        previous = current
      }
      XCTAssertEqual(
        try FileManager.default.contentsOfDirectory(atPath: root.path), ["manifest.json"])
      return
    }

    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    let child = Process()
    child.executableURL = URL(fileURLWithPath: "/bin/sh")
    child.arguments = [
      "-c",
      #"umask 022; exec /usr/bin/xcrun xctest -XCTest MONMacOSCaptureTests.CaptureManifestPublisherTests/testProductionPublisherIsPrivateUnderPermissiveUmask "$1""#,
      "capture-manifest-fixture", Bundle(for: Self.self).bundleURL.path,
    ]
    child.environment = ProcessInfo.processInfo.environment.merging([
      "MON_CAPTURE_MANIFEST_CHILD": root.path
    ]) { _, child in child }
    let output = Pipe()
    child.standardOutput = output
    child.standardError = output
    try child.run()
    let deadline = Date().addingTimeInterval(20)
    while child.isRunning, Date() < deadline { Thread.sleep(forTimeInterval: 0.01) }
    if child.isRunning { _ = kill(child.processIdentifier, SIGKILL) }
    child.waitUntilExit()
    let report = String(decoding: output.fileHandleForReading.readDataToEndOfFile(), as: UTF8.self)
    XCTAssertEqual(child.terminationReason, .exit, report)
    XCTAssertEqual(child.terminationStatus, 0, report)
    XCTAssertTrue(
      FileManager.default.fileExists(atPath: root.appendingPathComponent("manifest.json").path))
  }

  func testManifestBudgetDoesNotExpandJournalBudget() throws {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    let largerThanJournal = Data(repeating: 32, count: 65 * 1024 + 1)
    XCTAssertThrowsError(try InputEventJournalFile(output: root, initial: largerThanJournal))
    XCTAssertFalse(
      FileManager.default.fileExists(atPath: root.appendingPathComponent("input-events.json").path))
    XCTAssertThrowsError(
      try InputEventJournalFile(
        output: root, initial: Data(repeating: 32, count: 32 * 1024 * 1024 + 1),
        leaf: .captureManifest))
    XCTAssertFalse(
      FileManager.default.fileExists(atPath: root.appendingPathComponent("manifest.json").path))
    let journal = try InputEventJournalFile(output: root, initial: Data("confirmed".utf8))
    XCTAssertThrowsError(try journal.replace(largerThanJournal))
    try journal.verifyCurrent(Data("confirmed".utf8))
  }

  func testProductionPublisherRejectsAReplacedManifestWithoutOverwritingIt() throws {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    var value = manifest()
    let file = try CaptureSession.createManifest(value, output: root)
    let path = root.appendingPathComponent("manifest.json")
    let original = root.appendingPathComponent("retained-original.json")
    let bytes = try Data(contentsOf: path)
    try FileManager.default.moveItem(at: path, to: original)
    let descriptor = open(path.path, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC, 0o600)
    XCTAssertGreaterThanOrEqual(descriptor, 0)
    guard descriptor >= 0 else { return }
    let replacement = FileHandle(fileDescriptor: descriptor, closeOnDealloc: true)
    try replacement.write(contentsOf: bytes)
    try replacement.close()
    value.state = "recording"
    XCTAssertThrowsError(try CaptureSession.write(value, using: file))
    XCTAssertEqual(try Data(contentsOf: path), bytes)
    XCTAssertEqual(try Data(contentsOf: original), bytes)
  }
}
