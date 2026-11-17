import AVFoundation
import AudioToolbox
import CoreAudio
import Foundation

struct MicrophoneDeviceProofPending: Error {}

/// The native boundary is injectable so ownership and actual AUHAL property
/// ordering can be exercised without opening a microphone in automated tests.
protocol MicrophoneAudioUnitAPI: AnyObject, Sendable {
  func create() throws -> AudioUnit
  func set(
    _ unit: AudioUnit, _ property: AudioUnitPropertyID, _ scope: AudioUnitScope,
    _ element: AudioUnitElement, _ value: UnsafeRawPointer, _ bytes: UInt32) throws
  func uint32(
    _ unit: AudioUnit, _ property: AudioUnitPropertyID, _ scope: AudioUnitScope,
    _ element: AudioUnitElement
  ) throws -> UInt32
  func hardwareFormat(_ unit: AudioUnit) throws -> AudioStreamBasicDescription
  func deviceUID(_ device: AudioObjectID) throws -> String
  func deviceAlive(_ device: AudioObjectID) throws -> Bool
  func addUnitListener(
    _ unit: AudioUnit, _ property: AudioUnitPropertyID,
    _ callback: AudioUnitPropertyListenerProc, _ context: UnsafeMutableRawPointer) throws
  func removeUnitListener(
    _ unit: AudioUnit, _ property: AudioUnitPropertyID,
    _ callback: AudioUnitPropertyListenerProc, _ context: UnsafeMutableRawPointer) throws
  func addDeviceListener(
    _ device: AudioObjectID, _ address: AudioObjectPropertyAddress,
    _ callback: AudioObjectPropertyListenerProc, _ context: UnsafeMutableRawPointer) throws
  func removeDeviceListener(
    _ device: AudioObjectID, _ address: AudioObjectPropertyAddress,
    _ callback: AudioObjectPropertyListenerProc, _ context: UnsafeMutableRawPointer) throws
  func initialize(_ unit: AudioUnit) throws
  func start(_ unit: AudioUnit) throws
  func stop(_ unit: AudioUnit) throws
  func uninitialize(_ unit: AudioUnit) throws
  func dispose(_ unit: AudioUnit) throws
  func render(
    _ unit: AudioUnit, _ flags: UnsafeMutablePointer<AudioUnitRenderActionFlags>,
    _ timestamp: UnsafePointer<AudioTimeStamp>, _ frames: UInt32,
    _ data: UnsafeMutablePointer<AudioBufferList>
  ) -> OSStatus
}

