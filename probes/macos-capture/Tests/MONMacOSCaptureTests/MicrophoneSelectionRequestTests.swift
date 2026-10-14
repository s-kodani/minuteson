import Foundation
import XCTest

@testable import MONMacOSCapture

final class MicrophoneSelectionRequestTests: XCTestCase {
  private let owner = "735027d4-a03e-43d3-830a-21e767dfaf88"

  func testOwnedMonotonicRequestsAndNoUIDInAutomaticMode() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let reader = try MicrophoneSelectionReader(output: directory, owner: owner)
    let path = directory.appendingPathComponent("microphone-selection.request")
    XCTAssertNil(try reader.next())
    func write(_ revision: Int, _ mode: String, _ uid: String?) throws {
      let data = try JSONSerialization.data(withJSONObject: [
        "schemaVersion": 1, "ownerId": owner, "revision": revision,
        "mode": mode, "uid": uid as Any? ?? NSNull(),
      ])
      try data.write(to: path, options: .atomic)
    }
    try write(1, "fixed", "usb-123")
    let fixed = try XCTUnwrap(reader.next())
    XCTAssertEqual(fixed.uid, "usb-123")
    XCTAssertEqual(reader.revision, 0)
    try reader.acknowledge(fixed)
    XCTAssertNil(try reader.next())
    try write(2, "automatic", nil)
    let automatic = try XCTUnwrap(reader.next())
    XCTAssertEqual(automatic.mode, "automatic")
    XCTAssertNil(automatic.uid)
    try reader.acknowledge(automatic)
    XCTAssertEqual(reader.revision, 2)
    try write(1, "fixed", "usb-123")
    XCTAssertNil(try reader.next())
    try write(3, "fixed", "bad\nuid")
    XCTAssertThrowsError(try reader.next())
    XCTAssertEqual(reader.revision, 2)
  }

  func testSymlinkCannotBecomeAMicrophoneSelectionRequest() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let target = directory.appendingPathComponent("target")
    try Data("{}".utf8).write(to: target)
    try FileManager.default.createSymbolicLink(
      at: directory.appendingPathComponent("microphone-selection.request"),
      withDestinationURL: target)
    let reader = try MicrophoneSelectionReader(output: directory, owner: owner)
    XCTAssertThrowsError(try reader.next())
  }
}
