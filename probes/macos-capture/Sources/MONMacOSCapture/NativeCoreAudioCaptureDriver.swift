import AVFoundation
import CoreAudio
import CoreMedia
import Darwin
import Foundation

/// Explicit inclusive/exclusive scope. An empty application selection must
/// never be interpreted as permission to acquire global output.
enum CoreAudioTapScope: Equatable, Sendable {
  case system
  case application(CaptureApplicationIdentity)

  func description(objects: [(pid: pid_t, objectID: AudioObjectID)], ownPID: pid_t)
    throws -> CATapDescription
  {
    guard objects.count <= TapAudioProcesses.maximumObjects,
      objects.allSatisfy({ $0.pid > 0 && $0.objectID != kAudioObjectUnknown }),
      Set(objects.map(\.objectID)).count == objects.count
    else { throw ProbeError.capture("tap process catalog is invalid or ambiguous") }
    let description: CATapDescription
    switch self {
    case .system:
      guard ownPID > 0 else { throw ProbeError.capture("tap owner PID is invalid") }
      description = CATapDescription(
        stereoGlobalTapButExcludeProcesses: objects.filter { $0.pid == ownPID }.map(\.objectID))
    case .application(let identity):
      guard identity.valid, identity.pid != ownPID else {
        throw ProbeError.capture("selected tap application identity is invalid")
      }
      let selected = objects.filter { $0.pid == identity.pid }.map(\.objectID)
      // One application may have several audio objects; all must retain the
      // exact confirmed PID. No name/bundle-only selection or global fallback.
      guard !selected.isEmpty else {
        throw ProbeError.capture("selected application has no audio process object; no fallback")
      }
      description = CATapDescription(stereoMixdownOfProcesses: selected)
    }
    description.name = "MON audio-only capture"
    description.isPrivate = true
    description.muteBehavior = .unmuted
    return description
  }
}

/// Allows native ownership/cleanup failures to be tested without creating any
/// OS tap, aggregate device, permission request or audio input.
protocol CoreAudioTapHAL: Sendable {
  func processes() throws -> [(pid: pid_t, objectID: AudioObjectID)]
  func createTap(_ description: CATapDescription) throws -> AudioObjectID
  func tapUID(_ tap: AudioObjectID) throws -> String
  func createAggregate(tapUID: String) throws -> AudioObjectID
  func format(_ tap: AudioObjectID) throws -> AudioStreamBasicDescription
  func processPID(_ object: AudioObjectID) throws -> pid_t
  func alive(_ device: AudioObjectID) throws -> UInt32
  func createIO(
    _ device: AudioObjectID, queue: DispatchQueue,
    block: @escaping AudioDeviceIOBlock
  ) throws -> AudioDeviceIOProcID
  func start(_ device: AudioObjectID, io: AudioDeviceIOProcID) -> OSStatus
  func stop(_ device: AudioObjectID, io: AudioDeviceIOProcID) -> OSStatus
  func destroyIO(_ device: AudioObjectID, io: AudioDeviceIOProcID) throws
  func destroyAggregate(_ device: AudioObjectID) throws
  func destroyTap(_ tap: AudioObjectID) throws
}

/// All lifecycle methods are serialized by CoreAudioCaptureStream. HAL blocks
/// copy borrowed bytes on a separate synchronous callback queue before return.
final class NativeCoreAudioCaptureDriver: CoreAudioCaptureDriver, @unchecked Sendable {
  private let scope: CoreAudioTapScope
  private let hal: any CoreAudioTapHAL
  private let ioQueue = DispatchQueue(label: "app.mon.capture.tap-io")
  private let monitorQueue = DispatchQueue(label: "app.mon.capture.tap-monitor")
  private var monitor: DispatchSourceTimer?
  private var tap: AudioObjectID = AudioObjectID(kAudioObjectUnknown)
  private var aggregate: AudioObjectID = AudioObjectID(kAudioObjectUnknown)
  private var io: AudioDeviceIOProcID?
  private var startAttempted = false
  private var running = false
  private var acquired = false
  private var detached = false
  private let requireIdentity: @Sendable (CaptureApplicationIdentity) throws -> Void
  private let mayContinue: @Sendable () -> Bool

  init(
    scope: CoreAudioTapScope, hal: any CoreAudioTapHAL = NativeCoreAudioTapHAL(),
    mayContinue: @escaping @Sendable () -> Bool = { true },
    requireIdentity: @escaping @Sendable (CaptureApplicationIdentity) throws -> Void =
      CaptureApplications.current
  ) {
    self.scope = scope
    self.hal = hal
    self.mayContinue = mayContinue
    self.requireIdentity = requireIdentity
  }