final class NativeMicrophoneAudioUnitAPI: MicrophoneAudioUnitAPI, @unchecked Sendable {
  private func check(_ status: OSStatus, _ operation: String) throws {
    guard status == noErr else {
      throw ProbeError.capture("microphone AUHAL \(operation) failed: \(status)")
    }
  }
  func create() throws -> AudioUnit {
    var description = AudioComponentDescription(
      componentType: kAudioUnitType_Output,
      componentSubType: kAudioUnitSubType_HALOutput,
      componentManufacturer: kAudioUnitManufacturer_Apple,
      componentFlags: 0, componentFlagsMask: 0)
    guard let component = AudioComponentFindNext(nil, &description) else {
      throw ProbeError.capture("microphone AUHAL component is unavailable")
    }
    var unit: AudioUnit?
    try check(AudioComponentInstanceNew(component, &unit), "create")
    guard let unit else { throw ProbeError.capture("microphone AUHAL instance is unavailable") }
    return unit
  }
  func set(
    _ unit: AudioUnit, _ property: AudioUnitPropertyID, _ scope: AudioUnitScope,
    _ element: AudioUnitElement, _ value: UnsafeRawPointer, _ bytes: UInt32
  ) throws {
    try check(
      AudioUnitSetProperty(unit, property, scope, element, value, bytes), "set property \(property)"
    )
  }
  func uint32(
    _ unit: AudioUnit, _ property: AudioUnitPropertyID, _ scope: AudioUnitScope,
    _ element: AudioUnitElement
  ) throws -> UInt32 {
    var value: UInt32 = 0
    var size = UInt32(MemoryLayout<UInt32>.size)
    try check(
      AudioUnitGetProperty(unit, property, scope, element, &value, &size),
      "read property \(property)")
    guard size == MemoryLayout<UInt32>.size else {
      throw ProbeError.capture("microphone AUHAL property size changed")
    }
    return value
  }
  func hardwareFormat(_ unit: AudioUnit) throws -> AudioStreamBasicDescription {
    var value = AudioStreamBasicDescription()
    var size = UInt32(MemoryLayout<AudioStreamBasicDescription>.size)
    try check(
      AudioUnitGetProperty(
        unit, kAudioUnitProperty_StreamFormat, kAudioUnitScope_Input, 1,
        &value, &size), "read hardware format")
    guard size == MemoryLayout<AudioStreamBasicDescription>.size else {
      throw ProbeError.capture("microphone AUHAL hardware format size changed")
    }
    return value
  }
  func deviceUID(_ device: AudioObjectID) throws -> String {
    var address = AudioObjectPropertyAddress(
      mSelector: kAudioDevicePropertyDeviceUID,
      mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain)
    var value: CFString = "" as CFString
    var size = UInt32(MemoryLayout<CFString>.size)
    try withUnsafeMutablePointer(to: &value) { pointer in
      try check(
        AudioObjectGetPropertyData(device, &address, 0, nil, &size, pointer), "read device UID")
    }
    guard size == MemoryLayout<CFString>.size else {
      throw ProbeError.capture("microphone UID size changed")
    }
    return value as String
  }
  func deviceAlive(_ device: AudioObjectID) throws -> Bool {
    var address = AudioObjectPropertyAddress(
      mSelector: kAudioDevicePropertyDeviceIsAlive,
      mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain)
    var value: UInt32 = 0
    var size = UInt32(MemoryLayout<UInt32>.size)
    try check(
      AudioObjectGetPropertyData(device, &address, 0, nil, &size, &value),
      "read device availability")
    guard size == MemoryLayout<UInt32>.size else {
      throw ProbeError.capture("microphone availability size changed")
    }
    return value != 0
  }
  func addUnitListener(
    _ unit: AudioUnit, _ property: AudioUnitPropertyID,
    _ callback: AudioUnitPropertyListenerProc, _ context: UnsafeMutableRawPointer
  ) throws {
    try check(AudioUnitAddPropertyListener(unit, property, callback, context), "observe unit")
  }
  func removeUnitListener(
    _ unit: AudioUnit, _ property: AudioUnitPropertyID,
    _ callback: AudioUnitPropertyListenerProc, _ context: UnsafeMutableRawPointer
  ) throws {
    try check(
      AudioUnitRemovePropertyListenerWithUserData(unit, property, callback, context),
      "remove unit observation")
  }
  func addDeviceListener(
    _ device: AudioObjectID, _ address: AudioObjectPropertyAddress,
    _ callback: AudioObjectPropertyListenerProc, _ context: UnsafeMutableRawPointer
  ) throws {
    var address = address
    try check(AudioObjectAddPropertyListener(device, &address, callback, context), "observe device")
  }
  func removeDeviceListener(
    _ device: AudioObjectID, _ address: AudioObjectPropertyAddress,
    _ callback: AudioObjectPropertyListenerProc, _ context: UnsafeMutableRawPointer
  ) throws {
    var address = address
    try check(
      AudioObjectRemovePropertyListener(device, &address, callback, context),
      "remove device observation")
  }
  func initialize(_ unit: AudioUnit) throws { try check(AudioUnitInitialize(unit), "initialize") }
  func start(_ unit: AudioUnit) throws { try check(AudioOutputUnitStart(unit), "start") }
  func stop(_ unit: AudioUnit) throws { try check(AudioOutputUnitStop(unit), "stop") }
  func uninitialize(_ unit: AudioUnit) throws {
    try check(AudioUnitUninitialize(unit), "uninitialize")
  }
  func dispose(_ unit: AudioUnit) throws {
    try check(AudioComponentInstanceDispose(unit), "dispose")
  }
  func render(
    _ unit: AudioUnit, _ flags: UnsafeMutablePointer<AudioUnitRenderActionFlags>,
    _ timestamp: UnsafePointer<AudioTimeStamp>, _ frames: UInt32,
    _ data: UnsafeMutablePointer<AudioBufferList>
  ) -> OSStatus {
    AudioUnitRender(unit, flags, timestamp, 1, frames, data)
  }
}

