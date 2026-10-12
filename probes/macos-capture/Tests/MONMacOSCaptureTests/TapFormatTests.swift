import AVFoundation
import CoreAudio
import XCTest

@testable import MONMacOSCapture

final class TapFormatTests: XCTestCase {
  func testOwnedFormatRetainsEveryFieldForInterleavedAndPlanarPCM() throws {
    for interleaved in [false, true] {
      for channels: UInt32 in [1, 2] {
        let original = try XCTUnwrap(
          AVAudioFormat(
            commonFormat: .pcmFormatFloat32, sampleRate: 48_000,
            channels: channels, interleaved: interleaved))
        for _ in 0..<100 {
          let owned = try CoreAudioTapSample.format(original.streamDescription.pointee)
          XCTAssertEqual(owned.commonFormat, .pcmFormatFloat32)
          XCTAssertEqual(owned.isInterleaved, original.isInterleaved)
          XCTAssertTrue(
            NativeCoreAudioCaptureDriver.sameFormat(
              owned.streamDescription.pointee, original.streamDescription.pointee))
        }
      }
    }
  }

  func testInvalidFormatsFailWithActualMetadataWithoutClaimingPermissionDenied() throws {
    let original = try XCTUnwrap(
      AVAudioFormat(
        commonFormat: .pcmFormatFloat32, sampleRate: 48_000, channels: 2, interleaved: true))
    let changes: [(inout AudioStreamBasicDescription) -> Void] = [
      { $0.mSampleRate = .nan }, { $0.mSampleRate = 0 },
      { $0.mChannelsPerFrame = 0 }, { $0.mChannelsPerFrame = 33 },
      { $0.mFormatID = kAudioFormatMPEG4AAC }, { $0.mBitsPerChannel = 16 },
      { $0.mFormatFlags &= ~kAudioFormatFlagIsFloat },
      { $0.mFormatFlags |= kAudioFormatFlagIsBigEndian },
      { $0.mBytesPerFrame = 4 }, { $0.mBytesPerPacket = 0 },
      { $0.mFramesPerPacket = 2 },
    ]
    for change in changes {
      var invalid = original.streamDescription.pointee
      change(&invalid)
      XCTAssertThrowsError(try CoreAudioTapSample.format(invalid)) { error in
        let text = String(describing: error)
        XCTAssertTrue(text.contains("権限拒否とは断定できません"))
        XCTAssertTrue(text.contains("rate="))
        XCTAssertTrue(text.contains("flags="))
        XCTAssertTrue(text.contains("packetFrames="))
      }
    }
  }
}
