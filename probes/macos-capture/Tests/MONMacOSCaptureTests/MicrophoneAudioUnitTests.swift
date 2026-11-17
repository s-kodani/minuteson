import AVFoundation
import AudioToolbox
import CoreAudio
import Foundation
import XCTest

@testable import MONMacOSCapture

private struct UnitSet: Equatable {
  let property: UInt32
  let scope: UInt32
  let element: UInt32
  let value: UInt32?
}
private final class UnitAPIFake: MicrophoneAudioUnitAPI, @unchecked Sendable {
  let key = DispatchSpecificKey<Int>()
  private let lock = NSLock()
  private let handle = AudioUnit(bitPattern: 0x1234)!
  private var events: [String] = []
  private var writes: [UnitSet] = []
  private var wrongOwner = false
  private var reads = 0
  private var current: UInt32 = 7
  private var uid = "powerconf:input"
  private var alive = true
  private var maximum: UInt32 = 4096
  private var running = false
  private var callback: AURenderCallbackStruct?
  private var unitListeners: [(UInt32, AudioUnitPropertyListenerProc, UnsafeMutableRawPointer)] = []
  private var deviceListeners:
    [(AudioObjectPropertyAddress, AudioObjectPropertyListenerProc, UnsafeMutableRawPointer)] = []
  private var format = AudioStreamBasicDescription(
    mSampleRate: 16000, mFormatID: kAudioFormatLinearPCM,
    mFormatFlags: kAudioFormatFlagIsSignedInteger | kAudioFormatFlagIsPacked,
    mBytesPerPacket: 2, mFramesPerPacket: 1, mBytesPerFrame: 2, mChannelsPerFrame: 1,
    mBitsPerChannel: 16,
    mReserved: 0)
  var onUIDRead: (@Sendable () -> Void)?
  var onRender: (@Sendable () -> Void)?
  var renderStatus: OSStatus = noErr
  var corruptLayout = false
  var corruptPlaneCount: UInt32?
  var corruptPointer = false
  var corruptChannels = false
  var failStop = false
  var failCallbackInstall = false
  var enlargedMaximumAtInitialize: UInt32?
  private var renderCalls = 0
  var calls: [String] { lock.withLock { events } }
  var settings: [UnitSet] { lock.withLock { writes } }
  var controlsStayedOnOwner: Bool { lock.withLock { !wrongOwner } }
  var metadataReads: Int { lock.withLock { reads } }
  var renders: Int { lock.withLock { renderCalls } }
  var callbackRemoved: Bool { lock.withLock { callback?.inputProc == nil } }
  func makeEngine() -> NativeMicrophoneCaptureEngine {
    let owner = DispatchQueue(label: "test.microphone-unit-owner")
    owner.setSpecific(key: key, value: 1)
    return NativeMicrophoneCaptureEngine(api: self, owner: owner)
  }
  private func control(_ name: String) {
    lock.withLock {
      if DispatchQueue.getSpecific(key: key) != 1 { wrongOwner = true }
      events.append(name)
    }
  }
  func setDevice(_ value: UInt32) { lock.withLock { current = value } }
  func setUID(_ value: String) { lock.withLock { uid = value } }
  func setAlive(_ value: Bool) { lock.withLock { alive = value } }
  func setMaximum(_ value: UInt32) { lock.withLock { maximum = value } }
  func setRate(_ value: Double) { lock.withLock { format.mSampleRate = value } }
  func create() throws -> AudioUnit {
    control("create")
    return handle
  }
  func set(
    _ unit: AudioUnit, _ property: UInt32, _ scope: UInt32, _ element: UInt32,
    _ value: UnsafeRawPointer, _ bytes: UInt32
  ) throws {
    control("set:\(property)")
    if property == kAudioOutputUnitProperty_SetInputCallback {
      let value = value.assumingMemoryBound(to: AURenderCallbackStruct.self).pointee
      if value.inputProc != nil && failCallbackInstall {
        throw ProbeError.capture("synthetic callback install failure")
      }
      lock.withLock {
        callback = value
        writes.append(UnitSet(property: property, scope: scope, element: element, value: nil))
      }
    } else if property == kAudioUnitProperty_StreamFormat {
      let description = value.assumingMemoryBound(to: AudioStreamBasicDescription.self).pointee
      XCTAssertEqual(scope, kAudioUnitScope_Output)
      XCTAssertEqual(element, 1)
      XCTAssertEqual(description.mSampleRate, lock.withLock { format.mSampleRate })
      XCTAssertEqual(description.mFormatID, kAudioFormatLinearPCM)
      XCTAssertNotEqual(description.mFormatFlags & kAudioFormatFlagIsFloat, 0)
      XCTAssertEqual(description.mBitsPerChannel, 32)
      lock.withLock {
        writes.append(UnitSet(property: property, scope: scope, element: element, value: nil))
      }
    } else {
      let value = value.assumingMemoryBound(to: UInt32.self).pointee
      lock.withLock {
        writes.append(UnitSet(property: property, scope: scope, element: element, value: value))
        if property == kAudioOutputUnitProperty_CurrentDevice { current = value }
        if property == kAudioUnitProperty_MaximumFramesPerSlice { maximum = value }
      }
    }
  }
  func uint32(_ unit: AudioUnit, _ property: UInt32, _ scope: UInt32, _ element: UInt32) throws
    -> UInt32
  {
    lock.withLock {
      reads += 1
      switch property {
      case kAudioOutputUnitProperty_CurrentDevice: return current
      case kAudioUnitProperty_MaximumFramesPerSlice: return maximum
      case kAudioOutputUnitProperty_IsRunning: return running ? 1 : 0
      default:
        XCTFail("unexpected property \(property)")
        return 0
      }
    }
  }
  func hardwareFormat(_ unit: AudioUnit) throws -> AudioStreamBasicDescription {
    lock.withLock {
      reads += 1
      return format
    }
  }
  func deviceUID(_ device: AudioObjectID) throws -> String {
    onUIDRead?()
    return lock.withLock {
      reads += 1
      return uid
    }
  }
  func deviceAlive(_ device: AudioObjectID) throws -> Bool {
    lock.withLock {
      reads += 1
      return alive
    }
  }
  func addUnitListener(
    _ unit: AudioUnit, _ property: UInt32,
    _ callback: AudioUnitPropertyListenerProc, _ context: UnsafeMutableRawPointer
  ) throws {
    control("addUnitListener")
    lock.withLock { unitListeners.append((property, callback, context)) }
  }
  func removeUnitListener(
    _ unit: AudioUnit, _ property: UInt32,
    _ callback: AudioUnitPropertyListenerProc, _ context: UnsafeMutableRawPointer
  ) throws {
    control("removeUnitListener")
    lock.withLock { unitListeners.removeAll { $0.0 == property && $0.2 == context } }
  }
  func addDeviceListener(
    _ device: AudioObjectID, _ address: AudioObjectPropertyAddress,
    _ callback: AudioObjectPropertyListenerProc, _ context: UnsafeMutableRawPointer
  ) throws {
    control("addDeviceListener")
    lock.withLock { deviceListeners.append((address, callback, context)) }
  }
  func removeDeviceListener(
    _ device: AudioObjectID, _ address: AudioObjectPropertyAddress,
    _ callback: AudioObjectPropertyListenerProc, _ context: UnsafeMutableRawPointer
  ) throws {
    control("removeDeviceListener")
    lock.withLock {
      deviceListeners.removeAll { $0.0.mSelector == address.mSelector && $0.2 == context }
    }
  }
  func initialize(_ unit: AudioUnit) throws {
    control("initialize")
    if let enlargedMaximumAtInitialize { setMaximum(enlargedMaximumAtInitialize) }
  }
  func start(_ unit: AudioUnit) throws {
    control("start")
    lock.withLock { running = true }
  }
  func stop(_ unit: AudioUnit) throws {
    control("stop")
    if failStop { throw ProbeError.capture("synthetic stop failure") }
    lock.withLock { running = false }
  }
  func uninitialize(_ unit: AudioUnit) throws { control("uninitialize") }
  func dispose(_ unit: AudioUnit) throws { control("dispose") }
  func render(
    _ unit: AudioUnit, _ flags: UnsafeMutablePointer<AudioUnitRenderActionFlags>,
    _ timestamp: UnsafePointer<AudioTimeStamp>, _ frames: UInt32,
    _ data: UnsafeMutablePointer<AudioBufferList>
  ) -> OSStatus {
    lock.withLock { renderCalls += 1 }
    onRender?()
    if renderStatus != noErr { return renderStatus }
    for part in UnsafeMutableAudioBufferListPointer(data) {
      XCTAssertEqual(part.mDataByteSize, frames * 4)
      XCTAssertEqual(part.mNumberChannels, 1)
      part.mData!.assumingMemoryBound(to: Float.self).initialize(
        repeating: 0.25, count: Int(frames))
    }
    if corruptLayout { data.pointee.mBuffers.mDataByteSize = 1 }
    if let corruptPlaneCount { data.pointee.mNumberBuffers = corruptPlaneCount }
    if corruptPointer { data.pointee.mBuffers.mData = UnsafeMutableRawPointer(bitPattern: 1) }
    if corruptChannels { data.pointee.mBuffers.mNumberChannels = 2 }
    return noErr
  }
  func savedInputCallback() -> AURenderCallbackStruct? { lock.withLock { callback } }
  func savedUnitNotification() -> (@Sendable () -> Void)? {
    guard let callback = lock.withLock({ unitListeners.first }) else { return nil }
    let saved = SavedUnitNotification(
      property: callback.0, callback: callback.1,
      context: callback.2, unit: handle)
    return { saved.invoke() }
  }
  func replay(_ callback: AURenderCallbackStruct, frames: UInt32 = 160) -> OSStatus {
    guard let proc = callback.inputProc, let context = callback.inputProcRefCon else {
      return noErr
    }
    var flags: AudioUnitRenderActionFlags = []
    var stamp = AudioTimeStamp()
    stamp.mFlags = [.hostTimeValid, .sampleTimeValid]
    stamp.mHostTime = 12345
    stamp.mSampleTime = 320
    return proc(context, &flags, &stamp, 1, frames, nil)
  }
  func fire(_ property: UInt32 = kAudioUnitProperty_StreamFormat) {
    let callback = lock.withLock { unitListeners.first { $0.0 == property } }
    callback?.1(callback!.2, handle, property, kAudioUnitScope_Input, 1)
  }
  func emit(
    _ frames: UInt32 = 160, host: UInt64 = 12345, sample: Float64 = 320,
    flags: AudioTimeStampFlags = [.hostTimeValid, .sampleTimeValid]
  ) -> OSStatus {
    let callback = lock.withLock { callback }
    guard let proc = callback?.inputProc, let context = callback?.inputProcRefCon else {
      return noErr
    }
    var actions: AudioUnitRenderActionFlags = []
    var stamp = AudioTimeStamp()
    stamp.mHostTime = host
    stamp.mSampleTime = sample
    stamp.mFlags = flags
    return proc(context, &actions, &stamp, 1, frames, nil)
  }
}
private struct SavedUnitNotification: @unchecked Sendable {
  let property: UInt32
  let callback: AudioUnitPropertyListenerProc
  let context: UnsafeMutableRawPointer
  let unit: AudioUnit
  func invoke() { callback(context, unit, property, kAudioUnitScope_Input, 1) }
}
private final class UnitDelivery: @unchecked Sendable {
  private let lock = NSLock()
  private var values: [(UInt32, Bool, UInt64, Bool, Int64, Double, Float)] = []
  func receive(_ buffer: AVAudioPCMBuffer, _ time: AVAudioTime) {
    do { try CaptureInputFault.validate(buffer) } catch {
      XCTFail("valid native PCM was rejected: \(error)")
    }
    lock.withLock {
      values.append(
        (
          buffer.frameLength, time.isHostTimeValid, time.hostTime,
          time.isSampleTimeValid, time.sampleTime, time.sampleRate, buffer.floatChannelData![0][0]
        ))
    }
  }
  var count: Int { lock.withLock { values.count } }
  var first: (UInt32, Bool, UInt64, Bool, Int64, Double, Float)? { lock.withLock { values.first } }
}
final class MicrophoneAudioUnitTests: XCTestCase, @unchecked Sendable {
  private let binding = MicrophoneRecovery.Binding(uid: "powerconf:input", objectID: 101, epoch: 1)
  private func started(_ api: UnitAPIFake, _ delivery: UnitDelivery = UnitDelivery()) throws
    -> NativeMicrophoneCaptureEngine
  {
    let engine = api.makeEngine()
    try engine.selectDevice(binding)
    try engine.installTap(format: engine.inputFormat, handler: delivery.receive)
    engine.prepare()
    try engine.start()
    return engine
  }
  func testInputOnlyAUHALOrderAndNativeFloat32ClientAvoidsDefaultAggregate() throws {
    let api = UnitAPIFake()
    let engine = try started(api)
    XCTAssertEqual(
      Array(api.settings.prefix(3)),
      [
        UnitSet(
          property: kAudioOutputUnitProperty_EnableIO, scope: kAudioUnitScope_Input, element: 1,
          value: 1),
        UnitSet(
          property: kAudioOutputUnitProperty_EnableIO, scope: kAudioUnitScope_Output, element: 0,
          value: 0),
        UnitSet(
          property: kAudioOutputUnitProperty_CurrentDevice, scope: kAudioUnitScope_Global,
          element: 0, value: 101),
      ])
    XCTAssertEqual(engine.inputFormat.sampleRate, 16000)
    XCTAssertEqual(engine.inputFormat.commonFormat, .pcmFormatFloat32)
    XCTAssertTrue(engine.isRunning)
    try engine.stop()
    XCTAssertTrue(api.controlsStayedOnOwner)
    XCTAssertEqual(Array(api.calls.suffix(2)), ["uninitialize", "dispose"])
    XCTAssertTrue(api.callbackRemoved)
  }
  func testAlreadySelectedDeviceDoesNotRenegotiateBluetooth() throws {
    let api = UnitAPIFake()
    api.setDevice(101)
    let engine = try started(api)
    defer { try? engine.stop() }
    XCTAssertFalse(api.settings.contains { $0.property == kAudioOutputUnitProperty_CurrentDevice })
  }
  func testCallbackPreservesHardwareClocksAndNeverReadsHALMetadata() throws {
    let api = UnitAPIFake()
    let delivery = UnitDelivery()
    let engine = try started(api, delivery)
    defer { try? engine.stop() }
    let before = api.metadataReads
    XCTAssertEqual(api.emit(), noErr)
    XCTAssertEqual(api.metadataReads, before)
    XCTAssertEqual(delivery.count, 1)
    let sample = try XCTUnwrap(delivery.first)
    XCTAssertEqual(sample.0, 160)
    XCTAssertTrue(sample.1)
    XCTAssertEqual(sample.2, 12345)
    XCTAssertTrue(sample.3)
    XCTAssertEqual(sample.4, 320)
    XCTAssertEqual(sample.5, 16000)
    XCTAssertEqual(sample.6, 0.25)
    try engine.verifyCallbackDevice(binding)
    XCTAssertEqual(api.metadataReads, before)
  }
  func testMetadataChangeRevokesCallbackProofBeforeControlRevalidation() throws {
    let api = UnitAPIFake()
    let delivery = UnitDelivery()
    let engine = try started(api, delivery)
    defer { try? engine.stop() }
    api.fire()
    XCTAssertThrowsError(try engine.verifyCallbackDevice(binding)) {
      XCTAssertTrue($0 is MicrophoneDeviceProofPending)
    }
    XCTAssertEqual(api.emit(), noErr)
    XCTAssertEqual(api.renders, 0)
    try engine.verifyDevice(binding)
    XCTAssertEqual(api.emit(), noErr)
    XCTAssertEqual(delivery.count, 1)
    api.setUID("different:input")
    api.fire(kAudioOutputUnitProperty_CurrentDevice)
    XCTAssertThrowsError(try engine.verifyDevice(binding))
    XCTAssertEqual(api.emit(), noErr)
    XCTAssertEqual(delivery.count, 1)
  }
  func testRacingUnchangedMetadataIsPendingInsteadOfADeviceMismatch() throws {
    let api = UnitAPIFake()
    let engine = try started(api)
    defer { try? engine.stop() }
    api.onUIDRead = { api.fire() }
    XCTAssertThrowsError(try engine.verifyDevice(binding)) {
      XCTAssertTrue($0 is MicrophoneDeviceProofPending)
    }
    api.onUIDRead = nil
    try engine.verifyDevice(binding)
    try engine.verifyCallbackDevice(binding)
  }
  func testOversizedZeroRenderErrorAndCorruptPCMNeverReachSinkOrResumeByProof() throws {
    let names = [
      "oversize", "zero", "render-status", "byte-count", "planes-99", "planes-0", "pointer",
      "channels",
    ]
    for failure in 0..<8 {
      let name = names[failure]
      let api = UnitAPIFake()
      let delivery = UnitDelivery()
      let engine = try started(api, delivery)
      if failure == 2 { api.renderStatus = -50 }
      if failure == 3 { api.corruptLayout = true }
      if failure == 4 { api.corruptPlaneCount = 99 }
      if failure == 5 { api.corruptPlaneCount = 0 }
      if failure == 6 { api.corruptPointer = true }
      if failure == 7 { api.corruptChannels = true }
      let frames: UInt32 = failure == 0 ? 4097 : failure == 1 ? 0 : 160
      XCTAssertNotEqual(api.emit(frames), noErr, name)
      XCTAssertEqual(delivery.count, 0, name)
      XCTAssertThrowsError(try engine.verifyDevice(binding), name)
      XCTAssertEqual(api.emit(), noErr, name)
      XCTAssertEqual(delivery.count, 0, name)
      try engine.stop()
    }
  }
  func testInitializeMaximumIsCheckedAgainstAllocatedCapacity() throws {
    let api = UnitAPIFake()
    api.enlargedMaximumAtInitialize = 3500
    let delivery = UnitDelivery()
    let engine = try started(api, delivery)
    defer { try? engine.stop() }
    XCTAssertEqual(api.emit(3500), noErr)
    XCTAssertEqual(delivery.count, 1)
    api.setMaximum(4097)
    api.fire(kAudioUnitProperty_MaximumFramesPerSlice)
    XCTAssertThrowsError(try engine.verifyDevice(binding))
  }
  func testLowRateNative4096AndInitializeLargerSliceFitBoundedStorage() throws {
    for rate in [8000.0, 16000.0] {
      let api = UnitAPIFake()
      api.setRate(rate)
      api.enlargedMaximumAtInitialize = 8192
      let delivery = UnitDelivery()
      let engine = try started(api, delivery)
      XCTAssertEqual(engine.inputFormat.sampleRate, rate)
      XCTAssertEqual(api.emit(4096), noErr)
      XCTAssertEqual(api.emit(8192), noErr)
      XCTAssertEqual(delivery.count, 2)
      try engine.stop()
    }
  }
  func testNativeMaximumOverSharedBoundIsRejectedBeforePCMAllocation() throws {
    let api = UnitAPIFake()
    api.setMaximum(96001)
    let engine = api.makeEngine()
    try engine.selectDevice(binding)
    XCTAssertThrowsError(
      try engine.installTap(
        format: engine.inputFormat, handler: { _, _ in XCTFail("not installed") }))
    try engine.stop()
    XCTAssertFalse(api.calls.contains("initialize"))
  }
  func testRetiredNativeTokensIgnoreLateCallbacksEvenAfterAnotherUnitUsesSameHandle() throws {
    let oldAPI = UnitAPIFake()
    let oldDelivery = UnitDelivery()
    let old = try started(oldAPI, oldDelivery)
    let oldCallback = try XCTUnwrap(oldAPI.savedInputCallback())
    let oldNotification = try XCTUnwrap(oldAPI.savedUnitNotification())
    try old.stop()
    let currentAPI = UnitAPIFake()
    let currentDelivery = UnitDelivery()
    let current = try started(currentAPI, currentDelivery)
    defer { try? current.stop() }
    for _ in 0..<20 {
      XCTAssertEqual(oldAPI.replay(oldCallback), noErr)
      oldNotification()
    }
    XCTAssertEqual(oldAPI.renders, 0)
    XCTAssertEqual(currentAPI.renders, 0)
    XCTAssertEqual(oldDelivery.count, 0)
    XCTAssertEqual(currentDelivery.count, 0)
    try current.verifyCallbackDevice(binding)
    XCTAssertEqual(currentAPI.emit(), noErr)
    XCTAssertEqual(currentDelivery.count, 1)
  }
  func testRemoveTapAndStopDrainBorrowedPCMBeforeDisposingUnit() throws {
    let api = UnitAPIFake()
    let delivery = UnitDelivery()
    let engine = try started(api, delivery)
    let entered = DispatchSemaphore(value: 0)
    let release = DispatchSemaphore(value: 0)
    let emitted = DispatchSemaphore(value: 0)
    let stopped = DispatchSemaphore(value: 0)
    api.onRender = {
      entered.signal()
      release.wait()
    }
    DispatchQueue.global().async {
      _ = api.emit()
      emitted.signal()
    }
    XCTAssertEqual(entered.wait(timeout: .now() + 1), .success)
    engine.removeTap()
    DispatchQueue.global().async {
      do { try engine.stop() } catch { XCTFail("stop failed: \(error)") }
      stopped.signal()
    }
    XCTAssertEqual(stopped.wait(timeout: .now() + .milliseconds(30)), .timedOut)
    XCTAssertFalse(api.calls.contains("dispose"))
    release.signal()
    XCTAssertEqual(emitted.wait(timeout: .now() + 1), .success)
    XCTAssertEqual(stopped.wait(timeout: .now() + 1), .success)
    XCTAssertEqual(delivery.count, 0)
    XCTAssertTrue(api.controlsStayedOnOwner)
    XCTAssertTrue(api.callbackRemoved)
  }
  func testFailedNativeStopKeepsOwnerAndRefConUntilProvenRetry() throws {
    let api = UnitAPIFake()
    let engine = try started(api)
    api.failStop = true
    XCTAssertThrowsError(try engine.stop())
    XCTAssertFalse(api.calls.contains("dispose"))
    XCTAssertEqual(api.emit(), noErr)
    XCTAssertThrowsError(try engine.verifyCallbackDevice(binding))
    api.failStop = false
    try engine.stop()
    XCTAssertEqual(api.calls.filter { $0 == "stop" }.count, 2)
    XCTAssertEqual(api.calls.filter { $0 == "dispose" }.count, 1)
    XCTAssertTrue(api.controlsStayedOnOwner)
  }
  func testRejectedCallbackInstallationCleansPartialNativeOwnership() throws {
    let api = UnitAPIFake()
    api.failCallbackInstall = true
    let engine = api.makeEngine()
    try engine.selectDevice(binding)
    XCTAssertThrowsError(
      try engine.installTap(
        format: engine.inputFormat, handler: { _, _ in XCTFail("not installed") }))
    try engine.stop()
    XCTAssertFalse(api.calls.contains("start"))
    XCTAssertEqual(api.calls.filter { $0 == "dispose" }.count, 1)
    XCTAssertTrue(api.controlsStayedOnOwner)
  }
}
