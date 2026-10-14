import Foundation
import XCTest

@testable import MONMacOSCapture

final class MicrophoneSelectionStatusTests: XCTestCase {
  private let owner = "735027d4-a03e-43d3-830a-21e767dfaf88"

  func testFallbackPublicationIsOwnedAndSeparateFromPCMStatus() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let report = MicrophoneSelectionStatus(
      ownerId: owner, revision: 1, requestRevision: 0, mode: "automatic",
      deviceUID: "built-in", source: "systemDefault",
      fallbackReason: "application_input_unavailable")
    try report.publish(to: directory)
    let file = directory.appendingPathComponent("microphone-selection.status.json")
    let value = try XCTUnwrap(
      JSONSerialization.jsonObject(with: Data(contentsOf: file)) as? [String: Any])
    XCTAssertEqual(value["ownerId"] as? String, owner)
    XCTAssertEqual(value["fallbackReason"] as? String, "application_input_unavailable")
    XCTAssertEqual(value["requestRevision"] as? Int, 0)
    XCTAssertNil(value["connected"])
    try MicrophoneSelectionStatus(
      ownerId: owner, revision: 2, requestRevision: 1, mode: "fixed",
      deviceUID: "built-in", source: "pending", fallbackReason: nil
    ).publish(to: directory)
    if let export = ProcessInfo.processInfo.environment["MON_MIC_SELECTION_FIXTURE_DIR"] {
      let destination = URL(fileURLWithPath: export, isDirectory: true)
      XCTAssertTrue(try FileManager.default.contentsOfDirectory(atPath: destination.path).isEmpty)
      try report.publish(to: destination)
    }
    XCTAssertThrowsError(
      try MicrophoneSelectionStatus(
        ownerId: owner, revision: 2, requestRevision: 1, mode: "fixed",
        deviceUID: "built-in", source: "systemDefault", fallbackReason: "mon_direct_recording"
      ).publish(to: directory))
  }
}
