import AVFoundation
import CoreAudio
import Foundation
import XCTest

@testable import MONMacOSCapture

private final class StartupConditions: @unchecked Sendable {
  private let lock = NSLock()
  private var allowed = true
  private var authorization: AVAuthorizationStatus = .authorized
  private var requested = 0
  func mayContinue() -> Bool { lock.withLock { allowed } }
  func requestStop() { lock.withLock { allowed = false } }
  func permission() -> AVAuthorizationStatus { lock.withLock { authorization } }
  func setPermission(_ value: AVAuthorizationStatus) { lock.withLock { authorization = value } }
  func requestedPermission() { lock.withLock { requested += 1 } }
  var requests: Int { lock.withLock { requested } }
}

private final class StartupClock: @unchecked Sendable {
  private let lock = NSLock()
  private var host: Int64 = 1_000_000_000
  func set(_ value: Int64) { lock.withLock { host = value } }
  func read() -> CaptureClockReading {
    lock.withLock {
      CaptureClockReading(
        hostBeforeNs: host, continuousNs: host + 510,
        utcMs: 1_700_000_000_000, hostAfterNs: host + 20)
    }
  }
  func domain() throws -> CaptureClockDomain {
    try CaptureClockDomain(units: MachTimeUnits(numerator: 1, denominator: 1), read: read)
  }
}

private final class StartupFactory: @unchecked Sendable {
  private let lock = NSLock()
  private let engines: [SilentMicrophoneEngine]
  private var count = 0
  init(_ engines: [SilentMicrophoneEngine]) { self.engines = engines }
  func make() throws -> any MicrophoneCaptureEngine {
    try lock.withLock {
      guard count < engines.count else { throw ProbeError.capture("unexpected new engine") }
      let engine = engines[count]
      count += 1
      return engine
    }
  }
  var calls: Int { lock.withLock { count } }
}

