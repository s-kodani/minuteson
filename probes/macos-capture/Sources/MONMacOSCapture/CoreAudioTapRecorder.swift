import AVFoundation
import CoreAudio
import Foundation

enum TapAudioProcesses {
  static func list() throws -> [(pid: pid_t, objectID: AudioObjectID)] {
    var address = AudioObjectPropertyAddress(
      mSelector: kAudioHardwarePropertyProcessObjectList,
      mScope: kAudioObjectPropertyScopeGlobal,
      mElement: kAudioObjectPropertyElementMain)
    var size: UInt32 = 0
    try check(
      AudioObjectGetPropertyDataSize(
        AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &size),
      "get audio process list size")
    guard size % UInt32(MemoryLayout<AudioObjectID>.stride) == 0 else {
      throw ProbeError.capture("audio process list has an invalid byte length")
    }
    let count = Int(size) / MemoryLayout<AudioObjectID>.stride
    guard count > 0 else { return [] }
    var ids = [AudioObjectID](repeating: 0, count: count)
    try ids.withUnsafeMutableBufferPointer { buffer in
      try check(
        AudioObjectGetPropertyData(
          AudioObjectID(kAudioObjectSystemObject), &address, 0, nil, &size,
          buffer.baseAddress!), "get audio process list")
    }
    var result: [(pid: pid_t, objectID: AudioObjectID)] = []
    for id in ids {
      var pidAddress = AudioObjectPropertyAddress(
        mSelector: kAudioProcessPropertyPID,
        mScope: kAudioObjectPropertyScopeGlobal,
        mElement: kAudioObjectPropertyElementMain)
      var pid: pid_t = 0
      var pidSize = UInt32(MemoryLayout<pid_t>.stride)
      if AudioObjectGetPropertyData(id, &pidAddress, 0, nil, &pidSize, &pid) == noErr, pid > 0 {
        result.append((pid, id))
      }
    }
    return result.sorted { $0.pid < $1.pid }
  }
}

final class CoreAudioTapRecorder: @unchecked Sendable {
  private struct SendableBuffer: @unchecked Sendable {
    let value: AVAudioPCMBuffer
    let observation: CaptureClockObservation?
    let sourceFrame: Int64?
    let presentation: Double?
  }

  private let outputURL: URL
  private let writeQueue = DispatchQueue(label: "app.mon.capture.coreaudio-write")
  private let pendingBuffers = DispatchSemaphore(value: 64)
  private let lock = NSLock()
  private var file: AVAudioFile?
  private var tapID: AudioObjectID = AudioObjectID(kAudioObjectUnknown)
  private var aggregateID: AudioObjectID = AudioObjectID(kAudioObjectUnknown)
  private var ioProcID: AudioDeviceIOProcID?
  private var started = false
  private var everStarted = false
  private var tornDown = false
  private var shutdownFailures: [String] = []
  private var stats = TrackStats()
  private let clock: CaptureClockDomain?
  private let levelsEnabled: Bool
  private let rawSegments: RawRecordingSegments?

  init(
    outputURL: URL, clock: CaptureClockDomain? = nil, levelsEnabled: Bool = false,
    rawSegments: RawRecordingSegments? = nil
  ) {
    self.outputURL = outputURL
    self.clock = clock
    self.levelsEnabled = levelsEnabled
    self.rawSegments = rawSegments
  }

  func start(targetPIDs: [pid_t]) throws {
    guard !everStarted, !tornDown else {
      throw ProbeError.capture("tap recorder cannot restart after acquisition or teardown")
    }
    let objects = try TapAudioProcesses.list()
    let selected = targetPIDs.compactMap { pid in
      objects.first(where: { $0.pid == pid })?.objectID
    }
    guard selected.count == targetPIDs.count else {
      throw ProbeError.capture("one or more selected PIDs have no Core Audio process object")
    }
    let description = CATapDescription(stereoMixdownOfProcesses: selected)
    description.name = "MON selected application audio"
    description.isPrivate = true
    description.muteBehavior = .unmuted
    do {
      try check(AudioHardwareCreateProcessTap(description, &tapID), "create process tap")
      let tapUID = try propertyString(tapID, kAudioTapPropertyUID)
      let aggregateDescription: [String: Any] = [
        kAudioAggregateDeviceNameKey: "MON Capture Tap",
        kAudioAggregateDeviceUIDKey: "app.mon.capture.\(UUID().uuidString)",
        kAudioAggregateDeviceIsPrivateKey: true,
        kAudioAggregateDeviceTapListKey: [[kAudioSubTapUIDKey: tapUID]],
      ]
      try check(
        AudioHardwareCreateAggregateDevice(aggregateDescription as CFDictionary, &aggregateID),
        "create tap aggregate device")
      let streamDescription: AudioStreamBasicDescription = try propertyValue(
        tapID, kAudioTapPropertyFormat)
      guard
        let format = AVAudioFormat(
          streamDescription: withUnsafePointer(to: streamDescription) { $0 })
      else {
        throw ProbeError.capture("tap audio format is unsupported")
      }
      guard format.commonFormat == .pcmFormatFloat32 else {
        throw ProbeError.capture("tap audio format is not Float32 PCM")
      }
      file = try AVAudioFile(
        forWriting: outputURL, settings: format.settings,
        commonFormat: .pcmFormatFloat32, interleaved: format.isInterleaved)
      lock.withLock {
        stats.sampleRate = format.sampleRate
        stats.channels = format.channelCount
      }
      try check(
        AudioDeviceCreateIOProcIDWithBlock(&ioProcID, aggregateID, nil) {
          [weak self] _, inputData, inputTime, _, _ in
          self?.receive(inputData: inputData, inputTime: inputTime, format: format)
        }, "create aggregate device IO callback")
      try check(AudioDeviceStart(aggregateID, ioProcID), "start tap aggregate device")
      started = true
      everStarted = true
    } catch {
      cleanup()
      throw error
    }
  }

