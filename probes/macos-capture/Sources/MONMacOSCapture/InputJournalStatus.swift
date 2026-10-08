import Foundation

/// A bounded copy of the journal writer's result, not an audio persistence or
/// notification-render receipt. Journal revisions are NOT heartbeat revisions.
struct InputJournalPersistence: Encodable, Equatable, Sendable {
  let requestedRevision: Int64
  let committedRevision: Int64
  let state: String
  let closed: Bool

  var valid: Bool {
    requestedRevision > 0 && requestedRevision <= Int64(InputEventHistory.safe)
      && committedRevision > 0 && committedRevision <= requestedRevision
      && ["confirmed", "pending", "failed", "invalid"].contains(state)
      && (state != "confirmed" || requestedRevision == committedRevision)
  }
}

struct InputJournalStatusReport: Encodable, Sendable {
  let schemaVersion = 1
  let ownerId: String
  let revision: Int64
  let persistence: InputJournalPersistence

  func encodedData() throws -> Data {
    guard InputLevelReport.validOwner(ownerId), revision > 0,
      revision <= Int64(InputEventHistory.safe), persistence.valid
    else { throw ProbeError.capture("input journal status is invalid") }
    let data = try JSONEncoder().encode(self)
    guard data.count <= 4096 else { throw ProbeError.capture("input journal status exceeds limit") }
    return data
  }

  func publish(to output: URL) throws {
    try encodedData().write(
      to: output.appendingPathComponent("input-journal-status.json"), options: .atomic)
  }
}

/// Production telemetry is descriptor-relative too. Its unsynced atomic update
/// is not a new journal commit receipt; only the copied writer result is shown.
final class InputJournalStatusPublisher {
  private let owner: String
  private let file: InputEventJournalFile
  private var revision: Int64 = 1

  init(output: URL, owner: String, initial: InputJournalPersistence) throws {
    self.owner = owner
    self.file = try InputEventJournalFile(
      output: output,
      initial: InputJournalStatusReport(ownerId: owner, revision: 1, persistence: initial)
        .encodedData(),
      leaf: .status, synchronizes: false)
  }

  func publish(_ persistence: InputJournalPersistence) throws {
    guard revision < Int64(InputEventHistory.safe) else {
      throw ProbeError.capture("input journal diagnostic heartbeat is exhausted")
    }
    revision += 1
    try file.replace(
      InputJournalStatusReport(
        ownerId: owner, revision: revision, persistence: persistence
      ).encodedData())
  }

  func removeIfOwned() { file.removeEphemeralIfOwned() }
}