/// Synthetic buffers and metadata only. This adapter never creates an
/// AVAudioEngine, Core Audio input unit, OS prompt or hardware recording.
private final class SilentMicrophoneEngine: MicrophoneCaptureEngine, @unchecked Sendable {
  private let lock = NSLock()
  private var format = AVAudioFormat(standardFormatWithSampleRate: 48000, channels: 1)!
  private var running = false
  private var binding: MicrophoneRecovery.Binding?
  private var uid: String?
  private var objectID: AudioObjectID?
  private var observer: (@Sendable () -> Void)?
  private var tap: (@Sendable (AVAudioPCMBuffer, AVAudioTime) -> Void)?
  private var starts = 0, stops = 0, removals = 0, verifies = 0
  private var inNotification = false, controlInNotification = false
  var onPrepare: (@Sendable (SilentMicrophoneEngine) -> Void)?
  var onStart: (@Sendable (SilentMicrophoneEngine, Int) throws -> Void)?
  var onVerify: (@Sendable (SilentMicrophoneEngine, Int) -> Void)?
  var inputFormat: AVAudioFormat { lock.withLock { format } }
  var isRunning: Bool { lock.withLock { running } }
  var startCount: Int { lock.withLock { starts } }
  var stopCount: Int { lock.withLock { stops } }
  var removalCount: Int { lock.withLock { removals } }
  var hasObserver: Bool { lock.withLock { observer != nil } }
  var hasTap: Bool { lock.withLock { tap != nil } }
  var controlledInNotification: Bool { lock.withLock { controlInNotification } }
  var selectedBinding: MicrophoneRecovery.Binding? { lock.withLock { binding } }
  func setFormat(_ value: AVAudioFormat) { lock.withLock { format = value } }
  func setRunning(_ value: Bool) { lock.withLock { running = value } }
  func setIdentity(uid: String, objectID: AudioObjectID) {
    lock.withLock {
      self.uid = uid
      self.objectID = objectID
    }
  }
  func selectDevice(_ binding: MicrophoneRecovery.Binding) throws {
    lock.withLock {
      self.binding = binding
      uid = binding.uid
      objectID = binding.objectID
    }
    try verifyDevice(binding)
  }
  func verifyDevice(_ binding: MicrophoneRecovery.Binding) throws {
    let count = lock.withLock {
      verifies += 1
      return verifies
    }
    onVerify?(self, count)
    try lock.withLock {
      guard uid == binding.uid, objectID == binding.objectID else {
        throw ProbeError.capture("synthetic microphone identity changed")
      }
    }
  }
  func observeConfiguration(_ handler: (@Sendable () -> Void)?) {
    lock.withLock { observer = handler }
  }
  func installTap(
    format: AVAudioFormat, handler: @escaping @Sendable (AVAudioPCMBuffer, AVAudioTime) -> Void
  ) { lock.withLock { tap = handler } }
  func savedTap() -> (@Sendable (AVAudioPCMBuffer, AVAudioTime) -> Void)? {
    lock.withLock { tap }
  }
  func savedObserver() -> (@Sendable () -> Void)? { lock.withLock { observer } }
  func notify(stopped: Bool = false) {
    let handler = lock.withLock {
      if stopped { running = false }
      inNotification = true
      return observer
    }
    handler?()
    lock.withLock { inNotification = false }
  }
  func prepare() {
    lock.withLock { if inNotification { controlInNotification = true } }
    onPrepare?(self)
  }
  func start() throws {
    let count = lock.withLock {
      if inNotification { controlInNotification = true }
      starts += 1
      running = true
      return starts
    }
    try onStart?(self, count)
  }
  func removeTap() {
    lock.withLock {
      if inNotification { controlInNotification = true }
      removals += 1
      tap = nil
    }
  }
  func stop() {
    lock.withLock {
      if inNotification { controlInNotification = true }
      stops += 1
      running = false
    }
  }
  func emit(
    frames: AVAudioFrameCount = 480, sampleTime: AVAudioFramePosition = 0,
    host: UInt64? = nil
  ) throws {
    let callback = savedTap()
    let buffer = try Self.pcm(format: inputFormat, frames: frames)
    let when =
      host.map {
        AVAudioTime(hostTime: $0, sampleTime: sampleTime, atRate: buffer.format.sampleRate)
      } ?? AVAudioTime(sampleTime: sampleTime, atRate: buffer.format.sampleRate)
    callback?(buffer, when)
  }
  static func pcm(format: AVAudioFormat, frames: AVAudioFrameCount = 480) throws
    -> AVAudioPCMBuffer
  {
    let buffer = try XCTUnwrap(AVAudioPCMBuffer(pcmFormat: format, frameCapacity: frames))
    buffer.frameLength = frames
    for channel in 0..<Int(format.channelCount) {
      if let samples = buffer.floatChannelData?[channel] {
        for index in 0..<Int(frames) { samples[index] = 0.125 }
      }
    }
    return buffer
  }
}

