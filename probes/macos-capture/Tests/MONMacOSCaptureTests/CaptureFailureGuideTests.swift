import AVFoundation
import XCTest
import ScreenCaptureKit

@testable import MONMacOSCapture

final class CaptureFailureGuideTests: XCTestCase {
  func testSystemPermissionRejectionHasScopedGuidanceAndOriginalDetail() {
    let error = NSError(domain: SCStreamErrorDomain,
      code: SCStreamError.Code.userDeclined.rawValue,
      userInfo: [NSLocalizedDescriptionKey: "capture denied evidence"])
    let result = CaptureFailureGuide.systemCaptureFailure(error)
    guard case .permission(let message) = result else { return XCTFail("expected permission") }
    XCTAssertTrue(message.contains("PC音声の取得がOSで許可されていません"))
    XCTAssertTrue(message.contains("画面収録とシステムオーディオ録音"))
    XCTAssertTrue(message.contains("「システムオーディオ録音のみ」とは別"))
    XCTAssertTrue(message.contains("画面へのアクセスも含む"))
    XCTAssertTrue(message.contains("自動で許可変更・再試行はしません"))
    XCTAssertTrue(message.contains("capture denied evidence"))
  }

  func testSystemPreparationFailureIsNotMisclassifiedAsPermission() {
    for error in [
      NSError(domain: SCStreamErrorDomain, code: SCStreamError.Code.failedToStart.rawValue),
      NSError(domain: "other.capture", code: SCStreamError.Code.userDeclined.rawValue),
    ] {
      guard case .capture(let message) = CaptureFailureGuide.systemCaptureFailure(error)
      else { return XCTFail("not an authorization denial") }
      XCTAssertTrue(message.contains("権限拒否とは断定できません"))
      XCTAssertTrue(message.contains(error.domain))
      XCTAssertFalse(message.contains("ご自身で変更"))
    }
  }

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
