import Foundation

/// Frozen recording scope and one owned asynchronous acquisition at a time.
/// Silence is not a lifecycle event. Only control-plane evidence interrupts a
/// binding; a timed-out OS request remains owned until it actually returns.
struct SystemAudioRecovery {
  struct Binding: Equatable, Sendable { let epoch: UInt64 }
  typealias Status = MicrophoneRecovery.Status

  let targetPID: pid_t?
  let applicationIdentity: CaptureApplicationIdentity?
  private(set) var operation: Binding?
  private var binding: Binding?
  private var lifecycle = CaptureLifecycle()
  private var epoch: UInt64 { lifecycle.epoch }
  private var interruptions: UInt64 { lifecycle.interruptions }
  private var state: String { lifecycle.state }
  private var reason: String?
  private var stopped: Bool { lifecycle.stopped }
  private var formatBlocked = false
  private var startedAt: Int64?
  private var lastAttempt: Int64?
  private(set) var history = InputEventHistory()
  private let observe: InputEventJournal.Observer?

  init(
    targetPID: pid_t?, applicationIdentity: CaptureApplicationIdentity?, retries: Bool,
    observe: InputEventJournal.Observer? = nil
  ) throws {
    guard targetPID.map({ $0 > 0 }) ?? true,
      applicationIdentity.map({ $0.valid && $0.pid == targetPID }) ?? true,
      !retries || targetPID == nil || applicationIdentity != nil
    else { throw ProbeError.capture("system recovery requires the exact confirmed scope") }
    self.targetPID = targetPID
    self.applicationIdentity = applicationIdentity
    self.observe = observe
  }

  var status: Status {
    Status(state: state, sourceEpoch: epoch, interruptions: interruptions, reason: reason)
  }

  func requireScope(targetPID: pid_t?, identity: CaptureApplicationIdentity?) throws {
    guard targetPID == self.targetPID, identity == applicationIdentity else {
      throw ProbeError.capture("system recovery cannot change the confirmed recording scope")
    }
  }

  mutating func begin(at now: Int64) throws -> Binding {
    guard !stopped, !formatBlocked, binding == nil, operation == nil,
      now >= 0, lastAttempt.map({ now >= $0 && now - $0 >= CaptureCore.retryNanoseconds }) ?? true,
      epoch < 9_007_199_254_740_991
    else { throw ProbeError.capture("system recovery cannot begin another acquisition") }
    guard lifecycle.begin() else { throw ProbeError.capture("system recovery capacity exceeded") }
    let token = Binding(epoch: epoch)
    binding = token
    operation = token
    startedAt = now
    lastAttempt = now
    observe?(status, history)
    return token
  }

  func owns(_ token: Binding) -> Bool { !stopped && binding == token }
  func accepts(_ token: Binding) -> Bool { owns(token) && state == "connected" }
  func acceptsPCM(_ token: Binding) -> Bool {
    owns(token) && (state == "connected" || state == "reconnecting")
  }

  mutating func beginRetirement(at now: Int64) throws -> Binding {
    guard !stopped, operation == nil, binding == nil, epoch > 0, now >= 0 else {
      throw ProbeError.capture("system stream retirement is already owned")
    }
    let token = Binding(epoch: epoch)
    operation = token
    startedAt = now
    return token
  }

  mutating func activate(_ token: Binding) throws {
    guard owns(token), operation == token, state == "reconnecting" else {
      throw ProbeError.capture("system acquisition result belongs to an obsolete stream")
    }
    guard lifecycle.connect() else { throw ProbeError.capture("system recovery is stopped") }
    reason = nil
    if interruptions > 0 { history.append("recovered", status: status) }
    observe?(status, history)
  }

  mutating func interrupt(_ reason: String, unavailable: Bool = false) {
    guard !stopped, !formatBlocked else { return }
    if reason == "format_changed" { formatBlocked = true }
    guard let first = lifecycle.interrupt(unavailable: unavailable) else { return }
    binding = nil
    self.reason = reason
    if first { history.append("interrupted", status: status) }
    observe?(status, history)
  }

  mutating func expire(at now: Int64) -> Bool {
    guard operation != nil, let startedAt, !stopped else { return false }
    if now < startedAt || now - startedAt >= CaptureCore.acquisitionTimeoutNanoseconds {
      interrupt("acquisition_timeout", unavailable: true)
      // Do not clear operation: cancellation cannot prove an OS call ended.
      return true
    }
    return false
  }

  mutating func operationFinished(_ token: Binding) {
    guard operation == token else { return }
    operation = nil
    startedAt = nil
  }

  mutating func stop() {
    lifecycle.stop()
    binding = nil
    reason = nil
    observe?(status, history)
  }
}
