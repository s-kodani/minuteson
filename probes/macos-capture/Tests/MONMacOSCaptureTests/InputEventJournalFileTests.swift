import Darwin
import Foundation
import XCTest

@testable import MONMacOSCapture

final class TipStorageFault: @unchecked Sendable {
  enum Point: CaseIterable, Sendable { case create, write, fileSync, replace, directorySync }
  private let lock = NSLock()
  private var armed: (Point, POSIXErrorCode, Int)?
  private var directoryDescriptor: Int32?
  private var fileDescriptor: Int32?
  var descriptors: (Int32?, Int32?) { lock.withLock { (directoryDescriptor, fileDescriptor) } }
  func arm(_ point: Point, code: POSIXErrorCode, skipping: Int = 0) {
    lock.withLock { armed = (point, code, skipping) }
  }
  func disarm() { lock.withLock { armed = nil } }
  private func failure(_ point: Point) -> POSIXErrorCode? {
    lock.withLock {
      guard let (target, code, skip) = armed, target == point else { return nil }
      if skip > 0 {
        armed = (target, code, skip - 1)
        return nil
      }
      return code
    }
  }
  func io() -> InputEventJournalFile.StorageIO {
    let native = InputEventJournalFile.StorageIO()
    return InputEventJournalFile.StorageIO(
      create: { [self] directory, name in
        lock.withLock { directoryDescriptor = directory }
        if let code = failure(.create) { throw POSIXError(code) }
        let fd = try native.create(directory, name)
        lock.withLock { fileDescriptor = fd }
        return fd
      },
      write: { [self] data, fd in
        if let code = failure(.write) {
          try native.write(Data(data.prefix(8)), fd)
          throw POSIXError(code)
        }
        try native.write(data, fd)
      },
      synchronize: { [self] fd in
        var info = stat()
        XCTAssertEqual(fstat(fd, &info), 0)
        let point: Point = info.st_mode & S_IFMT == S_IFDIR ? .directorySync : .fileSync
        if let code = failure(point) { throw POSIXError(code) }
        try native.synchronize(fd)
      },
      replace: { [self] directory, source, target in
        if let code = failure(.replace) { throw POSIXError(code) }
        try native.replace(directory, source, target)
      })
  }
}

final class InputEventJournalFileTests: XCTestCase {
  private func directory() throws -> URL {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false)
    let canonical = try XCTUnwrap(realpath(root.path, nil))
    defer { free(canonical) }
    return URL(fileURLWithPath: String(cString: canonical), isDirectory: true)
  }
  func testInitialNativeLeafPreservesPOSIXCauseAndNeverOverwritesExistingFile() throws {
    for code in [POSIXErrorCode.ENOSPC, .EDQUOT, .EIO] {
      for point in [TipStorageFault.Point.create, .write, .fileSync, .directorySync] {
        let root = try directory()
        defer { try? FileManager.default.removeItem(at: root) }
        let fault = TipStorageFault()
        fault.arm(point, code: code)
        XCTAssertThrowsError(
          try InputEventJournalFile(
            output: root, initial: Data("initial bytes".utf8), storageIO: fault.io())
        ) { XCTAssertEqual(($0 as? POSIXError)?.code, code) }
        let (directoryFD, fileFD) = fault.descriptors
        // A throwing initializer releases its pinned directory and any created
        // leaf descriptor; the retained bytes are not deleted to fix a leak.
        XCTAssertEqual(fcntl(try XCTUnwrap(directoryFD), F_GETFD), -1)
        XCTAssertEqual(errno, EBADF)
        if let fileFD {
          XCTAssertEqual(fcntl(fileFD, F_GETFD), -1)
          XCTAssertEqual(errno, EBADF)
        }
        let leaf = root.appendingPathComponent(InputEventJournalFile.name)
        if point == .create {
          XCTAssertFalse(FileManager.default.fileExists(atPath: leaf.path))
        } else {
          let retained = try Data(contentsOf: leaf)
          fault.disarm()
          XCTAssertThrowsError(
            try InputEventJournalFile(output: root, initial: Data("overwrite".utf8))
          ) {
            XCTAssertEqual(($0 as? POSIXError)?.code, .EEXIST)
          }
          XCTAssertEqual(try Data(contentsOf: leaf), retained)
        }
      }
    }
  }
  func testAtomicReplacementFaultsDistinguishVisibleRenameFromConfirmedSyncAndKeepOwnedRetry()
    throws
  {
    let old = Data("old confirmed document".utf8)
    let next = Data("new complete document".utf8)
    for code in [POSIXErrorCode.ENOSPC, .EDQUOT, .EIO] {
      for point in TipStorageFault.Point.allCases {
        let root = try directory()
        defer { try? FileManager.default.removeItem(at: root) }
        let fault = TipStorageFault()
        let file = try InputEventJournalFile(output: root, initial: old, storageIO: fault.io())
        fault.arm(point, code: code)
        XCTAssertThrowsError(try file.replace(next)) {
          XCTAssertEqual(($0 as? POSIXError)?.code, code)
        }
        let leaf = root.appendingPathComponent(InputEventJournalFile.name)
        XCTAssertEqual(try Data(contentsOf: leaf), point == .directorySync ? next : old)
        XCTAssertEqual(
          try FileManager.default.contentsOfDirectory(atPath: root.path),
          [InputEventJournalFile.name])
        // The common journal may retry its same owned leaf. Native recording
        // has a separate terminal gate and must not turn this into success.
        fault.disarm()
        try file.replace(next)
        XCTAssertEqual(try Data(contentsOf: leaf), next)
      }
    }
  }
  func testCaptureFailureGuidanceKeepsCodesBoundedAndNeverInfersRecoveredOrClosedAudio() throws {
    for code in [
      "raw_segment_storage_full", "raw_segment_storage_io_failed", "capture_clock_failed",
      "capture_audio_write_failed", "raw_segment_write_or_sync_failed",
      "raw_segment_final_sync_failed",
    ] {
      let message = CaptureFailureGuide.message(code)
      XCTAssertTrue(message.contains("（\(code)）"))
      XCTAssertTrue(message.contains("中断録音の部分救出"))
      XCTAssertTrue(message.contains("保証しません"))
      XCTAssertLessThan(message.utf8.count, 1000)
      var stats = TrackStats()
      if code == "capture_clock_failed" {
        stats.clockFailure = "invalid measured clock"
      } else if code == "capture_audio_write_failed" {
        stats.appendFailures = 1
      } else {
        stats.rawSegments = RawSegmentStatus(
          ownerId: "735027d4-a03e-43d3-830a-21e767dfaf88", lane: "microphone", sealedSegments: 0,
          sealedFrames: 0, lastEntrySHA256: nil, producerClosed: false, failure: code)
      }
      XCTAssertEqual(stats.captureFailure, code)
      XCTAssertThrowsError(try stats.requireWritable()) {
        XCTAssertEqual(String(describing: $0), message)
      }
    }
    for unknown in ["raw_segment_unknown", "x raw_segment_storage_full", ""] {
      XCTAssertEqual(CaptureFailureGuide.message(unknown), unknown)
    }
  }
}
