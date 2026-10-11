import CoreMedia
import Foundation
import ScreenCaptureKit

/// Driver calls are serialized away from the helper's main actor. Native stop
/// must close and drain the actual IO callback before its receipt is delivered.
protocol CoreAudioCaptureDriver: AnyObject, Sendable {
  func start(
    sample: @escaping @Sendable (CMSampleBuffer) -> Void,
    failure: @escaping @Sendable (Error) -> Void
  ) throws
  func stop() throws
  func detach() throws
}

final class CoreAudioCaptureStream: SystemCaptureStream, @unchecked Sendable {
  let native: SCStream? = nil
  var synchronizationClock: CMClock? { CMClockGetHostTimeClock() }
  let applicationName: String?
  let bundleIdentifier: String?
  private let driver: any CoreAudioCaptureDriver
  private let operationQueue = DispatchQueue(label: "app.mon.capture.tap-control")
  private let deliveryQueue = DispatchQueue(label: "app.mon.capture.tap-delivery")
  private let pending = DispatchSemaphore(value: 64)
  private let lock = NSLock()
  private var stopRequested = false
  private var started = false
  private var startAttempted = false
  private var detached = false
  private var acceptsSamples = false
  private var failureReported = false
  private let mayContinue: @Sendable () -> Bool
  private let deliver: @Sendable (CoreAudioCaptureStream, CMSampleBuffer) -> Void
  private let ended: @Sendable (CoreAudioCaptureStream, Error) -> Void

  init(
    driver: any CoreAudioCaptureDriver, applicationName: String? = nil,
    bundleIdentifier: String? = nil,
    mayContinue: @escaping @Sendable () -> Bool = { true },
    deliver: @escaping @Sendable (CoreAudioCaptureStream, CMSampleBuffer) -> Void,
    ended: @escaping @Sendable (CoreAudioCaptureStream, Error) -> Void
  ) {
    self.driver = driver
    self.applicationName = applicationName
    self.bundleIdentifier = bundleIdentifier
    self.mayContinue = mayContinue
    self.deliver = deliver
    self.ended = ended
  }

  func start() async throws {
    try await operation { source in
      guard source.mayContinue(),
        source.lock.withLock({
          guard !source.stopRequested && !source.startAttempted && !source.detached else {
            return false
          }
          source.startAttempted = true
          return true
        })
      else { throw ProbeError.capture("tap start is obsolete or stopped") }
      source.lock.withLock { source.acceptsSamples = true }
      do {
        try source.driver.start(
          sample: { [weak source] in source?.receive($0) },
          failure: { [weak source] in source?.fail($0) })
        source.lock.withLock { source.started = true }
        guard source.mayContinue(), source.lock.withLock({ !source.stopRequested }) else {
          throw ProbeError.capture("tap start completed after a stop request")
        }
      } catch {
        source.lock.withLock { source.acceptsSamples = false }
        throw error
      }
    }
  }

  func stop() async throws {
    try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
      requestStop { error in
        if let error { continuation.resume(throwing: error) } else { continuation.resume() }
      }
    }
  }

  func requestStop(_ completion: @escaping @Sendable (Error?) -> Void) {
    lock.withLock { stopRequested = true }
    operationQueue.async { [self] in
      do {
        try driver.stop()
        lock.withLock {
          acceptsSamples = false
          started = false
        }
        deliveryQueue.sync {}
        completion(nil)
      } catch {
        lock.withLock { acceptsSamples = false }
        deliveryQueue.sync {}
        completion(error)
      }
    }
  }

  func detach() throws {
    // This synchronous call is made only from the recorder's control task,
    // never the tap's callback/delivery queues.
    try operationQueue.sync {
      if lock.withLock({ detached }) { return }
      // The recorder can retire a source after a native failure notification,
      // without an explicit UI stop. A failure is not proof that IO stopped.
      try driver.stop()
      lock.withLock {
        acceptsSamples = false
        started = false
      }
      deliveryQueue.sync {}
      try driver.detach()
      lock.withLock {
        detached = true
        acceptsSamples = false
      }
      deliveryQueue.sync {}
    }
  }

  private struct OwnedSample: @unchecked Sendable { let value: CMSampleBuffer }

  private func receive(_ sample: CMSampleBuffer) {
    guard lock.withLock({ acceptsSamples && !detached && !failureReported }) else { return }
    guard pending.wait(timeout: .now()) == .success else {
      fail(ProbeError.capture("tap delivery queue exceeded its bounded capacity"))
      return
    }
    let owned = OwnedSample(value: sample)
    deliveryQueue.async { [self] in
      defer { pending.signal() }
      // Samples admitted before the actual OS stop are drained, not replaced
      // with invented silence. Recorder ownership still rejects retired epochs.
      if lock.withLock({ !detached && !failureReported }) { deliver(self, owned.value) }
    }
  }

  private func fail(_ error: Error) {
    let notify = lock.withLock {
      guard !detached, !failureReported else { return false }
      failureReported = true
      acceptsSamples = false
      return true
    }
    if notify {
      operationQueue.async { [self] in
        // Leave failed-stop resources owned. Detach/replacement must retry and
        // prove stop rather than interpreting the error as an OS stop receipt.
        do {
          try driver.stop()
          lock.withLock { started = false }
        } catch {}
        deliveryQueue.async { [self] in ended(self, error) }
      }
    }
  }

  private func operation(_ body: @escaping @Sendable (CoreAudioCaptureStream) throws -> Void)
    async throws
  {
    try await withCheckedThrowingContinuation { continuation in
      operationQueue.async { [self] in
        do {
          try body(self)
          continuation.resume()
        } catch { continuation.resume(throwing: error) }
      }
    }
  }
}
