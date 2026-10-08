import Foundation

/// A retained producer suffix, NOT a delivery receipt or live telemetry. The
/// observer only copies values under a short lock. Encoding, file I/O and sync
/// run on a separate serial queue, never on a PCM callback or policy lock.
final class InputEventJournal: @unchecked Sendable {
  typealias Observer = @Sendable (MicrophoneRecovery.Status, InputEventHistory) -> Void
  enum Lane: Sendable { case microphone, system }

  struct Record: Encodable, Sendable {
    let schemaVersion = 1
    let sealed: Bool
    let report: InputStatusReport

    func encodedData() throws -> Data {
      _ = try report.encodedData()
      let data = try JSONEncoder().encode(self)
      guard data.count <= 65 * 1024 else {
        throw ProbeError.capture("input event journal exceeds limit")
      }
      return data
    }
  }

  private struct State: Equatable, Sendable {
    let status: MicrophoneRecovery.Status
    let history: InputEventHistory

    static let initial = State(
      status: .init(state: "reconnecting", sourceEpoch: 0, interruptions: 0, reason: nil),
      history: InputEventHistory())

    var stopped: State {
      State(
        status: .init(
          state: "stopped", sourceEpoch: status.sourceEpoch,
          interruptions: status.interruptions, reason: status.reason), history: history)
    }

    func advances(_ old: State, input: String) -> Bool {
      history.matches(status, input: input)
        && status.sourceEpoch >= old.status.sourceEpoch
        && status.interruptions >= old.status.interruptions
        && (old.status.state != "stopped" || self == old)
        && history.lastSequence >= old.history.lastSequence
        && history.droppedThrough >= old.history.droppedThrough
        && (history.lastSequence != old.history.lastSequence || history == old.history)
        && old.history.events.allSatisfy { event in
          event.sequence <= history.droppedThrough || history.events.contains(event)
        }
    }
  }

  private let owner: String
  private let condition = NSLock()
  private let drainGroup = DispatchGroup()
  private let queue = DispatchQueue(label: "app.mon.capture.input-event-journal")
  private let file: InputEventJournalFile
  // Test hook runs only on the writer queue. It must never be called by the
  // observer. Normal recording always uses the real anchored writer below.
  private let beforeWrite: (@Sendable () throws -> Void)?
  private var microphone = State.initial
  private var system: State?
  private var revision: Int64 = 1
  private var committed: Int64 = 1
  private var pending: Record?
  private var scheduled = false
  private var closed = false
  private var invalid: String?
  private var failure: String?

  init(
    output: URL, owner: String, supportsSystem: Bool,
    beforeWrite: (@Sendable () throws -> Void)? = nil
  ) throws {
    guard InputLevelReport.validOwner(owner) else {
      throw ProbeError.capture("input event journal owner is invalid")
    }
    self.owner = owner
    self.system = supportsSystem ? .initial : nil
    self.beforeWrite = beforeWrite
    let initial = Record(
      sealed: false,
      report: InputStatusReport(
        ownerId: owner, revision: 1, microphone: State.initial.status,
        system: supportsSystem ? State.initial.status : nil,
        microphoneHistory: State.initial.history,
        systemHistory: supportsSystem ? State.initial.history : nil))
    // Exclusive creation and initial sync happen before audio initialization.
    // Never adopt or overwrite an older recording's journal.
    self.file = try InputEventJournalFile(output: output, initial: initial.encodedData())
  }

  func observer(_ lane: Lane) -> Observer {
    { [weak self] status, history in self?.observe(lane, status: status, history: history) }
  }

  /// Never waits for the writer or reads its file. In particular, pending sync
  /// must not block a policy/PCM callback or become a fabricated success.
  func persistence() -> InputJournalPersistence {
    condition.lock()
    defer { condition.unlock() }
    return InputJournalPersistence(
      requestedRevision: revision, committedRevision: committed,
      state: invalid != nil
        ? "invalid"
        : failure != nil
          ? "failed"
          : scheduled || committed != revision ? "pending" : "confirmed",
      closed: closed)
  }

  private func observe(
    _ lane: Lane, status: MicrophoneRecovery.Status, history: InputEventHistory
  ) {
    condition.lock()
    defer { condition.unlock() }
    guard !closed, invalid == nil else { return }
    let next = State(status: status, history: history)
    let old = lane == .microphone ? microphone : system
    guard let old, next.advances(old, input: lane == .microphone ? "microphone" : "system") else {
      invalid = "input event journal rejected an inconsistent transition"
      return
    }
    guard next != old else { return }
    if lane == .microphone { microphone = next } else { system = next }
    schedule(sealed: false)
  }

  /// Called only after stopping both policies, including the error path. Sealed
  /// means no more producer mutations, NOT that audio or notifications are
  /// saved/displayed. A bounded wait reports uncertain commit rather than
  /// pretending a timeout cancelled the writer or deleting its evidence.
  func finish(timeout: TimeInterval = 3) throws {
    condition.lock()
    if !closed {
      closed = true
      microphone = microphone.stopped
      system = system?.stopped
      if invalid == nil { schedule(sealed: true) }
    }
    condition.unlock()
    try flush(timeout: timeout)
  }

  func flush(timeout: TimeInterval = 3) throws {
    guard timeout.isFinite, timeout >= 0, timeout <= 5 else {
      throw ProbeError.capture("input event journal wait is invalid")
    }
    // DispatchTime is monotonic. A wall-clock correction must not stretch the
    // bounded stop wait or manufacture a persistence receipt.
    let result = drainGroup.wait(timeout: .now() + timeout)
    condition.lock()
    defer { condition.unlock() }
    guard result == .success, !scheduled else {
      throw ProbeError.capture("input event journal commit is still unconfirmed")
    }
    guard invalid == nil, failure == nil, committed == revision else {
      throw ProbeError.capture(invalid ?? failure ?? "input event journal commit is unconfirmed")
    }
  }

  // Must hold condition. Coalesce only snapshots, never renumber events. Each
  // lane's newest 64 transitions and eviction floor remain in the pending copy.
  // At most one writing and one pending copy exist even when disk I/O stalls.
  private func schedule(sealed: Bool) {
    guard revision < Int64(InputEventHistory.safe) else {
      invalid = "input event journal revision is exhausted"
      return
    }
    revision += 1
    pending = Record(
      sealed: sealed,
      report: InputStatusReport(
        ownerId: owner, revision: revision, microphone: microphone.status,
        system: system?.status, microphoneHistory: microphone.history,
        systemHistory: system?.history))
    if !scheduled {
      scheduled = true
      drainGroup.enter()
      queue.async { self.drain() }
    }
  }

  private func drain() {
    while true {
      condition.lock()
      guard let next = pending else {
        scheduled = false
        drainGroup.leave()
        condition.unlock()
        return
      }
      pending = nil
      condition.unlock()
      do {
        try beforeWrite?()
        try file.replace(next.encodedData())
        condition.lock()
        committed = next.report.revision
        failure = nil
        condition.unlock()
      } catch {
        condition.lock()
        // A short stable reason, never an arbitrary path or growing error log.
        failure = "input event journal could not confirm its file commit"
        condition.unlock()
      }
    }
  }
}
