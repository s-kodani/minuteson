import Foundation
import IOKit.pwr_mgt

protocol RecordingPowerBackend {
  func acquire() throws -> IOPMAssertionID
  func release(_ id: IOPMAssertionID) throws
}

private struct NativeRecordingPowerBackend: RecordingPowerBackend {
  func acquire() throws -> IOPMAssertionID {
    var id = IOPMAssertionID(kIOPMNullAssertionID)
    let status = IOPMAssertionCreateWithName(
      kIOPMAssertionTypePreventUserIdleSystemSleep as CFString,
      IOPMAssertionLevel(kIOPMAssertionLevelOn), "MON audio recording" as CFString, &id)
    guard status == kIOReturnSuccess else {
      throw RecordingPowerError(operation: "acquire", status: status)
    }
    return id
  }

  func release(_ id: IOPMAssertionID) throws {
    let status = IOPMAssertionRelease(id)
    guard status == kIOReturnSuccess else {
      throw RecordingPowerError(operation: "release", status: status)
    }
  }
}

struct RecordingPowerError: Error, CustomStringConvertible {
  let operation: String
  let status: IOReturn
  var description: String {
    "recording idle-sleep protection \(operation) failed (IOKit \(status))"
  }
}

/// One assertion owned by the capture process, not by a window or AI worker.
/// Display sleep and user-requested system sleep are deliberately not blocked.
final class RecordingPowerAssertion {
  private let backend: any RecordingPowerBackend
  private(set) var id: IOPMAssertionID?

  init(backend: (any RecordingPowerBackend)? = nil) {
    self.backend = backend ?? NativeRecordingPowerBackend()
  }

  func acquire() throws {
    if id == nil { id = try backend.acquire() }
  }

  func release() throws {
    guard let id else { return }
    try backend.release(id)
    // Keep ownership after a failed release so the cleanup path can retry.
    self.id = nil
  }

  deinit {
    if let id { try? backend.release(id) }
  }
}

struct RecordingPowerStatus: Encodable {
  let assertionType = "PreventUserIdleSystemSleep"
  let allowsDisplaySleep = true
  var acquired = false
  var released = false
}
