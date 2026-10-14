import Foundation

/// This is selected-device metadata, separate from binding, received PCM and
/// saved-track proof. A target UID is not a claim that audio is connected.
struct MicrophoneSelectionStatus: Encodable, Equatable {
  let schemaVersion = 1
  let ownerId: String
  let revision: Int64
  let requestRevision: Int64
  let mode: String
  let deviceUID: String
  let source: String
  let fallbackReason: String?

  func publish(to output: URL) throws {
    guard InputLevelReport.validOwner(ownerId),
      (1...9_007_199_254_740_991).contains(revision),
      (0...9_007_199_254_740_991).contains(requestRevision),
      InputDevices.validUID(deviceUID),
      (mode == "fixed" && (source == "fixed" || source == "pending")
        && fallbackReason == nil)
        || (mode == "automatic" && source == "application" && fallbackReason == nil)
        || (mode == "automatic" && source == "systemDefault"
          && fallbackReason.map {
            [
              "application_input_unavailable", "application_input_ambiguous",
              "mon_direct_recording",
            ]
            .contains($0)
          } == true)
        || (mode == "automatic" && source == "pending" && fallbackReason == nil)
    else { throw ProbeError.capture("microphone selection status is invalid") }
    let data = try JSONEncoder().encode(self)
    guard data.count <= 4096 else {
      throw ProbeError.capture("microphone selection status exceeds limit")
    }
    try data.write(
      to: output.appendingPathComponent("microphone-selection.status.json"), options: .atomic)
  }
}
