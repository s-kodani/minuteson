import Foundation

// A heartbeat of the actual binding policy, not evidence of received or saved PCM.
// Only ordinary recording publishes this leaf; private five-second tests keep
// their existing fixed-file ownership and cleanup contract.
struct InputStatusReport: Encodable, Sendable {
  static func validStatus(_ status: MicrophoneRecovery.Status, input: String) -> Bool {
    let reasons =
      input == "microphone"
      ? [
        "identity_changed", "engine_changed", "device_disconnected", "identity_ambiguous",
        "metadata_unavailable", "permission_unavailable", "restart_failed", "format_changed",
      ]
      : [
        "stream_stopped", "application_unavailable", "acquisition_timeout",
        "permission_unavailable", "restart_failed", "format_changed",
      ]
    return ["microphone", "system"].contains(input)
      && ["connected", "disconnected", "unavailable", "reconnecting", "stopped"].contains(
        status.state)
      && status.sourceEpoch <= InputEventHistory.safe
      && status.interruptions <= InputEventHistory.safe
      && (status.reason.map { reasons.contains($0) } ?? true)
      && (status.state != "connected"
        || (status.sourceEpoch > status.interruptions && status.reason == nil))
      && (!["disconnected", "unavailable"].contains(status.state)
        || (status.interruptions > 0 && status.reason != nil))
  }

  var schemaVersion: Int { microphoneHistory == nil ? 2 : 3 }
  let ownerId: String
  let revision: Int64
  let microphone: MicrophoneRecovery.Status
  var system: SystemAudioRecovery.Status? = nil
  var microphoneHistory: InputEventHistory? = nil
  var systemHistory: InputEventHistory? = nil

  enum CodingKeys: String, CodingKey {
    case schemaVersion, ownerId, revision, microphone, system, microphoneHistory, systemHistory
  }
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
    if let microphoneHistory {
      try container.encode(microphoneHistory, forKey: .microphoneHistory)
      if let systemHistory {
        try container.encode(systemHistory, forKey: .systemHistory)
      } else {
        try container.encodeNil(forKey: .systemHistory)
      }
    }
  }

  func encodedData() throws -> Data {
    guard InputLevelReport.validOwner(ownerId), revision > 0,
      revision <= 9_007_199_254_740_991,
      microphone.sourceEpoch <= 9_007_199_254_740_991,
      microphone.interruptions <= 9_007_199_254_740_991,
      system.map({
        $0.sourceEpoch <= 9_007_199_254_740_991 && $0.interruptions <= 9_007_199_254_740_991
      }) ?? true
    else { throw ProbeError.capture("input status report owner or counters are invalid") }
    if let microphoneHistory {
      guard microphoneHistory.matches(microphone),
        (system == nil) == (systemHistory == nil),
        system.map({ systemHistory?.matches($0, input: "system") == true }) ?? true
      else { throw ProbeError.capture("input status event history is inconsistent") }
    } else if systemHistory != nil {
      throw ProbeError.capture("input status event history requires both supported inputs")
    }
    let data = try JSONEncoder().encode(self)
    guard data.count <= (schemaVersion == 2 ? 16 : 64) * 1024 else {
      throw ProbeError.capture("input status report exceeds limit")
    }
    return data
  }

  func publish(to output: URL) throws {
    let data = try encodedData()
    try data.write(to: output.appendingPathComponent("input-status.json"), options: .atomic)
  }
}
