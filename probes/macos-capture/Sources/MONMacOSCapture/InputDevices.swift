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
  enum AutomaticSource: Equatable {
    case application
    case systemDefault(reason: String)
  }

  struct AutomaticChoice {
    let device: CaptureInputDevice
    let source: AutomaticSource
  }

  static func validUID(_ uid: String) -> Bool {
    CaptureCore.validUID(uid)
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
      devices: try CaptureCore.catalogOrder(devices, applications: false).map { devices[$0] })
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

  /// Read-only HAL metadata. This does not open any input stream. A process
  /// object may not exist yet, and meeting apps can move input into a helper
  /// process; callers must treat nil as unknown, not as proof of no microphone.
  static func activeInputObjects(for pid: pid_t) throws -> [AudioObjectID]? {
    guard pid > 0 else { throw ProbeError.usage("input process ID is invalid") }
    let system = AudioObjectID(kAudioObjectSystemObject)
    var processID = pid
    var processObject = AudioObjectID(kAudioObjectUnknown)
    var size = UInt32(MemoryLayout<AudioObjectID>.stride)
    var property = address(kAudioHardwarePropertyTranslatePIDToProcessObject)
    let result = withUnsafePointer(to: &processID) { pointer in
      AudioObjectGetPropertyData(
        system, &property, UInt32(MemoryLayout<pid_t>.stride), pointer,
        &size, &processObject)
    }
    try check(result)
    guard size == MemoryLayout<AudioObjectID>.stride else {
      throw ProbeError.capture("input process metadata has an invalid size")
    }
    if processObject == kAudioObjectUnknown { return nil }
    let running: UInt32 = try scalar(processObject, kAudioProcessPropertyIsRunningInput)
    guard running != 0 else { return nil }
    return try objects(
      processObject, kAudioProcessPropertyDevices, scope: kAudioObjectPropertyScopeInput)
  }

  /// Only one known, live device can override the system default. Empty,
  /// duplicated or unrecognised process devices are deliberately inconclusive.
  static func automaticChoice(
    from devices: [CaptureInputDevice], processInputObjects: [AudioObjectID]?
  ) throws -> AutomaticChoice {
    if let processInputObjects {
      let matching = devices.filter { processInputObjects.contains($0.objectID) }
      if processInputObjects.count == 1, matching.count == 1 {
        return AutomaticChoice(device: matching[0], source: .application)
      }
    }
    let fallback = try choose(nil, from: devices)
    let reason =
      processInputObjects == nil
      ? "application_input_unavailable"
      : "application_input_ambiguous"
    return AutomaticChoice(device: fallback, source: .systemDefault(reason: reason))
  }

  static func automaticChoice(
    from devices: [CaptureInputDevice], application: CaptureApplicationIdentity?,
    processFamily: CaptureProcessFamily? = nil
  ) throws -> AutomaticChoice {
    guard let application else {
      return AutomaticChoice(
        device: try choose(nil, from: devices),
        source: .systemDefault(reason: "mon_direct_recording"))
    }
    // Both checks prevent a PID reused during the HAL query from being
    // mistaken for the originally selected meeting application.
    guard (try? CaptureApplications.current(application)) != nil else {
      return try automaticChoice(from: devices, processInputObjects: nil)
    }
    let objects: [AudioObjectID]?
    do {
      let family = processFamily ?? CaptureProcessFamily(root: application)
      let members = try family.snapshot()
      objects = try applicationInputObjects(pids: Array(members.keys), read: activeInputObjects)
      try family.verify(members)
    } catch { return try automaticChoice(from: devices, processInputObjects: nil) }
    guard (try? CaptureApplications.current(application)) != nil else {
      return try automaticChoice(from: devices, processInputObjects: nil)
    }
    return try automaticChoice(from: devices, processInputObjects: objects)
  }

  static func applicationInputObjects(
    pids: [pid_t], read: (pid_t) throws -> [AudioObjectID]?
  ) throws -> [AudioObjectID]? {
    var inputs = Set<AudioObjectID>()
    for pid in pids {
      if let objects = try read(pid) {
        guard !objects.contains(kAudioObjectUnknown) else {
          throw ProbeError.capture("application input device identity is unavailable")
        }
        inputs.formUnion(objects)
      }
    }
    return inputs.isEmpty ? nil : inputs.sorted()
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