final class MicrophoneStartupTests: XCTestCase, @unchecked Sendable {
  private static func devices() -> CaptureInputDevices {
    CaptureInputDevices(
      microphonePermission: "authorized",
      devices: [
        CaptureInputDevice(uid: "other-default", name: "same name", isDefault: true, objectID: 1),
        CaptureInputDevice(uid: "frozen-mic", name: "same name", isDefault: false, objectID: 2),
      ])
  }
  private func directory() throws -> URL {
    let url = FileManager.default.temporaryDirectory.appendingPathComponent(
      "mon-silent-microphone-startup-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: url, withIntermediateDirectories: false)
    return url
  }
  private func recorder(
    _ directory: URL, _ engine: SilentMicrophoneEngine,
    _ conditions: StartupConditions = StartupConditions(), clock: CaptureClockDomain? = nil,
    testWindow: CaptureTestWindow? = nil, factory: StartupFactory? = nil,
    request: (@Sendable () async -> Bool)? = nil
  ) -> MicrophoneRecorder {
    MicrophoneRecorder(
      outputURL: directory.appendingPathComponent("microphone.caf"), clock: clock,
      microphoneUID: "frozen-mic", testWindow: testWindow, mayContinue: conditions.mayContinue,
      makeEngine: { try factory?.make() ?? engine }, devices: Self.devices,
      permission: conditions.permission,
      requestPermission: request ?? {
        conditions.requestedPermission()
        return false
      })
  }
  private func expectFailure(
    _ recorder: MicrophoneRecorder, contains text: String,
    file: StaticString = #filePath, line: UInt = #line
  ) async {
    do {
      try await recorder.start()
      XCTFail("startup unexpectedly succeeded", file: file, line: line)
    } catch {
      XCTAssertTrue(String(describing: error).contains(text), "\(error)", file: file, line: line)
    }
  }
  private func assertClosed(
    _ engine: SilentMicrophoneEngine, _ recorder: MicrophoneRecorder,
    file: StaticString = #filePath, line: UInt = #line
  ) {
    XCTAssertFalse(engine.isRunning, file: file, line: line)
    XCTAssertFalse(engine.hasTap, file: file, line: line)
    XCTAssertFalse(engine.hasObserver, file: file, line: line)
    XCTAssertEqual(recorder.snapshot().microphoneRecovery?.state, "stopped", file: file, line: line)
    XCTAssertEqual(recorder.snapshot().frames, 0, file: file, line: line)
    XCTAssertFalse(engine.controlledInNotification, file: file, line: line)
  }

  func testPreparingNotificationsRequireProofAndDoNotAdmitEarlyPCMOrCountFailure() async throws {
    let dir = try directory()
    defer { try? FileManager.default.removeItem(at: dir) }
    let engine = SilentMicrophoneEngine()
    let recorder = recorder(dir, engine)
    defer { recorder.stop() }
    engine.onPrepare = { source in
      source.notify()
      try? source.emit()
    }
    engine.onStart = { source, _ in
      source.notify()
      try source.emit()
    }
    try await recorder.start()
    XCTAssertEqual(engine.selectedBinding?.uid, "frozen-mic")
    XCTAssertEqual(engine.selectedBinding?.objectID, 2)
    XCTAssertEqual(engine.startCount, 1)
    XCTAssertEqual(recorder.snapshot().microphoneRecovery?.state, "connected")
    XCTAssertEqual(recorder.snapshot().frames, 0)
    XCTAssertEqual(recorder.snapshot().appendFailures, 0)
    try engine.emit()
    recorder.stop()
    XCTAssertEqual(recorder.snapshot().frames, 480)
    XCTAssertEqual(recorder.snapshot().appendFailures, 0)
    XCTAssertEqual(
      try AVAudioFile(forReading: dir.appendingPathComponent("microphone.caf")).length, 480)
    XCTAssertFalse(engine.controlledInNotification)
  }

  func testStartupStoppedByNotificationRetriesOnlySameEngineAndBinding() async throws {
    let dir = try directory()
    defer { try? FileManager.default.removeItem(at: dir) }
    let engine = SilentMicrophoneEngine()
    let recorder = recorder(dir, engine)
    defer { recorder.stop() }
    engine.onStart = { source, count in if count == 1 { source.notify(stopped: true) } }
    try await recorder.start()
    XCTAssertEqual(engine.startCount, 2)
    XCTAssertEqual(engine.selectedBinding?.epoch, 1)
    XCTAssertEqual(recorder.snapshot().microphoneRecovery?.state, "connected")
    XCTAssertEqual(recorder.snapshot().appendFailures, 0)
    XCTAssertFalse(engine.controlledInNotification)
  }

  func testRepeatedStoppedStartupIsBoundedAndCleansUpWithoutClaimingConnection() async throws {
    let dir = try directory()
    defer { try? FileManager.default.removeItem(at: dir) }
    let engine = SilentMicrophoneEngine()
    let recorder = recorder(dir, engine)
    engine.onStart = { source, _ in source.notify(stopped: true) }
    await expectFailure(recorder, contains: "not running after initialization")
    assertClosed(engine, recorder)
    XCTAssertEqual(engine.startCount, 3)
    XCTAssertEqual(engine.removalCount, 1)
    XCTAssertEqual(engine.stopCount, 1)
    recorder.stop()
    XCTAssertEqual(engine.stopCount, 1)
    XCTAssertEqual(engine.removalCount, 1)
  }

  func testNotRunningWithoutNotificationIsNotRetried() async throws {
    let dir = try directory()
    defer { try? FileManager.default.removeItem(at: dir) }
    let engine = SilentMicrophoneEngine()
    let recorder = recorder(dir, engine)
    engine.onStart = { source, _ in source.setRunning(false) }
    await expectFailure(recorder, contains: "not running after initialization")
    assertClosed(engine, recorder)
    XCTAssertEqual(engine.startCount, 1)
  }

  func testUnderlyingInitializationFailureIsRetainedAndResourcesAreClosed() async throws {
    let dir = try directory()
    defer { try? FileManager.default.removeItem(at: dir) }
    let engine = SilentMicrophoneEngine()
    let recorder = recorder(dir, engine)
    engine.onStart = { _, _ in throw ProbeError.capture("synthetic engine OSStatus -10875") }
    await expectFailure(recorder, contains: "synthetic engine OSStatus -10875")
    assertClosed(engine, recorder)
    XCTAssertEqual(recorder.deviceUID, "frozen-mic")
    XCTAssertEqual(recorder.snapshot().appendFailures, 0)
  }

  func testUIDAndObjectChangesNeverActivateOrFallbackToDefault() async throws {
    for (uid, object) in [("other-default", AudioObjectID(2)), ("frozen-mic", AudioObjectID(1))] {
      let dir = try directory()
      defer { try? FileManager.default.removeItem(at: dir) }
      let engine = SilentMicrophoneEngine()
      let recorder = recorder(dir, engine)
      engine.onStart = { source, _ in
        source.setIdentity(uid: uid, objectID: object)
        source.notify()
      }
      await expectFailure(recorder, contains: "identity changed")
      assertClosed(engine, recorder)
      XCTAssertEqual(engine.selectedBinding?.uid, "frozen-mic")
      XCTAssertEqual(engine.selectedBinding?.objectID, 2)
      XCTAssertEqual(engine.startCount, 1)
    }
  }

  func testChangedRateChannelsRepresentationAndInterleavingNeverActivate() async throws {
    let formats = [
      AVAudioFormat(standardFormatWithSampleRate: 44100, channels: 1)!,
      AVAudioFormat(standardFormatWithSampleRate: 48000, channels: 2)!,
      AVAudioFormat(
        commonFormat: .pcmFormatInt16, sampleRate: 48000, channels: 1, interleaved: false)!,
      AVAudioFormat(
        commonFormat: .pcmFormatFloat32, sampleRate: 48000, channels: 1, interleaved: true)!,
    ]
    for format in formats {
      let dir = try directory()
      defer { try? FileManager.default.removeItem(at: dir) }
      let engine = SilentMicrophoneEngine()
      let recorder = recorder(dir, engine)
      engine.onStart = { source, _ in
        source.setFormat(format)
        source.notify()
      }
      await expectFailure(recorder, contains: "formatChanged")
      assertClosed(engine, recorder)
      XCTAssertEqual(engine.startCount, 1)
    }
  }

  func testNotificationDuringIdentityProofRequiresAnotherStableProof() async throws {
    let dir = try directory()
    defer { try? FileManager.default.removeItem(at: dir) }
    let engine = SilentMicrophoneEngine()
    let recorder = recorder(dir, engine)
    defer { recorder.stop() }
    engine.onVerify = { source, count in
      if count == 2 {
        source.notify()
        try? source.emit()
      }
    }
    try await recorder.start()
    XCTAssertEqual(recorder.snapshot().microphoneRecovery?.state, "connected")
    XCTAssertEqual(recorder.snapshot().frames, 0)
    XCTAssertEqual(recorder.snapshot().appendFailures, 0)
  }

  func testContinuouslyChangingProofCannotActivateAndDoesNotLoopForever() async throws {
    let dir = try directory()
    defer { try? FileManager.default.removeItem(at: dir) }
    let engine = SilentMicrophoneEngine()
    let recorder = recorder(dir, engine)
    engine.onVerify = { source, count in if count > 1 { source.notify() } }
    await expectFailure(recorder, contains: "did not stabilize")
    assertClosed(engine, recorder)
    XCTAssertEqual(engine.startCount, 1)
  }

  func testStopRequestedDuringPrepareStartAndProofNeverActivates() async throws {
    for stage in 0..<3 {
      let dir = try directory()
      defer { try? FileManager.default.removeItem(at: dir) }
      let engine = SilentMicrophoneEngine()
      let conditions = StartupConditions()
      let recorder = recorder(dir, engine, conditions)
      if stage == 0 { engine.onPrepare = { _ in conditions.requestStop() } }
      if stage == 1 { engine.onStart = { _, _ in conditions.requestStop() } }
      if stage == 2 { engine.onVerify = { _, count in if count > 1 { conditions.requestStop() } } }
      await expectFailure(recorder, contains: "stop")
      assertClosed(engine, recorder)
      XCTAssertEqual(engine.startCount, stage == 0 ? 0 : 1)
    }
  }

  func testDeniedRestrictedAndUndecidedPermissionKeepRealGuidanceWithoutEngineStart() async throws {
    for (permission, message) in [
      (AVAuthorizationStatus.denied, "許可されていません"),
      (.restricted, "システムの制限"), (.notDetermined, "未決定"),
    ] {
      let dir = try directory()
      defer { try? FileManager.default.removeItem(at: dir) }
      let engine = SilentMicrophoneEngine()
      let conditions = StartupConditions()
      conditions.setPermission(permission)
      let recorder = recorder(dir, engine, conditions)
      await expectFailure(recorder, contains: message)
      XCTAssertEqual(conditions.requests, permission == .notDetermined ? 1 : 0)
      XCTAssertNil(engine.selectedBinding)
      XCTAssertFalse(engine.hasTap)
      XCTAssertFalse(
        FileManager.default.fileExists(atPath: dir.appendingPathComponent("microphone.caf").path))
    }
  }

  func testPermissionRequestReturnValueDoesNotReplaceFreshAuthorizationState() async throws {
    let dir = try directory()
    defer { try? FileManager.default.removeItem(at: dir) }
    let engine = SilentMicrophoneEngine()
    let conditions = StartupConditions()
    conditions.setPermission(.notDetermined)
    let recorder = recorder(
      dir, engine, conditions,
      request: {
        conditions.requestedPermission()
        conditions.setPermission(.denied)
        return true
      })
    await expectFailure(recorder, contains: "許可されていません")
    XCTAssertEqual(conditions.requests, 1)
    XCTAssertNil(engine.selectedBinding)
  }

  func testGrantedPermissionUsesFreshStateAndStillRequiresFullEngineProof() async throws {
    let dir = try directory()
    defer { try? FileManager.default.removeItem(at: dir) }
    let engine = SilentMicrophoneEngine()
    let conditions = StartupConditions()
    conditions.setPermission(.notDetermined)
    let recorder = recorder(
      dir, engine, conditions,
      request: {
        conditions.requestedPermission()
        conditions.setPermission(.authorized)
        return true
      })
    defer { recorder.stop() }
    try await recorder.start()
    XCTAssertEqual(conditions.requests, 1)
    XCTAssertEqual(recorder.snapshot().microphoneRecovery?.state, "connected")
  }

  func testActiveNotificationRejectsPCMAndLateCallbacksAfterStop() async throws {
    let dir = try directory()
    defer { try? FileManager.default.removeItem(at: dir) }
    let engine = SilentMicrophoneEngine()
    let recorder = recorder(dir, engine)
    try await recorder.start()
    try engine.emit()
    let oldTap = try XCTUnwrap(engine.savedTap())
    let oldObserver = try XCTUnwrap(engine.savedObserver())
    engine.notify(stopped: true)
    try engine.emit(sampleTime: 480)
    XCTAssertEqual(recorder.snapshot().frames, 480)
    XCTAssertEqual(recorder.snapshot().appendFailures, 1)
    recorder.stop()
    let snapshot = recorder.snapshot()
    oldObserver()
    oldTap(
      try SilentMicrophoneEngine.pcm(format: engine.inputFormat),
      AVAudioTime(sampleTime: 480, atRate: 48000))
    XCTAssertEqual(recorder.snapshot().frames, snapshot.frames)
    XCTAssertEqual(recorder.snapshot().appendFailures, snapshot.appendFailures)
    XCTAssertEqual(recorder.snapshot().microphoneRecovery, snapshot.microphoneRecovery)
    XCTAssertEqual(
      try AVAudioFile(forReading: dir.appendingPathComponent("microphone.caf")).length, 480)
  }

  func testStartupStatusReadDoesNotWaitForEngineIdentityControl() async throws {
    let dir = try directory()
    defer { try? FileManager.default.removeItem(at: dir) }
    let engine = SilentMicrophoneEngine()
    let recorder = recorder(dir, engine)
    defer {
      engine.onVerify = nil
      recorder.stop()
    }
    engine.onVerify = { [weak recorder] _, count in
      guard count == 2, let recorder else { return }
      let completed = DispatchSemaphore(value: 0)
      DispatchQueue.global().async {
        XCTAssertEqual(recorder.snapshot().microphoneRecovery?.state, "reconnecting")
        completed.signal()
      }
      XCTAssertEqual(
        completed.wait(timeout: .now() + 1), .success,
        "status getter was blocked by engine verification")
    }
    try await recorder.start()
    XCTAssertEqual(recorder.snapshot().microphoneRecovery?.state, "connected")
  }

  func testCallbackIdentityReadDoesNotBlockStatusOrDeadlockOnConfigurationNotification()
    async throws
  {
    let dir = try directory()
    defer { try? FileManager.default.removeItem(at: dir) }
    let engine = SilentMicrophoneEngine()
    let recorder = recorder(dir, engine)
    defer {
      engine.onVerify = nil
      recorder.stop()
    }
    try await recorder.start()
    engine.onVerify = { [weak recorder] source, count in
      guard count == 3, let recorder else { return }
      let completed = DispatchSemaphore(value: 0)
      DispatchQueue.global().async {
        XCTAssertEqual(recorder.snapshot().microphoneRecovery?.state, "connected")
        completed.signal()
      }
      XCTAssertEqual(completed.wait(timeout: .now() + 1), .success)
      source.notify(stopped: true)
    }
    try engine.emit()
    XCTAssertEqual(recorder.snapshot().frames, 0)
    XCTAssertEqual(recorder.snapshot().appendFailures, 1)
    XCTAssertFalse(engine.controlledInNotification)
  }

  func testFiveSecondWindowUsesNormalStartupProofWithoutAdmittingPreparationAudio() async throws {
    let dir = try directory()
    defer { try? FileManager.default.removeItem(at: dir) }
    let time = StartupClock()
    let engine = SilentMicrophoneEngine()
    let window = try CaptureTestWindow(ownerId: "735027d4-a03e-43d3-830a-21e767dfaf88")
    let recorder = recorder(dir, engine, clock: try time.domain(), testWindow: window)
    defer { recorder.stop() }
    engine.onPrepare = { source in
      source.notify()
      try? source.emit(host: 1_000_000_000)
    }
    engine.onStart = { source, _ in
      source.notify()
      try source.emit(host: 1_000_000_000)
    }
    try await recorder.start()
    XCTAssertEqual(recorder.snapshot().frames, 0)
    XCTAssertEqual(recorder.snapshot().appendFailures, 0)
    try engine.emit(host: 1_000_000_000)
    XCTAssertEqual(recorder.snapshot().frames, 0, "closed test window admitted PCM")
    try window.open(at: 1_000_000_500)
    try engine.emit(host: 1_000_000_000)
    XCTAssertEqual(recorder.snapshot().frames, 480)
    engine.notify(stopped: true)
    try engine.emit(sampleTime: 480, host: 1_000_000_000)
    XCTAssertEqual(recorder.snapshot().frames, 480)
    XCTAssertEqual(recorder.snapshot().appendFailures, 1)
    XCTAssertThrowsError(try recorder.snapshot().requireWritable()) { error in
      XCTAssertTrue(String(describing: error).contains("microphone_configuration_changed"))
    }
    XCTAssertFalse(window.metadata().completed)
    XCTAssertFalse(engine.controlledInNotification)
  }

  func testTapPreparationBeforeMicrophoneDoesNotBecomeAnActiveTestWriteFailure() async throws {
    for tapFirst in [false, true] {
      let dir = try directory()
      defer { try? FileManager.default.removeItem(at: dir) }
      let time = StartupClock()
      let engine = SilentMicrophoneEngine()
      let window = try CaptureTestWindow(ownerId: "735027d4-a03e-43d3-830a-21e767dfaf88")
      let recorder = recorder(dir, engine, clock: try time.domain(), testWindow: window)
      defer { recorder.stop() }
      // Emulate only the configuration notification induced by a new tap
      // aggregate. This does not create hardware, OS taps or permission prompts.
      if tapFirst { engine.notify(stopped: true) }
      try await recorder.start()
      if !tapFirst { engine.notify(stopped: true) }
      XCTAssertEqual(recorder.snapshot().frames, 0)
      try window.open(at: 1_000_000_500)
      try engine.emit(host: 1_000_000_000)
      if tapFirst {
        XCTAssertEqual(recorder.snapshot().appendFailures, 0)
        XCTAssertEqual(recorder.snapshot().frames, 480)
        XCTAssertEqual(recorder.deviceUID, "frozen-mic")
        XCTAssertNoThrow(try recorder.snapshot().requireWritable())
      } else {
        XCTAssertEqual(recorder.snapshot().frames, 0)
        XCTAssertEqual(recorder.snapshot().appendFailures, 1)
        XCTAssertTrue(
          recorder.snapshot().captureFailureDetail?.contains("microphone_configuration_changed")
            == true)
      }
      XCTAssertFalse(engine.controlledInNotification)
    }
  }

  func testNormalRebindPreservesCAFAndRetiresOldPCMAndNotificationEpochs() async throws {
    let dir = try directory()
    defer { try? FileManager.default.removeItem(at: dir) }
    let old = SilentMicrophoneEngine()
    let current = SilentMicrophoneEngine()
    let time = StartupClock()
    let factory = StartupFactory([old, current])
    let recorder = recorder(dir, old, clock: try time.domain(), factory: factory)
    defer { recorder.stop() }
    try await recorder.start()
    try old.emit(host: 1_000_000_000)
    let latePCM = try XCTUnwrap(old.savedTap())
    let lateNotification = try XCTUnwrap(old.savedObserver())
    old.notify(stopped: true)
    time.set(1_030_000_000)
    recorder.maintain()
    XCTAssertEqual(factory.calls, 2)
    XCTAssertEqual(recorder.snapshot().microphoneRecovery?.state, "connected")
    XCTAssertEqual(current.selectedBinding?.epoch, 2)
    XCTAssertEqual(recorder.snapshot().frames, 480)
    lateNotification()
    latePCM(
      try SilentMicrophoneEngine.pcm(format: old.inputFormat),
      AVAudioTime(hostTime: 1_000_000_000, sampleTime: 480, atRate: 48000))
    XCTAssertEqual(recorder.snapshot().frames, 480)
    XCTAssertEqual(recorder.snapshot().appendFailures, 0)
    try current.emit(host: 1_030_000_000)
    time.set(1_050_000_000)
    recorder.stop()
    XCTAssertEqual(recorder.snapshot().frames, 960)
    XCTAssertEqual(recorder.snapshot().appendFailures, 0)
    XCTAssertNil(recorder.snapshot().captureFailureDetail)
    let file = try AVAudioFile(forReading: dir.appendingPathComponent("microphone.caf"))
    XCTAssertEqual(file.length, 960)
    let buffer = try XCTUnwrap(
      AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: 960))
    try file.read(into: buffer)
    XCTAssertEqual(buffer.frameLength, 960)
    XCTAssertTrue((0..<960).allSatisfy { buffer.floatChannelData![0][$0] == 0.125 })
    XCTAssertEqual(old.removalCount, 1)
    XCTAssertEqual(old.stopCount, 1)
  }

