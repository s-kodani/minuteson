import Foundation
import XCTest

@testable import MONMacOSCapture

final class PreparationGate: @unchecked Sendable {
  private let lock = NSLock()
  private var continuation: CheckedContinuation<Void, Never>?
  private var released = false
  private var cancelled = false
  private var finished = false
  func wait() async {
    await withCheckedContinuation { continuation in
      let resume = lock.withLock { () -> Bool in
        if released { return true }
        self.continuation = continuation
        return false
      }
      if resume { continuation.resume() }
    }
    lock.withLock { finished = true }
  }
  func cancel() { lock.withLock { cancelled = true } }
  var wasCancelled: Bool { lock.withLock { cancelled } }
  var didFinish: Bool { lock.withLock { finished } }
  func release() {
    let pending = lock.withLock {
      released = true
      let pending = continuation
      continuation = nil
      return pending
    }
    pending?.resume()
  }
}

final class InitialCapturePreparationTests: XCTestCase, @unchecked Sendable {
  func testReadyAndFailedPreparationsAreIndependent() async {
    let failed = await InitialCapturePreparation.run(
      operation: { throw ProbeError.permission("denied") },
      cancel: { XCTFail("completed failure is not a timeout") })
    guard case .unavailable(let reason) = failed else {
      return XCTFail("initial failure must be explicit")
    }
    XCTAssertTrue(reason.contains("denied"))
    let ready = await InitialCapturePreparation.run(
      operation: {}, cancel: { XCTFail("healthy lane") })
    XCTAssertEqual(ready, .ready)
  }
  func testDeadlineInvalidatesAdmissionButDoesNotPretendPendingNativeWorkEnded() async throws {
    let gate = PreparationGate()
    let result = await InitialCapturePreparation.run(
      timeout: .milliseconds(20), operation: { await gate.wait() }, cancel: gate.cancel)
    XCTAssertEqual(result, .timedOut)
    XCTAssertTrue(gate.wasCancelled)
    XCTAssertFalse(gate.didFinish)
    gate.release()
    let deadline = ContinuousClock.now.advanced(by: .seconds(1))
    while !gate.didFinish, ContinuousClock.now < deadline {
      try await Task.sleep(for: .milliseconds(1))
    }
    XCTAssertTrue(gate.didFinish)
    XCTAssertEqual(result, .timedOut)
  }
}