  func start(
    sample: @escaping @Sendable (CMSampleBuffer) -> Void,
    failure: @escaping @Sendable (Error) -> Void
  ) throws {
    guard !acquired, !detached else { throw ProbeError.capture("tap acquisition already owned") }
    acquired = true
    guard mayContinue() else {
      throw ProbeError.capture("tap acquisition cancelled before preparation")
    }
    if case .application(let identity) = scope { try requireIdentity(identity) }
    let objects = try hal.processes()
    let description = try scope.description(objects: objects, ownPID: getpid())
    tap = try hal.createTap(description)
    let uid = try hal.tapUID(tap)
    aggregate = try hal.createAggregate(tapUID: uid)
    let asbd = try hal.format(tap)
    guard let format = AVAudioFormat(streamDescription: withUnsafePointer(to: asbd) { $0 }),
      format.commonFormat == .pcmFormatFloat32
    else {
      throw ProbeError.capture("tap does not supply Float32 PCM")
    }
    let formatBox = SendableFormat(value: format)
    io = try hal.createIO(aggregate, queue: ioQueue) {
      _, input, timestamp, _, _ in
      // Zero-length callbacks are not evidence of denied permission or PCM.
      let buffers = UnsafeMutableAudioBufferListPointer(UnsafeMutablePointer(mutating: input))
      guard buffers.count <= 32 else {
        failure(ProbeError.capture("tap input exceeds bounded plane capacity"))
        return
      }
      if buffers.count == 0 || buffers.allSatisfy({ $0.mDataByteSize == 0 }) { return }
      do {
        sample(
          try CoreAudioTapSample.copy(
            input, time: timestamp.pointee,
            format: formatBox.value))
      } catch { failure(error) }
    }
    if case .application(let identity) = scope {
      try requireIdentity(identity)
      try requireObjects(description.processes, pid: identity.pid)
    }
    guard mayContinue() else {
      throw ProbeError.capture("tap acquisition cancelled before native IO start")
    }
    // Permission may be requested only here, at an explicitly requested start.
    // Even a failed start retains the IO ID until stop/detach prove closure.
    startAttempted = true
    guard let io else { throw ProbeError.capture("tap IO callback is missing") }
    try checked(hal.start(aggregate, io: io), "start tap IO")
    running = true
    let selected = description.processes
    let tapID = tap
    let aggregateID = aggregate
    let timer = DispatchSource.makeTimerSource(queue: monitorQueue)
    timer.schedule(deadline: .now() + .milliseconds(250), repeating: .milliseconds(250))
    timer.setEventHandler { [weak self] in
      guard let self else { return }
      do {
        if case .application(let identity) = self.scope {
          try self.requireIdentity(identity)
          try self.requireObjects(selected, pid: identity.pid)
        }
        let alive = try self.hal.alive(aggregateID)
        let current = try self.hal.format(tapID)
        guard alive == 1, Self.sameFormat(asbd, current) else {
          throw ProbeError.capture("tap device disconnected or audio format changed")
        }
      } catch { failure(error) }
    }
    monitor = timer
    timer.resume()
  }

  func stop() throws {
    monitor?.cancel()
    monitor = nil
    monitorQueue.sync {}
    if let io {
      if running || startAttempted {
        let status = hal.stop(aggregate, io: io)
        // A failed start can already have no running IO; a running source must
        // obtain an actual successful stop, not interpret an error as closure.
        guard status == noErr || (!running && status == kAudioHardwareNotRunningError) else {
          throw CaptureFailureGuide.coreAudioFailure(status, operation: "stop tap IO")
        }
        running = false
        startAttempted = false
      }
      try hal.destroyIO(aggregate, io: io)
      self.io = nil
    }
    ioQueue.sync {}
  }

  func detach() throws {
    guard !running, !startAttempted, io == nil else {
      throw ProbeError.capture("tap native IO closure has not been proven")
    }
    if detached { return }
    if aggregate != kAudioObjectUnknown {
      try hal.destroyAggregate(aggregate)
      aggregate = AudioObjectID(kAudioObjectUnknown)
    }
    if tap != kAudioObjectUnknown {
      try hal.destroyTap(tap)
      tap = AudioObjectID(kAudioObjectUnknown)
    }
    detached = true
  }

  private struct SendableFormat: @unchecked Sendable { let value: AVAudioFormat }

  static func sameFormat(_ a: AudioStreamBasicDescription, _ b: AudioStreamBasicDescription) -> Bool
  {
    a.mSampleRate == b.mSampleRate && a.mFormatID == b.mFormatID
      && a.mFormatFlags == b.mFormatFlags && a.mBytesPerPacket == b.mBytesPerPacket
      && a.mFramesPerPacket == b.mFramesPerPacket && a.mBytesPerFrame == b.mBytesPerFrame
      && a.mChannelsPerFrame == b.mChannelsPerFrame && a.mBitsPerChannel == b.mBitsPerChannel
  }