  func stop() throws {
    let failures = tearDown()
    if !failures.isEmpty {
      throw ProbeError.capture(failures.joined(separator: "; "))
    }
  }

  func stopRecording() throws -> CaptureClosure {
    guard everStarted else { throw ProbeError.capture("tap acquisition never started") }
    let failures = tearDown(proveEnding: true)
    guard failures.isEmpty, let captureClosure else {
      throw ProbeError.capture(
        failures.isEmpty
          ? "tap acquisition has no proven closure" : failures.joined(separator: "; "))
    }
    return captureClosure
  }

  func snapshot() -> TrackStats {
    lock.withLock {
      var result = stats
      result.rawSegments = rawSegments?.snapshot()
      if result.captureFailure != nil { result.inputLevel.invalidate() }
      return result
    }
  }

  private var captureClosure: CaptureClosure?

  private func tearDown(proveEnding: Bool = false) -> [String] {
    if tornDown { return shutdownFailures }
    tornDown = true
    var failures: [String] = []
    if started {
      let status = AudioDeviceStop(aggregateID, ioProcID)
      if status != noErr { failures.append("stop tap aggregate device: \(status)") }
      started = false
    }
    if let ioProcID {
      let status = AudioDeviceDestroyIOProcID(aggregateID, ioProcID)
      if status != noErr { failures.append("destroy IO callback: \(status)") }
      self.ioProcID = nil
    }
    writeQueue.sync {}
    do { try rawSegments?.finish() } catch {
      failures.append("raw tap audio finalization failed")
      lock.withLock { stats.appendFailures += 1 }
    }
    if proveEnding, failures.isEmpty, captureClosure == nil {
      do {
        guard let clock else { throw ProbeError.capture("tap capture clock is missing") }
        captureClosure = try clock.close(.system)
      } catch { failures.append(String(describing: error)) }
    }
    lock.withLock { file = nil }
    if aggregateID != AudioObjectID(kAudioObjectUnknown) {
      let status = AudioHardwareDestroyAggregateDevice(aggregateID)
      if status != noErr { failures.append("destroy aggregate device: \(status)") }
      aggregateID = AudioObjectID(kAudioObjectUnknown)
    }
    if tapID != AudioObjectID(kAudioObjectUnknown) {
      let status = AudioHardwareDestroyProcessTap(tapID)
      if status != noErr { failures.append("destroy process tap: \(status)") }
      tapID = AudioObjectID(kAudioObjectUnknown)
    }
    shutdownFailures = failures
    return failures
  }

