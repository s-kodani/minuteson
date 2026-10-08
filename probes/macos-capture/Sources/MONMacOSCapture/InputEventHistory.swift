import Foundation

/// Control-plane transitions, captured at the policy mutation rather than at
/// the UI/heartbeat sampling rate. No PCM callback arrival is a recovery event.
/// The bounded suffix explicitly names the evicted prefix; it is not a claim
/// that every event has been displayed or retained through process termination.
struct InputEventHistory: Encodable, Equatable, Sendable {
  static let capacity = 64
  static let safe: UInt64 = 9_007_199_254_740_991

  struct Event: Encodable, Equatable, Sendable {
    let sequence: UInt64
    let kind: String
    let status: MicrophoneRecovery.Status
  }

  private(set) var lastSequence: UInt64 = 0
  private(set) var droppedThrough: UInt64 = 0
  private(set) var events: [Event] = []

  mutating func append(_ kind: String, status: MicrophoneRecovery.Status) {
    // On exhaustion the heartbeat validation fails closed, without trapping or
    // stopping audio capture. There is no wrapping, renumbering, or fake event.
    guard lastSequence < Self.safe else { return }
    lastSequence += 1
    events.append(Event(sequence: lastSequence, kind: kind, status: status))
    if events.count > Self.capacity {
      droppedThrough = events.removeFirst().sequence
    }
  }

  func matches(_ current: MicrophoneRecovery.Status, input: String = "microphone") -> Bool {
    guard InputStatusReport.validStatus(current, input: input),
      lastSequence <= Self.safe, droppedThrough <= lastSequence,
      events.count <= Self.capacity,
      lastSequence - droppedThrough == UInt64(events.count),
      droppedThrough
        == (lastSequence > UInt64(Self.capacity)
          ? lastSequence - UInt64(Self.capacity) : 0),
      current.interruptions == lastSequence / 2 + lastSequence % 2
    else { return false }
    var prior: Event?
    for (index, event) in events.enumerated() {
      let sequence = droppedThrough + UInt64(index) + 1
      let interrupted = sequence % 2 == 1
      guard event.sequence == sequence,
        InputStatusReport.validStatus(event.status, input: input),
        event.kind == (interrupted ? "interrupted" : "recovered"),
        event.status.interruptions == sequence / 2 + sequence % 2,
        event.status.sourceEpoch <= Self.safe,
        event.status.sourceEpoch <= current.sourceEpoch,
        interrupted
          ? (["disconnected", "unavailable"].contains(event.status.state)
            && event.status.reason != nil)
          : (event.status.state == "connected" && event.status.reason == nil
            && event.status.sourceEpoch > event.status.interruptions)
      else { return false }
      if let prior {
        guard event.status.sourceEpoch >= prior.status.sourceEpoch,
          interrupted || event.status.sourceEpoch > prior.status.sourceEpoch
        else { return false }
      }
      prior = event
    }
    guard let last = events.last else { return current.interruptions == 0 }
    return current.state == "stopped"
      || (last.kind == "recovered" ? current == last.status : current.state != "connected")
  }
}