  private func requireObjects(_ selected: [AudioObjectID], pid: pid_t) throws {
    guard !selected.isEmpty else { throw ProbeError.capture("selected tap objects are empty") }
    for object in selected {
      let current = try hal.processPID(object)
      guard current == pid else { throw ProbeError.capture("tap process object identity changed") }
    }
  }

  private func checked(_ status: OSStatus, _ operation: String) throws {
    guard status == noErr else {
      throw CaptureFailureGuide.coreAudioFailure(status, operation: operation)
    }
  }
}

struct NativeCoreAudioTapHAL: CoreAudioTapHAL {
  func processes() throws -> [(pid: pid_t, objectID: AudioObjectID)] {
    try TapAudioProcesses.list()
  }
  func createTap(_ description: CATapDescription) throws -> AudioObjectID {
    var tap = AudioObjectID(kAudioObjectUnknown)
    try checked(AudioHardwareCreateProcessTap(description, &tap), "create process tap")
    guard tap != kAudioObjectUnknown else { throw ProbeError.capture("created tap is missing") }
    return tap
  }
  func tapUID(_ tap: AudioObjectID) throws -> String {
    try string(tap, kAudioTapPropertyUID)
  }
  func createAggregate(tapUID: String) throws -> AudioObjectID {
    let specification: [String: Any] = [
      kAudioAggregateDeviceNameKey: "MON Audio-only Tap",
      kAudioAggregateDeviceUIDKey: "app.mon.capture.audio-only.\(UUID().uuidString)",
      kAudioAggregateDeviceIsPrivateKey: true,
      kAudioAggregateDeviceTapAutoStartKey: true,
      kAudioAggregateDeviceTapListKey: [[kAudioSubTapUIDKey: tapUID]],
    ]
    var device = AudioObjectID(kAudioObjectUnknown)
    try checked(
      AudioHardwareCreateAggregateDevice(specification as CFDictionary, &device),
      "create tap aggregate")
    guard device != kAudioObjectUnknown else {
      throw ProbeError.capture("created aggregate is missing")
    }
    return device
  }
  func format(_ tap: AudioObjectID) throws -> AudioStreamBasicDescription {
    try value(tap, kAudioTapPropertyFormat)
  }
  func processPID(_ object: AudioObjectID) throws -> pid_t {
    try value(object, kAudioProcessPropertyPID)
  }
  func alive(_ device: AudioObjectID) throws -> UInt32 {
    try value(device, kAudioDevicePropertyDeviceIsAlive)
  }
  func createIO(
    _ device: AudioObjectID, queue: DispatchQueue,
    block: @escaping AudioDeviceIOBlock
  ) throws -> AudioDeviceIOProcID {
    var io: AudioDeviceIOProcID?
    try checked(
      AudioDeviceCreateIOProcIDWithBlock(&io, device, queue, block), "create tap IO callback")
    guard let io else { throw ProbeError.capture("created tap IO callback is missing") }
    return io
  }
  func start(_ device: AudioObjectID, io: AudioDeviceIOProcID) -> OSStatus {
    AudioDeviceStart(device, io)
  }
  func stop(_ device: AudioObjectID, io: AudioDeviceIOProcID) -> OSStatus {
    AudioDeviceStop(device, io)
  }
  func destroyIO(_ device: AudioObjectID, io: AudioDeviceIOProcID) throws {
    try checked(AudioDeviceDestroyIOProcID(device, io), "destroy tap IO callback")
  }
  func destroyAggregate(_ device: AudioObjectID) throws {
    try checked(AudioHardwareDestroyAggregateDevice(device), "destroy tap aggregate")
  }
  func destroyTap(_ tap: AudioObjectID) throws {
    try checked(AudioHardwareDestroyProcessTap(tap), "destroy process tap")
  }

  private func checked(_ status: OSStatus, _ operation: String) throws {
    guard status == noErr else {
      throw CaptureFailureGuide.coreAudioFailure(status, operation: operation)
    }
  }

  private func value<T>(_ object: AudioObjectID, _ selector: AudioObjectPropertySelector) throws
    -> T
  {
    var address = AudioObjectPropertyAddress(
      mSelector: selector,
      mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain)
    let pointer = UnsafeMutablePointer<T>.allocate(capacity: 1)
    defer { pointer.deallocate() }
    var size = UInt32(MemoryLayout<T>.stride)
    try checked(
      AudioObjectGetPropertyData(object, &address, 0, nil, &size, pointer), "read tap property")
    guard size == MemoryLayout<T>.stride else {
      throw ProbeError.capture("tap property size changed")
    }
    return pointer.move()
  }

  private func string(_ object: AudioObjectID, _ selector: AudioObjectPropertySelector) throws
    -> String
  {
    let result: CFString = try value(object, selector)
    guard CFStringGetLength(result) > 0, CFStringGetLength(result) <= 1024 else {
      throw ProbeError.capture("tap UID is missing or exceeds capacity")
    }
    return result as String
  }
}
