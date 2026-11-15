import Foundation

/// One session owns this worker. Each turn commits one immutable segment;
/// alternating lanes prevents a delayed input from monopolizing disk syncs.
final class RawSegmentCommitWorker: @unchecked Sendable {
  // Reserve the maximum receipt and tip plus prefix/job bookkeeping while a
  // segment is active, not only after metadata has been encoded.
  static let metadataReserveBytes = CaptureCore.maximumEntryBytes + 1024 + 512
  static let segmentAudioReserveBytes = 128_068
  static let segmentReserveBytes = segmentAudioReserveBytes + metadataReserveBytes
  static let controlReserveBytes = 1024 + 512
  private struct Job: Sendable {
    let bytes: Int
    let run: @Sendable () -> Void
  }
  private let condition = NSCondition()
  private let queue = DispatchQueue(label: "app.mon.capture.segment-commits")
  private var jobs: [String: [Job]] = [:]
  private var counts: [String: Int] = [:]
  private var acquired: [String: Int64] = [:]
  private var compactBytes = 0
  private var compactJobs = 0
  private var running = false
  private var previousLane = "system"

  func reserveAcquired(lane: String, nanoseconds: Int64) -> Bool {
    condition.lock()
    defer { condition.unlock() }
    let pending = acquired[lane, default: 0]
    guard CaptureCore.permitsStoragePending(
      bytes: 0, nanoseconds: pending, jobs: 0,
      nextBytes: 0, nextNanoseconds: nanoseconds)
    else { return false }
    acquired[lane] = pending + nanoseconds
    return true
  }

  func releaseAcquired(lane: String, nanoseconds: Int64) {
    condition.lock()
    acquired[lane, default: 0] -= nanoseconds
    assert(acquired[lane, default: 0] >= 0)
    condition.unlock()
  }

  func beginSegment() throws {
    condition.lock()
    defer { condition.unlock() }
    guard CaptureCore.permitsStoragePending(
      bytes: compactBytes, nanoseconds: 0, jobs: compactJobs,
      nextBytes: Self.segmentReserveBytes, nextNanoseconds: 0)
    else { throw ProbeError.capture("compact segment queue exceeded its bounded capacity") }
    compactJobs += 1
    compactBytes += Self.segmentReserveBytes
  }

  func beginControl() throws {
    condition.lock()
    defer { condition.unlock() }
    guard CaptureCore.permitsStoragePending(
      bytes: compactBytes, nanoseconds: 0, jobs: compactJobs,
      nextBytes: Self.controlReserveBytes, nextNanoseconds: 0)
    else { throw ProbeError.capture("compact commit queue exceeded its bounded capacity") }
    compactJobs += 1
    compactBytes += Self.controlReserveBytes
  }

  func validateSegmentBytes(current: Int, next: Int) throws {
    guard current >= 68, next >= 0, current <= Self.segmentAudioReserveBytes - next else {
      throw ProbeError.capture("compact segment exceeded its reserved byte capacity")
    }
  }

  func submit(lane: String, bytes: Int, run: @escaping @Sendable () -> Void) {
    condition.lock()
    defer { condition.unlock() }
    counts[lane, default: 0] += 1
    jobs[lane, default: []].append(Job(bytes: bytes, run: run))
    if !running {
      running = true
      queue.async { [self] in process() }
    }
  }

  func drain(lane: String) {
    condition.lock()
    while counts[lane, default: 0] > 0 { condition.wait() }
    condition.unlock()
  }

  func pending(lane: String) -> (nanoseconds: Int64, bytes: Int, jobs: Int) {
    condition.lock()
    defer { condition.unlock() }
    return (acquired[lane, default: 0], compactBytes, compactJobs)
  }

  private func process() {
    while true {
      condition.lock()
      let order = previousLane == "microphone" ? ["system", "microphone"] : ["microphone", "system"]
      guard let lane = order.first(where: { !(jobs[$0] ?? []).isEmpty }) else {
        running = false
        condition.broadcast()
        condition.unlock()
        return
      }
      let job = jobs[lane]!.removeFirst()
      previousLane = lane
      condition.unlock()
      job.run()
      condition.lock()
      compactBytes -= job.bytes
      compactJobs -= 1
      counts[lane, default: 0] -= 1
      condition.broadcast()
      condition.unlock()
    }
  }
}