  func testChangedRebindFormatRetainsExistingCAFAndCannotBecomeConnected() async throws {
    let dir = try directory()
    defer { try? FileManager.default.removeItem(at: dir) }
    let old = SilentMicrophoneEngine()
    let changed = SilentMicrophoneEngine()
    let time = StartupClock()
    changed.setFormat(AVAudioFormat(standardFormatWithSampleRate: 44100, channels: 1)!)
    let factory = StartupFactory([old, changed])
    let recorder = recorder(dir, old, clock: try time.domain(), factory: factory)
    defer { recorder.stop() }
    try await recorder.start()
    try old.emit(host: 1_000_000_000)
    old.notify(stopped: true)
    recorder.maintain()
    XCTAssertEqual(factory.calls, 2)
    XCTAssertEqual(recorder.snapshot().microphoneRecovery?.state, "unavailable")
    XCTAssertEqual(recorder.snapshot().microphoneRecovery?.reason, "format_changed")
    XCTAssertEqual(recorder.snapshot().frames, 480)
    XCTAssertFalse(changed.hasTap)
    XCTAssertFalse(changed.hasObserver)
    XCTAssertEqual(changed.startCount, 0)
    XCTAssertEqual(changed.stopCount, 1)
    recorder.stop()
    XCTAssertEqual(
      try AVAudioFile(forReading: dir.appendingPathComponent("microphone.caf")).length, 480)
    XCTAssertEqual(recorder.snapshot().appendFailures, 0)
  }
}
