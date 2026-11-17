import Foundation

/// One bounded timer owns telemetry IO and revisions. Native reconfiguration
/// may wait for Bluetooth on the control thread; it must not age the other
/// input's live status. Every tick reads the actual policies, never old reports.
final class CaptureTelemetryPublisher: @unchecked Sendable {
  private let queue = DispatchQueue(label: "app.mon.capture.telemetry")
  private let key = DispatchSpecificKey<UInt8>()
  private var timer: DispatchSourceTimer?
  private var stopped = false
  private var revision: Int64 = 0
  private let output: URL
  private let owner: String
  private let publishesStatus: Bool
  private let snapshots: @Sendable () -> (TrackStats, TrackStats)
  private let journal: @Sendable () -> Void

  init(
    output: URL, owner: String, publishesStatus: Bool,
    snapshots: @escaping @Sendable () -> (TrackStats, TrackStats),
    journal: @escaping @Sendable () -> Void = {}
  ) {
    self.output = output
    self.owner = owner
    self.publishesStatus = publishesStatus
    self.snapshots = snapshots
    self.journal = journal
    queue.setSpecific(key: key, value: 1)
  }

  func start() {
    queue.sync {
      guard !stopped, timer == nil else { return }
      publish()
      let source = DispatchSource.makeTimerSource(queue: queue)
      source.schedule(deadline: .now(), repeating: .milliseconds(100), leeway: .milliseconds(10))
      source.setEventHandler { [weak self] in self?.publish() }
      timer = source
      source.resume()
    }
  }

  /// Drains the sole publication owner before session cleanup removes leaves.
  /// DispatchSource coalesces timer ticks; slow IO cannot queue unlimited jobs.
  func stop() {
    let cancel = {
      self.stopped = true
      self.timer?.setEventHandler {}
      self.timer?.cancel()
      self.timer = nil
    }
    if DispatchQueue.getSpecific(key: key) != nil { cancel() } else { queue.sync(execute: cancel) }
  }

  deinit { stop() }

  private func publish() {
    guard !stopped, revision < Int64(InputEventHistory.safe) else { return }
    revision += 1
    let (microphone, system) = snapshots()
    if publishesStatus, let status = microphone.microphoneRecovery,
      let history = microphone.inputEventHistory
    {
      try? InputStatusReport(
        ownerId: owner, revision: revision, microphone: status,
        system: system.systemRecovery, microphoneHistory: history,
        systemHistory: system.inputEventHistory
      ).publish(to: output)
    }
    journal()
    let now = PCMInputLevel.nowNanoseconds()
    try? InputLevelReport(
      ownerId: owner, revision: revision,
      microphone: microphone.inputLevel.snapshot(at: now),
      system: system.inputLevel.snapshot(at: now)
    ).publish(to: output)
  }
}