/// Opaque tokens are never reused. A native listener that was already queued
/// at unregister can only perform a strong lookup, never dereference a freed
/// Swift object or accidentally target a later unit with a recycled address.
private final class MicrophoneAudioUnitRegistry: @unchecked Sendable {
  static let shared = MicrophoneAudioUnitRegistry()
  private final class WeakContext {
    weak var value: MicrophoneAudioUnitContext?
    init(_ value: MicrophoneAudioUnitContext) { self.value = value }
  }
  private let lock = NSLock()
  private var next: UInt = 1
  private var active: [UInt: WeakContext] = [:]
  func register(_ context: MicrophoneAudioUnitContext) throws -> UnsafeMutableRawPointer {
    try lock.withLock {
      guard next < UInt.max, let token = UnsafeMutableRawPointer(bitPattern: next) else {
        throw ProbeError.capture("microphone AUHAL callback token capacity exhausted")
      }
      active[next] = WeakContext(context)
      next += 1
      return token
    }
  }
  func resolve(_ token: UnsafeMutableRawPointer) -> MicrophoneAudioUnitContext? {
    lock.withLock { active[UInt(bitPattern: token)]?.value }
  }
  func unregister(_ token: UnsafeMutableRawPointer) {
    _ = lock.withLock { active.removeValue(forKey: UInt(bitPattern: token)) }
  }
}
private let microphoneUnitChanged: AudioUnitPropertyListenerProc = { token, _, _, _, _ in
  MicrophoneAudioUnitRegistry.shared.resolve(token)?.changed()
}
private let microphoneDeviceChanged: AudioObjectPropertyListenerProc = { _, _, _, token in
  guard let token else { return noErr }
  MicrophoneAudioUnitRegistry.shared.resolve(token)?.changed()
  return noErr
}
private let microphoneInputCallback: AURenderCallback = { token, flags, timestamp, _, frames, _ in
  MicrophoneAudioUnitRegistry.shared.resolve(token)?
    .receive(flags: flags, timestamp: timestamp, frames: frames) ?? noErr
}

/// RefCon, PCM storage and the unit share one lifetime. Controls run on the
/// engine's serial owner; render callbacks never synchronously enter that owner.
final class MicrophoneAudioUnitContext: @unchecked Sendable {
  let api: any MicrophoneAudioUnitAPI
  let unit: AudioUnit
  let binding: MicrophoneRecovery.Binding
  private let condition = NSCondition()
  private var observer: (@Sendable () -> Void)?
  private var tap: (@Sendable (AVAudioPCMBuffer, AVAudioTime) -> Void)?
  private var revision: UInt64 = 0
  private var provenRevision: UInt64?
  private var terminalFailure: String?
  private var inFlight = 0
  private var rendering = false
  private var accepting = false
  private var started = false
  private var closing = false
  private var closed = false
  private var buffer: AVAudioPCMBuffer?
  private var storagePointers: [UnsafeMutableRawPointer] = []
  private(set) var format: AVAudioFormat
  // Owner-queue-only native stages. Failed stop/disposal retains the native
  // self-reference and every allocation until a later proven closure succeeds.
  private var retained = false
  private var token: UnsafeMutableRawPointer?
  private var initialized = false
  private var startAttempted = false
  private var callbackInstalled = false
  private var unitListeners: [AudioUnitPropertyID] = []
  private var deviceListeners: [AudioObjectPropertyAddress] = []

