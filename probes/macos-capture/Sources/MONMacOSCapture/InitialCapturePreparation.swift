import Foundation

/// A deadline decides admission, not ownership. Cancellation must invalidate
/// the recorder's binding; its real native operation remains owned until it
/// returns and is retired. Final stop must still prove that retirement.
final class InitialCapturePreparation: @unchecked Sendable {
  enum Outcome: Equatable, Sendable {
    case ready
    case unavailable(String)
    case timedOut
  }
  private let lock = NSLock()
  private var result: Outcome?

  static func run(
    timeout: Duration = .seconds(5),
    operation: @escaping @Sendable () async throws -> Void,
    cancel: @escaping @Sendable () -> Void
  ) async -> Outcome {
    let state = InitialCapturePreparation()
    let task = Task {
      let result: Outcome
      do {
        try await operation()
        result = .ready
      } catch { result = .unavailable(String(describing: error)) }
      state.lock.withLock { if state.result == nil { state.result = result } }
    }
    let deadline = ContinuousClock.now.advanced(by: timeout)
    while ContinuousClock.now < deadline, !Task.isCancelled {
      if let result = state.lock.withLock({ state.result }) { return result }
      try? await Task.sleep(for: .milliseconds(10))
    }
    let result = state.lock.withLock { () -> Outcome in
      if let result = state.result { return result }
      state.result = .timedOut
      return .timedOut
    }
    if result == .timedOut {
      cancel()
      task.cancel()
    }
    return result
  }
}
