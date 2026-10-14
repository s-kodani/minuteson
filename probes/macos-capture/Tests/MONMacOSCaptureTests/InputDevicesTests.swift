import CoreAudio
import Foundation
import XCTest

@testable import MONMacOSCapture

final class InputDevicesTests: XCTestCase {
  private let devices = [
    CaptureInputDevice(uid: "built-in", name: "同じ名前", isDefault: true, objectID: 1),
    CaptureInputDevice(uid: "usb-123", name: "同じ名前", isDefault: false, objectID: 2),
  ]

  func testExplicitStableIdentityDoesNotUseNameOrDefault() throws {
    XCTAssertEqual(try InputDevices.choose("usb-123", from: devices).objectID, 2)
    XCTAssertEqual(try InputDevices.choose(nil, from: devices).uid, "built-in")
    XCTAssertThrowsError(try InputDevices.choose("missing", from: devices))
    XCTAssertThrowsError(try InputDevices.choose("同じ名前", from: devices))
    XCTAssertThrowsError(try InputDevices.choose("usb-123", from: [devices[0]]))
    XCTAssertEqual(try InputDevices.choose("usb-123", from: devices.reversed()).objectID, 2)
  }

  func testMissingOrAmbiguousIdentityNeverFallsBackToAnotherDevice() {
    XCTAssertThrowsError(try InputDevices.choose(nil, from: []))
    XCTAssertThrowsError(try InputDevices.choose(nil, from: [devices[1]]))
    XCTAssertThrowsError(try InputDevices.choose("usb-123", from: [devices[1], devices[1]]))
    XCTAssertThrowsError(try InputDevices.choose(nil, from: [devices[0], devices[0]]))
  }

  func testUIDBoundsRejectEmptyControlAndOversizedIdentifiers() {
    for uid in [
      "", " \t", "a\nb", "a\0b", String(repeating: "x", count: 1025),
      String(repeating: "日", count: 342),
    ] {
      XCTAssertFalse(InputDevices.validUID(uid))
      XCTAssertThrowsError(try InputDevices.choose(uid, from: devices))
    }
    XCTAssertTrue(InputDevices.validUID(String(repeating: "x", count: 1024)))
    XCTAssertTrue(InputDevices.validUID("USB input/名前 with spaces"))
    XCTAssertTrue(InputDevices.validUID("a\u{200D}b"))
    XCTAssertFalse(InputDevices.validUID("a\u{0085}b"))
  }

  func testMetadataReportHidesTransientObjectIDs() throws {
    let report = CaptureInputDevices(microphonePermission: "notDetermined", devices: devices)
    let json =
      try JSONSerialization.jsonObject(with: JSONEncoder().encode(report)) as! [String: Any]
    XCTAssertEqual(json["schemaVersion"] as? Int, 1)
    XCTAssertEqual(json["microphonePermission"] as? String, "notDetermined")
    let rows = json["devices"] as! [[String: Any]]
    XCTAssertEqual(rows[1]["uid"] as? String, "usb-123")
    XCTAssertNil(rows[1]["objectID"])
  }

  func testAutomaticChoiceOnlyUsesOneKnownProcessInput() throws {
    let direct = try InputDevices.automaticChoice(from: devices, application: nil)
    XCTAssertEqual(direct.device.uid, "built-in")
    XCTAssertEqual(direct.source, .systemDefault(reason: "mon_direct_recording"))
    let app = try InputDevices.automaticChoice(from: devices, processInputObjects: [2])
    XCTAssertEqual(app.device.uid, "usb-123")
    XCTAssertEqual(app.source, .application)

    let inconclusive: [[AudioObjectID]?] = [nil, [], [2, 1], [2, 2], [99]]
    for objects in inconclusive {
      let fallback = try InputDevices.automaticChoice(
        from: devices, processInputObjects: objects)
      XCTAssertEqual(fallback.device.uid, "built-in")
      XCTAssertNotEqual(fallback.source, .application)
    }
    XCTAssertEqual(
      try InputDevices.automaticChoice(from: devices, processInputObjects: nil).source,
      .systemDefault(reason: "application_input_unavailable"))
    XCTAssertThrowsError(
      try InputDevices.automaticChoice(from: [devices[1]], processInputObjects: nil))
  }

  func testProcessLookupRejectsInvalidPIDWithoutOpeningAudio() {
    XCTAssertThrowsError(try InputDevices.activeInputObjects(for: 0))
    XCTAssertThrowsError(try InputDevices.activeInputObjects(for: -1))
  }

  func testProcessLookupOfThisProcessIsReadOnlyAndBounded() throws {
    let objects = try InputDevices.activeInputObjects(for: getpid())
    XCTAssertLessThanOrEqual(objects?.count ?? 0, 128)
  }
}