  private func receive(
    inputData: UnsafePointer<AudioBufferList>, inputTime: UnsafePointer<AudioTimeStamp>,
    format: AVAudioFormat
  ) {
    guard lock.withLock({ (try? stats.requireWritable(raw: rawSegments?.snapshot())) != nil })
    else {
      return
    }
    let receivedAt = levelsEnabled ? PCMInputLevel.nowNanoseconds() : nil
    let sources = UnsafeMutableAudioBufferListPointer(UnsafeMutablePointer(mutating: inputData))
    guard !sources.isEmpty, let first = sources.first, first.mData != nil else { return }
    let bytesPerFrame = Int(format.streamDescription.pointee.mBytesPerFrame)
    guard bytesPerFrame > 0, Int(first.mDataByteSize) % bytesPerFrame == 0 else {
      lock.withLock { stats.appendFailures += 1 }
      return
    }
    let frames = AVAudioFrameCount(Int(first.mDataByteSize) / bytesPerFrame)
    guard frames > 0, pendingBuffers.wait(timeout: .now()) == .success else {
      lock.withLock { stats.appendFailures += 1 }
      return
    }
    guard let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: frames) else {
      pendingBuffers.signal()
      lock.withLock { stats.appendFailures += 1 }
      return
    }
    buffer.frameLength = frames
    let destinations = UnsafeMutableAudioBufferListPointer(buffer.mutableAudioBufferList)
    guard sources.count == destinations.count else {
      pendingBuffers.signal()
      lock.withLock { stats.appendFailures += 1 }
      return
    }
    for index in sources.indices {
      guard let source = sources[index].mData, let destination = destinations[index].mData,
        sources[index].mDataByteSize == destinations[index].mDataByteSize,
        sources[index].mNumberChannels == destinations[index].mNumberChannels
      else {
        pendingBuffers.signal()
        lock.withLock { stats.appendFailures += 1 }
        return
      }
      memcpy(destination, source, Int(sources[index].mDataByteSize))
    }
    let presentation: Double? =
      inputTime.pointee.mFlags.contains(.sampleTimeValid)
      ? inputTime.pointee.mSampleTime / format.sampleRate : nil
    let rawFrame = inputTime.pointee.mSampleTime
    let sourceFrame =
      inputTime.pointee.mFlags.contains(.sampleTimeValid)
      ? Int64(exactly: rawFrame) : nil
    var observation = clock?.observe(
      hostTicks: inputTime.pointee.mFlags.contains(.hostTimeValid)
        ? inputTime.pointee.mHostTime : nil)
    if clock != nil, inputTime.pointee.mFlags.contains(.sampleTimeValid), sourceFrame == nil {
      observation = .invalid("tap source frame counter is not an exact integer")
    }
    lock.withLock {
      stats.callbackCount += 1
      if levelsEnabled { stats.inputLevel.receive(buffer, at: receivedAt) }
    }
    let queuedBuffer = SendableBuffer(
      value: buffer, observation: observation, sourceFrame: sourceFrame,
      presentation: presentation)
    writeQueue.async {
      defer { self.pendingBuffers.signal() }
      self.lock.withLock {
        guard (try? self.stats.requireWritable(raw: self.rawSegments?.snapshot())) != nil else {
          self.stats.inputLevel.invalidate()
          return
        }
        do {
          guard let file = self.file else { throw ProbeError.capture("tap CAF writer is closed") }
          try self.stats.prepareSavedClock(
            queuedBuffer.observation, frames: Int64(queuedBuffer.value.frameLength),
            sourceFrame: queuedBuffer.sourceFrame)
          try file.write(from: queuedBuffer.value)
          self.stats.frames += Int64(queuedBuffer.value.frameLength)
          if let presentation = queuedBuffer.presentation, presentation.isFinite {
            if self.stats.firstPresentationTimeSeconds == nil {
              self.stats.firstPresentationTimeSeconds = presentation
            }
            self.stats.lastPresentationTimeSeconds = presentation
          }
          try self.rawSegments?.enqueue(
            queuedBuffer.value, observation: queuedBuffer.observation,
            sourceFrame: queuedBuffer.sourceFrame)
        } catch {
          self.stats.appendFailures += 1
          self.stats.inputLevel.invalidate()
        }
      }
    }
  }

  private func cleanup() {
    _ = tearDown()
  }
}

private func check(_ status: OSStatus, _ operation: String) throws {
  guard status == noErr else {
    throw ProbeError.capture("\(operation) failed with Core Audio status \(status)")
  }
}

private func propertyValue<T>(
  _ objectID: AudioObjectID, _ selector: AudioObjectPropertySelector
) throws -> T {
  var address = AudioObjectPropertyAddress(
    mSelector: selector, mScope: kAudioObjectPropertyScopeGlobal,
    mElement: kAudioObjectPropertyElementMain)
  let value = UnsafeMutablePointer<T>.allocate(capacity: 1)
  defer { value.deallocate() }
  var size = UInt32(MemoryLayout<T>.stride)
  try check(AudioObjectGetPropertyData(objectID, &address, 0, nil, &size, value), "read property")
  guard size == MemoryLayout<T>.stride else { throw ProbeError.capture("unexpected property size") }
  return value.move()
}

private func propertyString(
  _ objectID: AudioObjectID, _ selector: AudioObjectPropertySelector
) throws -> String {
  var address = AudioObjectPropertyAddress(
    mSelector: selector, mScope: kAudioObjectPropertyScopeGlobal,
    mElement: kAudioObjectPropertyElementMain)
  var value: CFString = "" as CFString
  var size = UInt32(MemoryLayout<CFString>.stride)
  try withUnsafeMutablePointer(to: &value) { pointer in
    try check(
      AudioObjectGetPropertyData(objectID, &address, 0, nil, &size, pointer),
      "read string property")
  }
  guard size == MemoryLayout<CFString>.stride, CFStringGetLength(value) > 0 else {
    throw ProbeError.capture("tap UID is missing")
  }
  return value as String
}
