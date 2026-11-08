import AVFoundation
import CryptoKit
import Foundation
import XCTest

@testable import MONMacOSCapture

final class AudioImportTests: XCTestCase {
  private func directory() throws -> URL {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false)
    return root
  }

  private func wav(_ root: URL, nonFinite: Bool = false) throws -> URL {
    let url = root.appendingPathComponent("source.wav")
    let format = try XCTUnwrap(
      AVAudioFormat(
        commonFormat: .pcmFormatFloat32, sampleRate: 16000, channels: 2, interleaved: true))
    let buffer = try XCTUnwrap(AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 5000))
    buffer.frameLength = 5000
    let samples = try XCTUnwrap(buffer.floatChannelData)
    for frame in 0..<5000 {
      samples[0][frame * 2] = 0.25
      samples[0][frame * 2 + 1] = -0.5
    }
    if nonFinite { samples[0][4097 * 2] = .nan }
    let file = try AVAudioFile(
      forWriting: url, settings: format.settings, commonFormat: .pcmFormatFloat32, interleaved: true
    )
    try file.write(from: buffer)
    return url
  }

  func testWAVContentDetectionAndCompactMonoCAFPreservesDuration() async throws {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    let source = try wav(root)
    let moved = root.appendingPathComponent("misleading.mp3")
    try FileManager.default.moveItem(at: source, to: moved)
    let original = try Data(contentsOf: moved)
    let inspected = try await AudioImporter.decode(input: moved, output: nil)
    XCTAssertEqual(inspected.detectedFormat, "wav")
    XCTAssertEqual(inspected.frames, 5000)
    XCTAssertEqual(inspected.durationUs, 312500)
    let output = root.appendingPathComponent("decoded.caf")
    let converted = try await AudioImporter.decode(input: moved, output: output)
    XCTAssertEqual(converted, inspected)
    XCTAssertEqual(try Data(contentsOf: moved), original)
    let file = try AVAudioFile(
      forReading: output, commonFormat: .pcmFormatFloat32, interleaved: true)
    let buffer = try XCTUnwrap(
      AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: 5000))
    try file.read(into: buffer)
    XCTAssertEqual(buffer.frameLength, 5000)
    XCTAssertEqual(file.fileFormat.sampleRate, 16000)
    XCTAssertEqual(file.fileFormat.channelCount, 1)
    XCTAssertEqual(file.fileFormat.commonFormat, .pcmFormatInt16)
    XCTAssertEqual(try Data(contentsOf: output).count, 68 + 5000 * 2)
    let samples = try XCTUnwrap(buffer.floatChannelData)
    for frame in 0..<5000 {
      XCTAssertEqual(samples[0][frame], -0.125, accuracy: 1.0 / 32768)
    }
  }

  func testCancellationAndDestinationOverwriteLeaveOriginalFilesUntouched() async throws {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    let source = try wav(root)
    let original = try Data(contentsOf: source)
    let output = root.appendingPathComponent("existing.caf")
    let existing = Data("retain existing destination".utf8)
    try existing.write(to: output)
    do {
      _ = try await AudioImporter.decode(input: source, output: output)
      XCTFail("must refuse overwrite")
    } catch {}
    XCTAssertEqual(try Data(contentsOf: output), existing)
    do {
      _ = try await AudioImporter.decode(input: source, output: nil, cancelled: { true })
      XCTFail("must honor cancellation")
    } catch {}
    XCTAssertEqual(try Data(contentsOf: source), original)
  }

  func testBrokenAndNonFiniteAudioAreRejectedWithoutChangingSource() async throws {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    let nonFinite = try wav(root, nonFinite: true)
    let original = try Data(contentsOf: nonFinite)
    do {
      _ = try await AudioImporter.decode(input: nonFinite, output: nil)
      XCTFail("must reject non-finite PCM")
    } catch {}
    XCTAssertEqual(try Data(contentsOf: nonFinite), original)
    let broken = root.appendingPathComponent("broken.m4a")
    try Data("not audio".utf8).write(to: broken)
    do {
      _ = try await AudioImporter.decode(input: broken, output: nil)
      XCTFail("must reject invalid input")
    } catch {}
  }

  private struct Fixture: Decodable {
    let file: String
    let accepted: Bool
    let decode: Bool
    let codec: String?
    let bytes: Int
    let sha256: String
  }
  private func fixtures() throws -> (URL, [Fixture]) {
    var root = URL(fileURLWithPath: #filePath)
    for _ in 0..<5 { root.deleteLastPathComponent() }
    root.appendPathComponent("tests/fixtures/audio-import")
    return (
      root,
      try JSONDecoder().decode(
        [Fixture].self, from: Data(contentsOf: root.appendingPathComponent("manifest.json")))
    )
  }
  func testSharedContainerCorpusAndProvenance() throws {
    let (root, fixtures) = try fixtures()
    for fixture in fixtures {
      let url = root.appendingPathComponent(fixture.file)
      let before = try Data(contentsOf: url)
      XCTAssertEqual(before.count, fixture.bytes)
      XCTAssertEqual(
        SHA256.hash(data: before).map { String(format: "%02x", $0) }.joined(), fixture.sha256)
      let file = try FileHandle(forReadingFrom: url)
      defer { try? file.close() }
      if fixture.accepted {
        XCTAssertNoThrow(try AudioImporter.checkM4ATracks(file), fixture.file)
      } else {
        XCTAssertThrowsError(try AudioImporter.checkM4ATracks(file), fixture.file)
      }
      XCTAssertEqual(try Data(contentsOf: url), before)
    }
  }
  func testChapterAACAndALACDecodeOnlyAudioAndPreserveSource() async throws {
    let (fixturesRoot, fixtures) = try fixtures()
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    for fixture in fixtures where fixture.decode {
      let input = fixturesRoot.appendingPathComponent(fixture.file)
      let before = try Data(contentsOf: input)
      let output = root.appendingPathComponent(fixture.file + ".caf")
      let info = try await AudioImporter.decode(input: input, output: output)
      XCTAssertEqual(info.codec, fixture.codec)
      XCTAssertEqual(info.detectedFormat, "m4a")
      XCTAssertEqual(info.sampleRate, 48000)
      XCTAssertEqual(info.channels, 1)
      XCTAssertTrue((48000...49152).contains(info.frames))
      let pcm = try AVAudioFile(forReading: output)
      XCTAssertEqual(
        UInt64(pcm.length),
        (info.frames * 16000 + UInt64(info.sampleRate) - 1) / UInt64(info.sampleRate))
      XCTAssertEqual(pcm.fileFormat.sampleRate, 16000)
      XCTAssertEqual(pcm.fileFormat.channelCount, 1)
      XCTAssertEqual(pcm.fileFormat.commonFormat, .pcmFormatInt16)
      XCTAssertEqual(try Data(contentsOf: input), before)
    }
  }

  func testM4ATrackInspectionRejectsVideoMultipleAudioAndInvalidBoxSizes() throws {
    let root = try directory()
    defer { try? FileManager.default.removeItem(at: root) }
    func box(_ type: String, _ content: Data) -> Data {
      var size = UInt32(content.count + 8).bigEndian
      return withUnsafeBytes(of: &size) { Data($0) } + Data(type.utf8) + content
    }
    func track(_ type: String) -> Data {
      var id = UInt32(type == "soun" ? 1 : 2).bigEndian
      let header = Data(repeating: 0, count: 12) + withUnsafeBytes(of: &id) { Data($0) }
      return box(
        "trak",
        box("tkhd", header)
          + box("mdia", box("hdlr", Data(repeating: 0, count: 8) + Data(type.utf8))))
    }
    func inspect(_ data: Data) throws {
      let url = root.appendingPathComponent("boxes")
      try data.write(to: url)
      let file = try FileHandle(forReadingFrom: url)
      defer { try? file.close() }
      try AudioImporter.checkM4ATracks(file)
    }
    XCTAssertNoThrow(try inspect(box("moov", track("soun"))))
    XCTAssertThrowsError(try inspect(box("moov", track("soun") + track("vide"))))
    XCTAssertThrowsError(try inspect(box("moov", track("soun") + track("soun"))))
    XCTAssertThrowsError(try inspect(Data([0, 0, 0, 1]) + Data("moov".utf8)))
    XCTAssertThrowsError(try inspect(box("moov", track("soun")) + Data([0])))
    XCTAssertThrowsError(try inspect(Data(repeating: 0, count: 8 * 4097)))
  }
}
