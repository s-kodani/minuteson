import Foundation

// A heartbeat of the actual binding policy, not evidence of received or saved PCM.
// Only ordinary recording publishes this leaf; private five-second tests keep
// their existing fixed-file ownership and cleanup contract.
struct InputStatusReport: Encodable {
  let schemaVersion = 1
  let ownerId: String
  let revision: Int64
  let microphone: MicrophoneRecovery.Status

  func publish(to output: URL) throws {
    guard InputLevelReport.validOwner(ownerId), revision > 0,
      revision <= 9_007_199_254_740_991,
      microphone.sourceEpoch <= 9_007_199_254_740_991,
      microphone.interruptions <= 9_007_199_254_740_991
    else { throw ProbeError.capture("input status report owner or counters are invalid") }
    let data = try JSONEncoder().encode(self)
    guard data.count <= 16 * 1024 else {
      throw ProbeError.capture("input status report exceeds limit")
    }
    try data.write(to: output.appendingPathComponent("input-status.json"), options: .atomic)
  }
}
