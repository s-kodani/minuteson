import AVFoundation
import XCTest

@testable import MONMacOSCapture

final class CaptureFailureGuideTests: XCTestCase {
  func testPermissionStatesAreDistinctWithoutRequestingAccess() {
    let pending = CaptureFailureGuide.microphonePermission(.notDetermined)
    XCTAssertTrue(pending.contains("未決定"))
    XCTAssertTrue(pending.contains("拒否されたとは限りません"))
    let denied = CaptureFailureGuide.microphonePermission(.denied)
    XCTAssertTrue(denied.contains("許可されていません"))
    XCTAssertTrue(denied.contains("プライバシーとセキュリティ → マイク"))
    XCTAssertTrue(denied.contains("MON Capture Probe"))
    XCTAssertTrue(CaptureFailureGuide.microphonePermission(.restricted).contains("管理者"))
    XCTAssertEqual(CaptureFailureGuide.microphonePermission(.authorized), "マイク権限は許可済みです。")
  }

  func testInitializationGuidePreservesEngineFailureDetail() {
    let result = CaptureFailureGuide.microphoneInitialization(ProbeError.capture("engine failed -123"))
    XCTAssertTrue(result.contains("音声エンジン"))
    XCTAssertTrue(result.contains("engine failed -123"))
    XCTAssertFalse(result.contains("許可されていません"))
  }
}
