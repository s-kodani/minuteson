import Darwin
import Foundation

/// The native owner atomically replaces this bounded control file. A request
/// changes policy only; it does not claim that new PCM has been acquired.
struct MicrophoneSelectionRequest: Decodable, Equatable {
  let schemaVersion: Int
  let ownerId: String
  let revision: Int64
  let mode: String
  let uid: String?

  func valid(for owner: String, after previousRevision: Int64) -> Bool {
    schemaVersion == 1 && ownerId == owner && InputLevelReport.validOwner(owner)
      && revision > previousRevision && revision <= 9_007_199_254_740_991
      && ((mode == "automatic" && uid == nil)
        || (mode == "fixed" && uid.map(InputDevices.validUID) == true))
  }
}

final class MicrophoneSelectionReader {
  private let path: String
  private let owner: String
  private(set) var revision: Int64 = 0

  init(output: URL, owner: String) throws {
    guard InputLevelReport.validOwner(owner) else {
      throw ProbeError.capture("microphone selection owner is invalid")
    }
    path = output.appendingPathComponent("microphone-selection.request").path
    self.owner = owner
  }

  func next() throws -> MicrophoneSelectionRequest? {
    let fd = open(path, O_RDONLY | O_NOFOLLOW | O_CLOEXEC)
    if fd < 0 {
      if errno == ENOENT { return nil }
      throw ProbeError.capture("microphone selection request cannot be opened")
    }
    defer { _ = close(fd) }
    var info = stat()
    guard fstat(fd, &info) == 0, (info.st_mode & mode_t(S_IFMT)) == mode_t(S_IFREG),
      info.st_size > 0, info.st_size <= 4096
    else { throw ProbeError.capture("microphone selection request is not a bounded file") }
    var bytes = [UInt8](repeating: 0, count: 4097)
    let count = bytes.withUnsafeMutableBytes { buffer in read(fd, buffer.baseAddress, buffer.count)
    }
    guard count > 0, count <= 4096 else {
      throw ProbeError.capture("microphone selection request cannot be read")
    }
    let request = try JSONDecoder().decode(
      MicrophoneSelectionRequest.self, from: Data(bytes.prefix(count)))
    guard request.valid(for: owner, after: 0) else {
      throw ProbeError.capture("microphone selection request is invalid")
    }
    guard request.revision > revision else { return nil }
    return request
  }

  func acknowledge(_ request: MicrophoneSelectionRequest) throws {
    guard request.valid(for: owner, after: revision) else {
      throw ProbeError.capture("microphone selection acknowledgement is invalid")
    }
    revision = request.revision
  }
}