  init(api: any MicrophoneAudioUnitAPI, unit: AudioUnit, binding: MicrophoneRecovery.Binding) {
    self.api = api
    self.unit = unit
    self.binding = binding
    format = AVAudioFormat(standardFormatWithSampleRate: 48000, channels: 1)!
  }
  private var refcon: UnsafeMutableRawPointer {
    guard let token else { preconditionFailure("microphone native token is not registered") }
    return token
  }
  private func locked<T>(_ body: () throws -> T) rethrows -> T {
    condition.lock()
    defer { condition.unlock() }
    return try body()
  }
  private func set<T>(
    _ property: AudioUnitPropertyID, _ scope: AudioUnitScope,
    _ element: AudioUnitElement, _ value: T
  ) throws {
    var value = value
    try withUnsafePointer(to: &value) {
      try api.set(unit, property, scope, element, $0, UInt32(MemoryLayout<T>.size))
    }
  }
  private func nativeFormat() throws -> AVAudioFormat {
    let hardware = try api.hardwareFormat(unit)
    guard hardware.mFormatID == kAudioFormatLinearPCM,
      hardware.mSampleRate.isFinite, hardware.mSampleRate.rounded() == hardware.mSampleRate,
      (8000...384000).contains(hardware.mSampleRate), (1...8).contains(hardware.mChannelsPerFrame),
      let format = AVAudioFormat(
        standardFormatWithSampleRate: hardware.mSampleRate,
        channels: hardware.mChannelsPerFrame)
    else {
      throw ProbeError.capture("microphone AUHAL hardware format is unusable")
    }
    return format
  }
  func select() throws {
    token = try MicrophoneAudioUnitRegistry.shared.register(self)
    _ = Unmanaged.passRetained(self)
    retained = true
    // AUHAL requires its IO direction before CurrentDevice is set. In
    // particular, never create an output stream or a default-device aggregate.
    try set(kAudioOutputUnitProperty_EnableIO, kAudioUnitScope_Input, 1, UInt32(1))
    try set(kAudioOutputUnitProperty_EnableIO, kAudioUnitScope_Output, 0, UInt32(0))
    let current = try api.uint32(
      unit, kAudioOutputUnitProperty_CurrentDevice, kAudioUnitScope_Global, 0)
    if current != binding.objectID {
      try set(kAudioOutputUnitProperty_CurrentDevice, kAudioUnitScope_Global, 0, binding.objectID)
    }
    format = try nativeFormat()
    for property in [
      kAudioOutputUnitProperty_CurrentDevice, kAudioUnitProperty_StreamFormat,
      kAudioOutputUnitProperty_IsRunning, kAudioUnitProperty_MaximumFramesPerSlice,
    ] {
      try api.addUnitListener(unit, property, microphoneUnitChanged, refcon)
      unitListeners.append(property)
    }
    for (selector, scope) in [
      (kAudioDevicePropertyDeviceUID, kAudioObjectPropertyScopeGlobal),
      (kAudioDevicePropertyDeviceIsAlive, kAudioObjectPropertyScopeGlobal),
      (kAudioDevicePropertyNominalSampleRate, kAudioObjectPropertyScopeGlobal),
      (kAudioDevicePropertyStreams, kAudioObjectPropertyScopeInput),
      (kAudioDevicePropertyBufferFrameSize, kAudioObjectPropertyScopeGlobal),
    ] {
      let address = AudioObjectPropertyAddress(
        mSelector: selector, mScope: scope,
        mElement: kAudioObjectPropertyElementMain)
      try api.addDeviceListener(binding.objectID, address, microphoneDeviceChanged, refcon)
      deviceListeners.append(address)
    }
    try proveOrAwaitRevision()
  }
  private func proveOrAwaitRevision() throws {
    do { try verify(binding) } catch is MicrophoneDeviceProofPending {
      // Real UID/format/alive reads passed. A racing metadata notification
      // keeps admission fenced until the control owner proves its next revision.
    }
  }
  func currentFormat() -> AVAudioFormat {
    do { return try nativeFormat() } catch {
      fail(String(describing: error))
      return locked { format }
    }
  }
  func observe(_ handler: (@Sendable () -> Void)?) { locked { observer = handler } }
  func install(
    format: AVAudioFormat,
    handler: @escaping @Sendable (AVAudioPCMBuffer, AVAudioTime) -> Void
  ) throws {
    try proveOrAwaitRevision()
    guard MicrophonePCMFormat.matches(self.format, format) else {
      throw CaptureInputFault.invalidFormat
    }
    let nativeMaximum = try api.uint32(
      unit, kAudioUnitProperty_MaximumFramesPerSlice, kAudioUnitScope_Global, 0)
    guard nativeMaximum > 0, nativeMaximum <= 96_000 else {
      throw ProbeError.capture("microphone AUHAL slice exceeds bounded PCM capacity")
    }
    let maximum = max(UInt32(4096), max(nativeMaximum, UInt32(format.sampleRate / 4)))
    guard let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: maximum) else {
      throw ProbeError.capture("microphone AUHAL PCM allocation failed")
    }
    let description = format.streamDescription.pointee
    try set(kAudioUnitProperty_StreamFormat, kAudioUnitScope_Output, 1, description)
    try set(kAudioUnitProperty_MaximumFramesPerSlice, kAudioUnitScope_Global, 0, maximum)
    let actualMaximum = try api.uint32(
      unit, kAudioUnitProperty_MaximumFramesPerSlice,
      kAudioUnitScope_Global, 0)
    guard actualMaximum > 0, actualMaximum <= maximum else {
      throw ProbeError.capture("microphone AUHAL slice exceeds bounded PCM capacity")
    }
    let storagePointers = UnsafeMutableAudioBufferListPointer(buffer.mutableAudioBufferList).map {
      $0.mData!
    }
    locked {
      self.buffer = buffer
      self.storagePointers = storagePointers
      tap = handler
      accepting = true
    }
    try set(
      kAudioOutputUnitProperty_SetInputCallback, kAudioUnitScope_Global, 0,
      AURenderCallbackStruct(inputProc: microphoneInputCallback, inputProcRefCon: refcon))
    callbackInstalled = true
    try proveOrAwaitRevision()
  }
  func removeTap() {
    locked {
      accepting = false
      tap = nil
    }
  }
  func prepare() throws {
    guard !locked({ closing || closed }) else {
      throw ProbeError.capture("microphone AUHAL preparation is retired")
    }
    guard !initialized else { return }
    try api.initialize(unit)
    initialized = true
    let maximum = try api.uint32(
      unit, kAudioUnitProperty_MaximumFramesPerSlice, kAudioUnitScope_Global, 0)
    guard maximum > 0, maximum <= 96_000 else {
      throw ProbeError.capture("microphone AUHAL initialized slice exceeds bounded PCM capacity")
    }
    if let current = locked({ buffer }), maximum > current.frameCapacity {
      guard !startAttempted, !locked({ started || rendering }) else {
        throw ProbeError.capture("microphone AUHAL active slice exceeds PCM capacity")
      }
      guard let enlarged = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: maximum) else {
        throw ProbeError.capture("microphone AUHAL PCM allocation failed")
      }
      let pointers = UnsafeMutableAudioBufferListPointer(enlarged.mutableAudioBufferList).map {
        $0.mData!
      }
      // Initialize has not started IO. Replace allocation-owned storage before
      // any callback can borrow it, including on devices with a larger slice.
      locked {
        buffer = enlarged
        storagePointers = pointers
      }
    }
  }
  func start() throws {
    try proveOrAwaitRevision()
    guard initialized, callbackInstalled else {
      throw ProbeError.capture("microphone AUHAL is not prepared")
    }
    startAttempted = true
    try api.start(unit)
    locked { started = true }
    try proveOrAwaitRevision()
  }
  func running() -> Bool {
    guard locked({ started && !closing && !closed && terminalFailure == nil }) else { return false }
    do {
      return try api.uint32(unit, kAudioOutputUnitProperty_IsRunning, kAudioUnitScope_Global, 0)
        != 0
    } catch {
      fail(String(describing: error))
      return false
    }
  }
  func verify(_ expected: MicrophoneRecovery.Binding) throws {
    let before = try locked { () throws -> UInt64 in
      guard binding == expected, !closing, !closed else {
        throw ProbeError.capture("microphone AUHAL binding is retired")
      }
      if let terminalFailure { throw ProbeError.capture(terminalFailure) }
      return revision
    }
    guard
      try api.uint32(unit, kAudioOutputUnitProperty_CurrentDevice, kAudioUnitScope_Global, 0)
        == expected.objectID,
      try api.deviceUID(expected.objectID) == expected.uid,
      try api.deviceAlive(expected.objectID),
      MicrophonePCMFormat.matches(format, try nativeFormat())
    else {
      throw ProbeError.capture("microphone AUHAL selected device or format changed")
    }
    if let capacity = locked({ buffer?.frameCapacity }) {
      let maximum = try api.uint32(
        unit, kAudioUnitProperty_MaximumFramesPerSlice,
        kAudioUnitScope_Global, 0)
      guard maximum > 0, maximum <= capacity else {
        throw ProbeError.capture("microphone AUHAL slice exceeds bounded PCM capacity")
      }
    }
    try locked {
      guard !closing, !closed else {
        throw ProbeError.capture("microphone AUHAL binding is retired")
      }
      if let terminalFailure { throw ProbeError.capture(terminalFailure) }
      guard revision == before else { throw MicrophoneDeviceProofPending() }
      provenRevision = before
    }
  }
  func verifyCallback(_ expected: MicrophoneRecovery.Binding) throws {
    try locked {
      guard binding == expected, !closing, !closed else {
        throw ProbeError.capture("microphone AUHAL callback binding is retired")
      }
      if let terminalFailure { throw ProbeError.capture(terminalFailure) }
      guard provenRevision == revision else { throw MicrophoneDeviceProofPending() }
    }
  }
  func changed() {
    let work = locked { () -> (Bool, (@Sendable () -> Void)?) in
      guard !closing, !closed else { return (false, nil) }
      inFlight += 1
      if revision < UInt64.max {
        revision += 1
      } else {
        terminalFailure = "microphone AUHAL metadata revision exhausted"
      }
      provenRevision = nil
      return (true, observer)
    }
    guard work.0 else { return }
    defer {
      locked {
        inFlight -= 1
        condition.broadcast()
      }
    }
    work.1?()
  }
  private func fail(_ detail: String) {
    let notify = locked { () -> (@Sendable () -> Void)? in
      guard !closing, !closed, terminalFailure == nil else { return nil }
      terminalFailure = detail
      provenRevision = nil
      return observer
    }
    notify?()
  }
  func receive(
    flags: UnsafeMutablePointer<AudioUnitRenderActionFlags>,
    timestamp: UnsafePointer<AudioTimeStamp>, frames: UInt32
  ) -> OSStatus {
    let work = locked {
      () -> (AVAudioPCMBuffer, @Sendable (AVAudioPCMBuffer, AVAudioTime) -> Void)? in
      guard accepting, started, !closing, !closed, terminalFailure == nil,
        provenRevision == revision,
        let buffer, let tap
      else { return nil }
      guard !rendering, frames > 0, frames <= buffer.frameCapacity else { return nil }
      rendering = true
      inFlight += 1
      return (buffer, tap)
    }
    guard let (buffer, tap) = work else {
      if locked({
        accepting && !closing && !closed && terminalFailure == nil
          && (rendering || frames == 0 || frames > (self.buffer?.frameCapacity ?? 0))
      }) {
        fail("microphone AUHAL callback exceeds PCM capacity or overlaps")
        return kAudioUnitErr_TooManyFramesToProcess
      }
      return noErr
    }
    defer {
      locked {
        rendering = false
        inFlight -= 1
        condition.broadcast()
      }
    }
    buffer.frameLength = frames
    // AVAudioPCMBuffer's list accessor may regenerate its header. Hold the
    // exact list passed to Render so a second accessor cannot hide corruption.
    let audioList = buffer.mutableAudioBufferList
    let parts = UnsafeMutableAudioBufferListPointer(audioList)
    restoreLayout(audioList, frames: frames)
    let status = api.render(unit, flags, timestamp, frames, audioList)
    guard status == noErr else {
      restoreLayout(audioList, frames: frames)
      fail("microphone AUHAL render failed: \(status)")
      return status
    }
    // The returned ABL header is untrusted. Restore allocation-owned metadata
    // before either walking planes or eventually releasing AVAudioPCMBuffer.
    guard audioList.pointee.mNumberBuffers == storagePointers.count else {
      restoreLayout(audioList, frames: frames)
      fail("microphone AUHAL returned an invalid PCM plane count")
      return kAudioUnitErr_FormatNotSupported
    }
    guard
      (0..<storagePointers.count).allSatisfy({
        parts[$0].mData == storagePointers[$0] && parts[$0].mNumberChannels == 1
          && parts[$0].mDataByteSize == frames * UInt32(MemoryLayout<Float>.size)
      })
    else {
      restoreLayout(audioList, frames: frames)
      fail("microphone AUHAL returned an invalid PCM layout")
      return kAudioUnitErr_FormatNotSupported
    }
    let ready = locked {
      accepting && !closing && !closed && terminalFailure == nil && provenRevision == revision
    }
    guard ready else { return noErr }
    // Preserve hardware sample/host flags and ticks. No arrival timestamp is
    // substituted for a missing hardware clock observation.
    tap(buffer, AVAudioTime(audioTimeStamp: timestamp, sampleRate: format.sampleRate))
    return noErr
  }
  private func restoreLayout(_ audioList: UnsafeMutablePointer<AudioBufferList>, frames: UInt32) {
    audioList.pointee.mNumberBuffers = UInt32(storagePointers.count)
    let parts = UnsafeMutableAudioBufferListPointer(audioList)
    for index in 0..<storagePointers.count {
      parts[index].mData = storagePointers[index]
      parts[index].mNumberChannels = 1
      parts[index].mDataByteSize = frames * UInt32(MemoryLayout<Float>.size)
    }
  }
  func close() throws {
    if locked({ closed }) { return }
    locked {
      closing = true
      accepting = false
      tap = nil
      observer = nil
      provenRevision = nil
    }
    if startAttempted {
      try api.stop(unit)
      startAttempted = false
    }
    condition.lock()
    while inFlight > 0 { condition.wait() }
    started = false
    condition.unlock()
    if callbackInstalled {
      try set(
        kAudioOutputUnitProperty_SetInputCallback, kAudioUnitScope_Global, 0,
        AURenderCallbackStruct(inputProc: nil, inputProcRefCon: nil))
      callbackInstalled = false
    }
    while let property = unitListeners.last {
      try api.removeUnitListener(unit, property, microphoneUnitChanged, refcon)
      unitListeners.removeLast()
    }
    while let address = deviceListeners.last {
      try api.removeDeviceListener(binding.objectID, address, microphoneDeviceChanged, refcon)
      deviceListeners.removeLast()
    }
    condition.lock()
    while inFlight > 0 { condition.wait() }
    condition.unlock()
    if initialized {
      try api.uninitialize(unit)
      initialized = false
    }
    try api.dispose(unit)
    locked {
      closed = true
      buffer = nil
      storagePointers = []
    }
    if let token {
      MicrophoneAudioUnitRegistry.shared.unregister(token)
      self.token = nil
    }
    if retained {
      retained = false
      Unmanaged.passUnretained(self).release()
    }
  }
}
