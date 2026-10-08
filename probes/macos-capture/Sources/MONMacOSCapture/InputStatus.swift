import Foundation

// A heartbeat of the actual binding policy, not evidence of received or saved PCM.
// Only ordinary recording publishes this leaf; private five-second tests keep
// their existing fixed-file ownership and cleanup contract.
struct InputStatusReport: Encodable {
  let schemaVersion = 2
  let ownerId: String
  let revision: Int64
  let microphone: MicrophoneRecovery.Status
  var system: SystemAudioRecovery.Status? = nil

  enum CodingKeys: String, CodingKey { case schemaVersion, ownerId, revision, microphone, system }
  func encode(to encoder: Encoder) throws {
    var container = encoder.container(keyedBy: CodingKeys.self)
    try container.encode(schemaVersion, forKey: .schemaVersion)
    try container.encode(ownerId, forKey: .ownerId)
    try container.encode(revision, forKey: .revision)
    try container.encode(microphone, forKey: .microphone)
    if let system {
      try container.encode(system, forKey: .system)
    } else {
      try container.encodeNil(forKey: .system)
    }
  }

  func publish(to output: URL) throws {
    guard InputLevelReport.validOwner(ownerId), revision > 0,
      revision <= 9_007_199_254_740_991,
      microphone.sourceEpoch <= 9_007_199_254_740_991,
      microphone.interruptions <= 9_007_199_254_740_991,
      system.map({
        $0.sourceEpoch <= 9_007_199_254_740_991 && $0.interruptions <= 9_007_199_254_740_991
      }) ?? true
    else { throw ProbeError.capture("input status report owner or counters are invalid") }
    let data = try JSONEncoder().encode(self)
    guard data.count <= 16 * 1024 else {
      throw ProbeError.capture("input status report exceeds limit")
    }
    try data.write(to: output.appendingPathComponent("input-status.json"), options: .atomic)
  }
}
