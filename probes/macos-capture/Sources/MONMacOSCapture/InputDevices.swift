import AVFoundation
import AudioToolbox
import CoreAudio
import Foundation

struct CaptureInputDevice: Encodable, Equatable {
  let uid: String
  let name: String
  let isDefault: Bool
  let objectID: AudioObjectID

  enum CodingKeys: String, CodingKey { case uid, name, isDefault }
}

struct CaptureInputDevices: Encodable {
  let schemaVersion = 1
  let microphonePermission: String
  let devices: [CaptureInputDevice]
}

enum InputDevices {
  static func validUID(_ uid: String) -> Bool {
    !uid.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
      && uid.utf8.count <= 1024
      && !uid.unicodeScalars.contains(where: { $0.value < 32 || (127...159).contains($0.value) })
  }

  // Metadata only: no engine, input tap, permission request or audio file.
  static func live() throws -> CaptureInputDevices {
    let system = AudioObjectID(kAudioObjectSystemObject)
    let defaultID: AudioObjectID = try scalar(system, kAudioHardwarePropertyDefaultInputDevice)
    let ids = try objects(system, kAudioHardwarePropertyDevices)
    guard ids.count <= 128 else { throw ProbeError.capture("too many input devices") }
    var devices: [CaptureInputDevice] = []
    for id in ids {
      let alive: UInt32 = try scalar(id, kAudioDevicePropertyDeviceIsAlive)
      if alive == 0 { continue }
      if try objects(id, kAudioDevicePropertyStreams, scope: kAudioObjectPropertyScopeInput).isEmpty
      {
        continue
      }
      let uid = try string(id, kAudioDevicePropertyDeviceUID)
      let name = try string(id, kAudioObjectPropertyName)
      guard validUID(uid), !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
        name.utf8.count <= 1024
      else { throw ProbeError.capture("input device identity is invalid") }
      devices.append(
        CaptureInputDevice(uid: uid, name: name, isDefault: id == defaultID, objectID: id))
    }
    guard Set(devices.map(\.uid)).count == devices.count else {
      throw ProbeError.capture("input device identities are duplicated")
    }
    let permission: String
    switch AVCaptureDevice.authorizationStatus(for: .audio) {
    case .authorized: permission = "authorized"
    case .denied: permission = "denied"
    case .restricted: permission = "restricted"
    case .notDetermined: permission = "notDetermined"
    @unknown default: permission = "unknown"
    }
    return CaptureInputDevices(
      microphonePermission: permission,
      devices: devices.sorted {
        $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending
      })
  }

  static func choose(_ uid: String?, from devices: [CaptureInputDevice]) throws
    -> CaptureInputDevice
  {
    if let uid {
      guard validUID(uid) else { throw ProbeError.usage("microphone UID is invalid") }
      let matches = devices.filter { $0.uid == uid }
      guard matches.count == 1 else {
        throw ProbeError.capture(
          "選択したマイクが見つかりません。機器の接続を確認して選び直してください。別のマイクへの自動切替はしていません。")
      }
      return matches[0]
    }
    let matches = devices.filter(\.isDefault)
    guard matches.count == 1 else { throw ProbeError.capture("default microphone is unavailable") }
    return matches[0]
  }

  static func verifyDevice(
    _ unit: AudioUnit, expected: AudioObjectID, expectedUID: String? = nil
  ) throws {
    var actual: AudioObjectID = 0
    var size = UInt32(MemoryLayout<AudioObjectID>.stride)
    try check(
      AudioUnitGetProperty(
        unit, kAudioOutputUnitProperty_CurrentDevice,
        kAudioUnitScope_Global, 0, &actual, &size))
    guard size == MemoryLayout<AudioObjectID>.stride, actual == expected else {
      throw ProbeError.capture(
        "microphone changed during recording; other device audio was not accepted")
    }
    if let expectedUID {
      guard validUID(expectedUID), try string(actual, kAudioDevicePropertyDeviceUID) == expectedUID
      else { throw ProbeError.capture("microphone object no longer has the confirmed UID") }
    }
  }

  private static func check(_ status: OSStatus) throws {
    guard status == noErr else {
      throw ProbeError.capture("input device metadata failed: \(status)")
    }
  }

  private static func address(
    _ selector: AudioObjectPropertySelector,
    scope: AudioObjectPropertyScope = kAudioObjectPropertyScopeGlobal
  ) -> AudioObjectPropertyAddress {
    AudioObjectPropertyAddress(
      mSelector: selector, mScope: scope, mElement: kAudioObjectPropertyElementMain)
  }

  private static func objects(
    _ id: AudioObjectID, _ selector: AudioObjectPropertySelector,
    scope: AudioObjectPropertyScope = kAudioObjectPropertyScopeGlobal
  ) throws -> [AudioObjectID] {
    var property = address(selector, scope: scope)
    var size: UInt32 = 0
    try check(AudioObjectGetPropertyDataSize(id, &property, 0, nil, &size))
    let stride = UInt32(MemoryLayout<AudioObjectID>.stride)
    guard size % stride == 0, size <= 128 * stride else {
      throw ProbeError.capture("input device list exceeds its capacity")
    }
    if size == 0 { return [] }
    var values = [AudioObjectID](repeating: 0, count: Int(size / stride))
    let capacity = size
    try values.withUnsafeMutableBufferPointer { buffer in
      try check(AudioObjectGetPropertyData(id, &property, 0, nil, &size, buffer.baseAddress!))
    }
    guard size <= capacity, size % stride == 0 else {
      throw ProbeError.capture("input device list changed during reading")
    }
    return Array(values.prefix(Int(size / stride)))
  }

  private static func scalar<T>(_ id: AudioObjectID, _ selector: AudioObjectPropertySelector) throws
    -> T
  {
    var property = address(selector)
    let value = UnsafeMutablePointer<T>.allocate(capacity: 1)
    defer { value.deallocate() }
    var size = UInt32(MemoryLayout<T>.stride)
    try check(AudioObjectGetPropertyData(id, &property, 0, nil, &size, value))
    guard size == MemoryLayout<T>.stride else {
      throw ProbeError.capture("invalid input metadata size")
    }
    return value.move()
  }

  private static func string(_ id: AudioObjectID, _ selector: AudioObjectPropertySelector) throws
    -> String
  {
    var property = address(selector)
    var value: CFString = "" as CFString
    var size = UInt32(MemoryLayout<CFString>.stride)
    try withUnsafeMutablePointer(to: &value) { pointer in
      try check(AudioObjectGetPropertyData(id, &property, 0, nil, &size, pointer))
    }
    guard size == MemoryLayout<CFString>.stride else {
      throw ProbeError.capture("invalid input name size")
    }
    return value as String
  }
}
