import AVFoundation
import CoreAudio
import ScreenCaptureKit
import XCTest

@testable import MONMacOSCapture

final class CaptureFailureGuideTests: XCTestCase {
  func testAudioOnlyPermissionFailureIsExplicitAndUnknownErrorsRemainUnknown() {
    guard
      case .permission(let message) = CaptureFailureGuide.coreAudioFailure(
        kAudioDevicePermissionsError, operation: "start tap IO")
    else { return XCTFail("permission") }
    XCTAssertTrue(message.contains("システムオーディオ録音のみ"))
    XCTAssertTrue(message.contains("MinutesOn、MON QA"))
    XCTAssertTrue(message.contains("MinutesOn Capture Helper"))
    XCTAssertTrue(message.contains("画面の収録許可は要求しません"))
    XCTAssertTrue(message.contains("start tap IO"))
    for status in [
      kAudioHardwareIllegalOperationError, kAudioHardwareBadDeviceError, OSStatus(-3801),
    ] {
      guard
        case .capture(let message) = CaptureFailureGuide.coreAudioFailure(status, operation: "tap")
      else { return XCTFail("unknown status is not permission denial") }
      XCTAssertTrue(message.contains("権限拒否とは断定できません"))
      XCTAssertTrue(message.contains(String(status)))
    }
  }
  func testSystemPermissionRejectionHasScopedGuidanceAndOriginalDetail() {
    let error = NSError(
      domain: SCStreamErrorDomain,
      code: SCStreamError.Code.userDeclined.rawValue,
      userInfo: [NSLocalizedDescriptionKey: "capture denied evidence"])
    let result = CaptureFailureGuide.systemCaptureFailure(error)
    guard case .permission(let message) = result else { return XCTFail("expected permission") }
    XCTAssertTrue(message.contains("PC音声の取得がOSで許可されていません"))
    XCTAssertTrue(message.contains("MinutesOn、MON QA"))
    XCTAssertTrue(message.contains("MinutesOn Capture Helper"))
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
    XCTAssertTrue(denied.contains("MinutesOn、MON QA"))
    XCTAssertTrue(denied.contains("MinutesOn Capture Helper"))
    XCTAssertTrue(CaptureFailureGuide.microphonePermission(.restricted).contains("管理者"))
    XCTAssertEqual(CaptureFailureGuide.microphonePermission(.authorized), "マイク権限は許可済みです。")
  }

  func testInitializationGuidePreservesEngineFailureDetail() {
    let result = CaptureFailureGuide.microphoneInitialization(
      ProbeError.capture("engine failed -123"))
    XCTAssertTrue(result.contains("音声エンジン"))
    XCTAssertTrue(result.contains("engine failed -123"))
    XCTAssertFalse(result.contains("許可されていません"))
  }
}
