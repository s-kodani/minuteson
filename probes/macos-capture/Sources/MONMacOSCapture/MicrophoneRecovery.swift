import CoreAudio
import Foundation

/// One frozen UID, not a name or the current default input. A binding token
/// belongs to one engine incarnation; its queued callbacks cannot be adopted
/// by a later engine even when Core Audio reuses a transient object ID.
struct MicrophoneRecovery {
  struct Binding: Equatable, Sendable {
    let uid: String
    let objectID: AudioObjectID
    let epoch: UInt64
  }

  struct Status: Encodable, Equatable {
    let state: String
    let sourceEpoch: UInt64
    let interruptions: UInt64
    let reason: String?

    enum CodingKeys: String, CodingKey { case state, sourceEpoch, interruptions, reason }
    func encode(to encoder: Encoder) throws {
      var container = encoder.container(keyedBy: CodingKeys.self)
      try container.encode(state, forKey: .state)
      try container.encode(sourceEpoch, forKey: .sourceEpoch)
      try container.encode(interruptions, forKey: .interruptions)
      if let reason {
        try container.encode(reason, forKey: .reason)
      } else {
        try container.encodeNil(forKey: .reason)
      }
    }
  }

  let uid: String
  private var epoch: UInt64 = 0
  private var interruptions: UInt64 = 0
  private var state = "reconnecting"
  private var reason: String?
  private var binding: Binding?
  private var interrupted = false
  private var stopped = false

  init(uid: String) throws {
    guard InputDevices.validUID(uid) else { throw ProbeError.capture("recovery UID is invalid") }
    self.uid = uid
  }

  var status: Status {
    Status(state: state, sourceEpoch: epoch, interruptions: interruptions, reason: reason)
  }

  func device(from devices: [CaptureInputDevice]) throws -> CaptureInputDevice {
    guard !stopped else { throw ProbeError.capture("microphone recovery is stopped") }
    let device = try InputDevices.choose(uid, from: devices)
    guard device.objectID != kAudioObjectUnknown else {
      throw ProbeError.capture("recovery input has no Core Audio object")
    }
    return device
  }

  mutating func begin(_ device: CaptureInputDevice) throws -> Binding {
    guard !stopped, binding == nil, device.uid == uid, device.objectID != kAudioObjectUnknown,
      epoch < 9_007_199_254_740_991
    else { throw ProbeError.capture("microphone recovery binding is invalid") }
    epoch += 1
    let token = Binding(uid: uid, objectID: device.objectID, epoch: epoch)
    binding = token
    state = "reconnecting"
    return token
  }

  mutating func activate(_ token: Binding) throws {
    guard !stopped, binding == token, state == "reconnecting" else {
      throw ProbeError.capture("microphone recovery result belongs to an obsolete input")
    }
    state = "connected"
    reason = nil
    interrupted = false
  }

  func accepts(_ token: Binding) -> Bool {
    !stopped && state == "connected" && binding == token
  }

  func owns(_ token: Binding) -> Bool { !stopped && binding == token }

  mutating func interrupt(_ reason: String, unavailable: Bool = false) {
    guard !stopped else { return }
    if !interrupted {
      interruptions += 1
      interrupted = true
    }
    state = unavailable ? "unavailable" : "disconnected"
    self.reason = reason
    binding = nil
  }

  mutating func stop() {
    stopped = true
    binding = nil
    state = "stopped"
  }
}
