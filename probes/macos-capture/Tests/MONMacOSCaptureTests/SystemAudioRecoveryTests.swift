import AVFoundation
import CoreMedia
import Foundation
import ScreenCaptureKit
import XCTest

@testable import MONMacOSCapture

private final class SystemTestTime: @unchecked Sendable {
  private let lock = NSLock()
  private var time: Int64 = 1_000_000_000
  private var allowed = true
  func read() -> Int64? { lock.withLock { time } }
  func set(_ value: Int64) { lock.withLock { time = value } }
  func mayContinue() -> Bool { lock.withLock { allowed } }
  func requestStop() { lock.withLock { allowed = false } }
}

private final class SilentSystemSource: SystemCaptureStream, @unchecked Sendable {
  let native: SCStream? = nil
  let applicationName: String? = "confirmed application"
  let bundleIdentifier: String? = "com.example.confirmed"
  private let lock = NSLock()
  private var held: Bool
  private var continuation: CheckedContinuation<Void, any Error>?
  private var starts = 0
  private var stops = 0
  private var detachments = 0
  private var stopAction: (@Sendable () -> Void)?
  private var stopCompletion: (@Sendable (Error?) -> Void)?
  private let holdStop: Bool
  private let startFailure: Bool
  private let stopFailure: Bool
  private var detachFailure: Bool
  init(
    held: Bool = false, startFailure: Bool = false, stopFailure: Bool = false,
    holdStop: Bool = false, detachFailure: Bool = false
  ) {
    self.held = held
    self.startFailure = startFailure
    self.stopFailure = stopFailure
    self.holdStop = holdStop
    self.detachFailure = detachFailure
  }
  var counts: (Int, Int, Int) { lock.withLock { (starts, stops, detachments) } }
  func start() async throws {
    try await withCheckedThrowingContinuation {
      (continuation: CheckedContinuation<Void, any Error>) in
      let wait = lock.withLock {
        starts += 1
        if held {
          self.continuation = continuation
          return true
        }
        return false
      }
      if !wait {
        if startFailure {
          continuation.resume(throwing: ProbeError.capture("synthetic start failure"))
        } else {
          continuation.resume()
        }
      }
    }
  }
  func release() {
    let pending = lock.withLock {
      held = false
      let pending = continuation
      continuation = nil
      return pending
    }
    pending?.resume()
  }
  func stop() async throws {
    lock.withLock { stops += 1 }
    if stopFailure { throw ProbeError.capture("synthetic stop failure") }
  }
  func requestStop(_ completion: @escaping @Sendable (Error?) -> Void) {
    let action = lock.withLock {
      stops += 1
      let action = stopAction
      stopAction = nil
      if holdStop { stopCompletion = completion }
      return action
    }
    action?()
    if !holdStop { completion(stopFailure ? ProbeError.capture("synthetic stop failure") : nil) }
  }
  func releaseStop() {
    let completion = lock.withLock {
      let completion = stopCompletion
      stopCompletion = nil
      return completion
    }
    completion?(stopFailure ? ProbeError.capture("synthetic stop failure") : nil)
  }
  func onStop(_ action: @escaping @Sendable () -> Void) { lock.withLock { stopAction = action } }
  func setDetachFailure(_ value: Bool) { lock.withLock { detachFailure = value } }
  func detach() throws {
    try lock.withLock {
      detachments += 1
      if detachFailure { throw ProbeError.capture("synthetic output detach failure") }
    }
  }
}

private final class SilentApplicationIdentity: @unchecked Sendable {
  private let lock = NSLock()
  private var current: CaptureApplicationIdentity
  init(_ identity: CaptureApplicationIdentity) { current = identity }
  func set(_ identity: CaptureApplicationIdentity) { lock.withLock { current = identity } }
  func require(_ identity: CaptureApplicationIdentity) throws {
    guard lock.withLock({ current == identity }) else {
      throw ProbeError.capture("synthetic PID reuse")
    }
  }
}

private final class SilentSystemFactory: @unchecked Sendable {
  private let lock = NSLock()
  let sources: [SilentSystemSource]
  private var scopes: [(pid_t?, CaptureApplicationIdentity?)] = []
  init(_ sources: [SilentSystemSource]) { self.sources = sources }
  var calls: Int { lock.withLock { scopes.count } }
  var observedScopes: [(pid_t?, CaptureApplicationIdentity?)] { lock.withLock { scopes } }
  func make(_ recorder: SystemAudioRecorder, _ token: SystemAudioRecovery.Binding) throws
    -> any SystemCaptureStream
  {
    let scope = recorder.confirmedScope()
    return try lock.withLock {
      let index = scopes.count
      guard index < sources.count else { throw ProbeError.capture("unexpected additional source") }
      scopes.append(scope)
      return sources[index]
    }
  }
}

final class SystemAudioRecoveryTests: XCTestCase, @unchecked Sendable {
  private func sleepReading(_ host: Int64, offset: Int64 = 500) -> CaptureClockReading {
    CaptureClockReading(
      hostBeforeNs: host, continuousNs: host + offset + 10,
      utcMs: 1_700_000_000_000, hostAfterNs: host + 20)
  }

  func testClockRecoveryWaitsForActualStopAndResumesOnlyNewConfirmedStreamWithoutChangingCAF()
    async throws
  {
    let output = try directory()
    defer { try? FileManager.default.removeItem(at: output) }
    let source = SilentSystemSource(holdStop: true)
    let current = SilentSystemSource()
    let factory = SilentSystemFactory([source, current])
    let time = SystemTestTime()
    let box = ReadingBox(sleepReading(1_000_000_000))
    let clock = try CaptureClockDomain(
      units: MachTimeUnits(numerator: 1, denominator: 1), allowsRecovery: true
    ) { box.read() }
    let recorder = SystemAudioRecorder(
      outputURL: output.appendingPathComponent("system.caf"), clock: clock,
      levelsEnabled: true, retries: true, factory: { try factory.make($0, $1) },
      now: time.read, requireIdentity: { _ in }, mayContinue: time.mayContinue)
    try await recorder.start(targetPID: identity.pid, applicationIdentity: identity)
    let micURL = output.appendingPathComponent("microphone.caf")
    let micFormat = try XCTUnwrap(AVAudioFormat(standardFormatWithSampleRate: 48000, channels: 2))
    var micFile: AVAudioFile? = try AVAudioFile(
      forWriting: micURL, settings: micFormat.settings,
      commonFormat: .pcmFormatFloat32, interleaved: false)
    let micBuffer = try XCTUnwrap(AVAudioPCMBuffer(pcmFormat: micFormat, frameCapacity: 480))
    micBuffer.frameLength = 480
    for frame in 0..<480 {
      micBuffer.floatChannelData![0][frame] = 0.125
      micBuffer.floatChannelData![1][frame] = -0.125
    }
    var micStats = TrackStats(frames: 960, sampleRate: 48000, channels: 2)
    box.set(sleepReading(1_020_000_000))
    let firstObservation = clock.observe(hostTicks: 1_010_000_000)
    recorder.receive(
      from: source, sampleBuffer: try sample(), observation: firstObservation
    )
    micStats.recordSavedClock(firstObservation, frames: 480, sourceFrame: 0, sourceEpoch: 1)
    try micFile?.write(from: micBuffer)
    let before = recorder.snapshot()
    XCTAssertEqual(before.frames, 480)
    box.set(sleepReading(1_030_000_000, offset: 10_000_000_500))
    let request = try XCTUnwrap(clock.recoveryRequest())
    recorder.receive(from: source, sampleBuffer: try sample(value: 9), observation: .suspended)
    XCTAssertEqual(recorder.snapshot().frames, 480)
    XCTAssertEqual(recorder.snapshot().appendFailures, 0)
    XCTAssertEqual(recorder.snapshot().systemRecovery?.reason, "clock_changed")
    let mic = try clock.quiesced(.microphone, for: request)
    let retirement = Task { try await recorder.suspendForClockRecovery(request) }
    try await wait { source.counts.1 == 1 }
    XCTAssertTrue(recorder.acquisitionPending)
    for _ in 0..<10 { recorder.maintain() }
    XCTAssertEqual(factory.calls, 1)
    XCTAssertEqual(source.counts.2, 0)
    XCTAssertThrowsError(try recorder.resumeAfterClockRecovery(request))
    recorder.receive(
      from: source, sampleBuffer: try sample(value: 9), observation: .timestamp(11_050_000_500))
    XCTAssertEqual(recorder.snapshot().frames, 480)
    source.releaseStop()
    let system = try await retirement.value
    XCTAssertFalse(recorder.acquisitionPending)
    XCTAssertEqual(source.counts.2, 1)
    for _ in 0..<10 { recorder.maintain() }
    XCTAssertEqual(factory.calls, 1)
    box.set(sleepReading(1_040_000_000, offset: 10_000_000_500))
    try clock.resume(request, microphone: mic, system: system)
    time.set(12_000_000_000)
    try recorder.resumeAfterClockRecovery(request)
    try await wait { !recorder.acquisitionPending }
    XCTAssertEqual(factory.calls, 2)
    XCTAssertEqual(factory.observedScopes.map(\.0), [identity.pid, identity.pid])
    XCTAssertEqual(factory.observedScopes.map(\.1), [identity, identity])
    recorder.sourceEnded(source)
    recorder.receive(from: source, sampleBuffer: try sample(value: 9), observation: .invalid("old"))
    try await wait { recorder.readyForPCM }
    box.set(sleepReading(1_060_000_000, offset: 10_000_000_500))
    let secondObservation = clock.observe(hostTicks: 1_050_000_000)
    recorder.receive(
      from: current, sampleBuffer: try sample(value: 0.5),
      observation: secondObservation)
    micStats.recordSavedClock(secondObservation, frames: 480, sourceFrame: 0, sourceEpoch: 2)
    try micFile?.write(from: micBuffer)
    micFile = nil
    let closure = try await recorder.stopRecording()
    let micClosure = try clock.close(.microphone)
    var stats = recorder.snapshot()
    try stats.finishClock(originNanoseconds: 1_010_000_500)
    XCTAssertEqual(stats.frames, 960)
    XCTAssertEqual(stats.appendFailures, 0)
    XCTAssertNil(stats.captureFailure)
    XCTAssertEqual(stats.clockSegments?.count, 2)
    XCTAssertEqual(stats.clockSegments?[1].timelineStartUs, 10_040_000)
    XCTAssertEqual(stats.systemRecovery?.sourceEpoch, 2)
    XCTAssertEqual(stats.systemRecovery?.interruptions, 1)
    XCTAssertEqual(stats.inputEventHistory?.events.map(\.kind), ["interrupted", "recovered"])
    XCTAssertEqual(stats.inputEventHistory?.events.first?.status.reason, "clock_changed")
    XCTAssertTrue(stats.inputEventHistory!.matches(stats.systemRecovery!, input: "system"))
    let (_, ending) = try clock.ending(
      originNanoseconds: 1_010_000_500, microphone: micClosure, system: closure)
    XCTAssertEqual(ending.endUs, 10_050_000)
    let saved = try AVAudioFile(forReading: output.appendingPathComponent("system.caf"))
    XCTAssertEqual(saved.length, 960)
    let pcm = try XCTUnwrap(AVAudioPCMBuffer(pcmFormat: saved.processingFormat, frameCapacity: 960))
    try saved.read(into: pcm)
    XCTAssertEqual(pcm.frameLength, 960)
    let values = try XCTUnwrap(pcm.floatChannelData)
    for frame in 0..<960 {
      let expected: Float = frame < 480 ? 0.25 : 0.5
      XCTAssertEqual(values[0][frame], expected)
      XCTAssertEqual(values[1][frame], -expected)
    }
    var manifest = Manifest(
      schemaVersion: 2, scenario: "synthetic-clock-recovery", captureMode: "application",
      backend: "screencapturekit", targetPID: identity.pid, targetPIDs: [identity.pid],
      startedAtUTC: "2023-11-14T22:13:20Z", requestedDurationSeconds: nil,
      osVersion: "synthetic", architecture: "arm64", state: "saved",
      microphone: micStats, system: recorder.snapshot(), errors: [])
    manifest.targetApplicationIdentity = identity
    manifest.recordingOwnerId =
      ProcessInfo.processInfo.environment["MON_CLOCK_RECOVERY_FIXTURE_OWNER"]
      ?? "735027d4-a03e-43d3-830a-21e767dfaf88"
    try manifest.finalizeStoppedClock(
      clock, microphoneClosure: micClosure, systemClosure: closure)
    XCTAssertEqual(manifest.captureClock?.originUtcMs, 1_700_000_000_009)
    try verifyRecordedCAF(micURL, expected: manifest.microphone)
    try verifyRecordedCAF(output.appendingPathComponent("system.caf"), expected: manifest.system)
    try manifest.encodedData().write(to: output.appendingPathComponent("manifest.json"))
    if let path = ProcessInfo.processInfo.environment["MON_CLOCK_RECOVERY_FIXTURE_DIR"] {
      let destination = URL(fileURLWithPath: path, isDirectory: true)
      XCTAssertTrue(try FileManager.default.contentsOfDirectory(atPath: path).isEmpty)
      for name in ["manifest.json", "microphone.caf", "system.caf"] {
        try FileManager.default.copyItem(
          at: output.appendingPathComponent(name), to: destination.appendingPathComponent(name))
      }
    }
  }

  func testClockRecoveryStopFailureCannotProduceQuiescenceOrStartReplacement() async throws {
    let output = try directory()
    defer { try? FileManager.default.removeItem(at: output) }
    let source = SilentSystemSource(stopFailure: true)
    let factory = SilentSystemFactory([source, SilentSystemSource()])
    let time = SystemTestTime()
    let box = ReadingBox(sleepReading(1_000_000_000))
    let clock = try CaptureClockDomain(
      units: MachTimeUnits(numerator: 1, denominator: 1), allowsRecovery: true
    ) { box.read() }
    let recorder = SystemAudioRecorder(
      outputURL: output.appendingPathComponent("system.caf"), clock: clock,
      retries: true, factory: { try factory.make($0, $1) }, now: time.read)
    try await recorder.start(targetPID: nil)
    recorder.receive(
      from: source, sampleBuffer: try sample(), observation: .timestamp(1_010_000_500))
    box.set(sleepReading(1_030_000_000, offset: 10_000_000_500))
    let request = try XCTUnwrap(clock.recoveryRequest())
    do {
      _ = try await recorder.suspendForClockRecovery(request)
      XCTFail("failed OS stop minted quiescence")
    } catch {}
    for _ in 0..<10 { recorder.maintain() }
    XCTAssertEqual(factory.calls, 1)
    XCTAssertEqual(source.counts.1, 1)
    XCTAssertEqual(source.counts.2, 0)
    XCTAssertThrowsError(try recorder.resumeAfterClockRecovery(request))
    do {
      _ = try await recorder.stopRecording()
      XCTFail("failed stop minted ending")
    } catch {}
    XCTAssertEqual(source.counts.1, 1)  // Reuse the actual stop result, not a second OS request.
    XCTAssertEqual(
      try AVAudioFile(forReading: output.appendingPathComponent("system.caf")).length, 480)
  }

  func testClockRecoveryTimeoutKeepsSingleStopRequestAndLateResultCannotResume() async throws {
    let output = try directory()
    defer { try? FileManager.default.removeItem(at: output) }
    let source = SilentSystemSource(holdStop: true)
    let factory = SilentSystemFactory([source, SilentSystemSource()])
    let time = SystemTestTime()
    let box = ReadingBox(sleepReading(1_000_000_000))
    let clock = try CaptureClockDomain(
      units: MachTimeUnits(numerator: 1, denominator: 1), allowsRecovery: true
    ) { box.read() }
    let recorder = SystemAudioRecorder(
      outputURL: output.appendingPathComponent("system.caf"), clock: clock,
      retries: true, factory: { try factory.make($0, $1) }, now: time.read)
    try await recorder.start(targetPID: nil)
    recorder.receive(
      from: source, sampleBuffer: try sample(), observation: .timestamp(1_010_000_500))
    box.set(sleepReading(1_030_000_000, offset: 10_000_000_500))
    let request = try XCTUnwrap(clock.recoveryRequest())
    do {
      _ = try await recorder.suspendForClockRecovery(request)
      XCTFail("unresponsive stop minted quiescence")
    } catch {}
    XCTAssertEqual(source.counts.1, 1)
    XCTAssertEqual(source.counts.2, 0)
    time.set(12_000_000_000)
    do {
      _ = try await recorder.suspendForClockRecovery(request)
      XCTFail("timed-out OS stop was requested a second time")
    } catch {}
    for _ in 0..<10 { recorder.maintain() }
    XCTAssertEqual(factory.calls, 1)
    source.releaseStop()
    XCTAssertThrowsError(try recorder.resumeAfterClockRecovery(request))
    recorder.receive(from: source, sampleBuffer: try sample(value: 9), observation: .suspended)
    XCTAssertEqual(recorder.snapshot().frames, 480)
    do {
      _ = try await recorder.stopRecording()
      XCTFail("unproven clock minted ending")
    } catch {}
    XCTAssertEqual(source.counts.1, 1)
    XCTAssertEqual(
      try AVAudioFile(forReading: output.appendingPathComponent("system.caf")).length, 480)
  }

  func testClockRecoveryWaitsForOwnedLateAcquisitionAndNeverAdoptsItsPCM() async throws {
    let output = try directory()
    defer { try? FileManager.default.removeItem(at: output) }
    let source = SilentSystemSource()
    let held = SilentSystemSource(held: true)
    let current = SilentSystemSource()
    let factory = SilentSystemFactory([source, held, current])
    let time = SystemTestTime()
    let box = ReadingBox(sleepReading(1_000_000_000))
    let clock = try CaptureClockDomain(
      units: MachTimeUnits(numerator: 1, denominator: 1), allowsRecovery: true
    ) { box.read() }
    let recorder = SystemAudioRecorder(
      outputURL: output.appendingPathComponent("system.caf"), clock: clock,
      retries: true, factory: { try factory.make($0, $1) }, now: time.read)
    try await recorder.start(targetPID: nil)
    recorder.receive(
      from: source, sampleBuffer: try sample(), observation: .timestamp(1_010_000_500))
    recorder.sourceEnded(source)
    time.set(2_000_000_000)
    recorder.maintain()
    try await wait { held.counts.0 == 1 }
    box.set(sleepReading(1_030_000_000, offset: 10_000_000_500))
    let request = try XCTUnwrap(clock.recoveryRequest())
    let retirement = Task { try await recorder.suspendForClockRecovery(request) }
    try await wait { recorder.snapshot().systemRecovery?.reason == "clock_changed" }
    XCTAssertTrue(recorder.acquisitionPending)
    time.set(12_000_000_000)
    for _ in 0..<10 { recorder.maintain() }
    XCTAssertEqual(factory.calls, 2)
    recorder.receive(from: held, sampleBuffer: try sample(value: 9), observation: .suspended)
    held.release()
    let system = try await retirement.value
    XCTAssertEqual(held.counts.1, 1)
    XCTAssertEqual(held.counts.2, 1)
    XCTAssertEqual(recorder.snapshot().frames, 480)
    box.set(sleepReading(1_040_000_000, offset: 10_000_000_500))
    try clock.resume(
      request, microphone: clock.quiesced(.microphone, for: request), system: system)
    try recorder.resumeAfterClockRecovery(request)
    try await wait { !recorder.acquisitionPending }
    XCTAssertEqual(factory.calls, 3)
    XCTAssertEqual(recorder.snapshot().systemRecovery?.sourceEpoch, 3)
    XCTAssertEqual(recorder.snapshot().systemRecovery?.interruptions, 1)
    recorder.receive(
      from: held, sampleBuffer: try sample(value: 9), observation: .invalid("obsolete"))
    XCTAssertEqual(recorder.snapshot().frames, 480)
    _ = try await recorder.stopRecording()
    XCTAssertEqual(
      try AVAudioFile(forReading: output.appendingPathComponent("system.caf")).length, 480)
  }

  func testTerminalClockFailureRejectsPCMAndReconnectAndCannotMintStopClosure() async throws {
    let output = try directory()
    defer { try? FileManager.default.removeItem(at: output) }
    let source = SilentSystemSource()
    let returned = SilentSystemSource()
    let factory = SilentSystemFactory([source, returned])
    let time = SystemTestTime()
    let recorder = try recorder(output, factory, time)
    try await recorder.start(targetPID: nil)
    recorder.receive(
      from: source, sampleBuffer: try sample(), observation: .timestamp(1_000_000_000))
    let frames = recorder.snapshot().frames
    recorder.receive(
      from: source, sampleBuffer: try sample(), observation: .invalid("missing hardware timestamp"))
    XCTAssertEqual(recorder.snapshot().frames, frames)
    XCTAssertEqual(recorder.snapshot().captureFailure, "capture_clock_failed")
    XCTAssertFalse(recorder.readyForPCM)
    recorder.receive(
      from: source, sampleBuffer: try sample(), observation: .timestamp(1_010_000_000))
    XCTAssertEqual(recorder.snapshot().frames, frames)
    recorder.sourceEnded(source)
    time.set(2_000_000_000)
    for _ in 0..<10 { recorder.maintain() }
    XCTAssertEqual(factory.calls, 1)
    for _ in 0..<2 {
      do {
        _ = try await recorder.stopRecording()
        XCTFail("failed clock cannot mint a closure")
      } catch {}
    }
    let file = try AVAudioFile(forReading: output.appendingPathComponent("system.caf"))
    XCTAssertEqual(file.length, frames)
  }

  func testNormalRecorderCommitsLifecycleHistoryWithoutHeartbeatOrRealStream() async throws {
    let temporary = try directory()
    defer { try? FileManager.default.removeItem(at: temporary) }
    let canonical = try XCTUnwrap(realpath(temporary.path, nil))
    defer { free(canonical) }
    let output = URL(fileURLWithPath: String(cString: canonical), isDirectory: true)
    let journal = try InputEventJournal(
      output: output, owner: "735027d4-a03e-43d3-830a-21e767dfaf88", supportsSystem: true)
    let source = SilentSystemSource()
    let returned = SilentSystemSource()
    let factory = SilentSystemFactory([source, returned])
    let time = SystemTestTime()
    let clock = try CaptureClockDomain(units: MachTimeUnits(numerator: 1, denominator: 1)) {
      CaptureClockReading(hostBeforeNs: 1, continuousNs: 2, utcMs: 1000, hostAfterNs: 3)
    }
    let recorder = SystemAudioRecorder(
      outputURL: output.appendingPathComponent("system.caf"), clock: clock, retries: true,
      factory: { recorder, token in try factory.make(recorder, token) }, now: { time.read() },
      inputEventObserver: journal.observer(.system))
    try await recorder.start(targetPID: nil)
    recorder.receive(
      from: source, sampleBuffer: try sample(), observation: .timestamp(1_000_000_000))
    recorder.sourceEnded(source)
    time.set(2_000_000_000)
    recorder.maintain()
    let deadline = ContinuousClock.now.advanced(by: .seconds(3))
    while recorder.snapshot().systemRecovery?.state != "connected", ContinuousClock.now < deadline {
      try await Task.sleep(for: .milliseconds(10))
    }
    XCTAssertEqual(factory.calls, 2)
    XCTAssertEqual(recorder.snapshot().systemRecovery?.state, "connected")
    try await recorder.stop()
    try journal.finish()
    let value = try XCTUnwrap(
      JSONSerialization.jsonObject(
        with: Data(contentsOf: output.appendingPathComponent("input-events.json")))
        as? [String: Any])
    let report = try XCTUnwrap(value["report"] as? [String: Any])
    let history = try XCTUnwrap(report["systemHistory"] as? [String: Any])
    XCTAssertEqual(history["lastSequence"] as? Int, 2)
    XCTAssertEqual((report["system"] as? [String: Any])?["state"] as? String, "stopped")
    XCTAssertFalse(
      FileManager.default.fileExists(
        atPath: output.appendingPathComponent("input-status.json").path))
  }

  func testInterruptedStopRecordingExportsOwnerBoundEndAndUnchangedPhysicalCAF() async throws {
    let directory = try directory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let read = { (host: Int64) in
      CaptureClockReading(
        hostBeforeNs: host, continuousNs: host + 10,
        utcMs: 1_700_000_000_000, hostAfterNs: host + 20)
    }
    let box = ReadingBox(read(1_000_000_000))
    let clock = try CaptureClockDomain(units: MachTimeUnits(numerator: 1, denominator: 1)) {
      box.read()
    }
    let source = SilentSystemSource()
    let recorder = SystemAudioRecorder(
      outputURL: directory.appendingPathComponent("system.caf"),
      clock: clock, retries: true, factory: { _, _ in source })
    try await recorder.start(targetPID: nil)
    box.set(read(1_020_000_000))
    recorder.receive(
      from: source, sampleBuffer: try sample(rightValue: 0.125),
      observation: clock.observe(hostTicks: 1_005_000_000)
    )
    let fixture = CaptureClockFixtureTests()
    var mic = try await fixture.writeCAF(
      [
        fixture.sample(
          frames: 480, value: 0.125,
          presentation: CMTime(value: 1, timescale: 1), rightValue: 0.0625)
      ], to: directory.appendingPathComponent("microphone.caf"))
    mic.recordSavedClock(clock.observe(hostTicks: 1_000_000_000), frames: 480, sourceFrame: 0)
    recorder.sourceEnded(source)
    XCTAssertEqual(recorder.snapshot().systemRecovery?.state, "disconnected")
    let micClosure = try clock.close(.microphone)
    box.set(read(1_050_000_000))
    let pcClosure = try await recorder.stopRecording()
    let replay = try await recorder.stopRecording()
    XCTAssertEqual(replay, pcClosure)
    let system = recorder.snapshot()
    XCTAssertEqual(system.frames, 480)
    try verifyRecordedCAF(directory.appendingPathComponent("system.caf"), expected: system)
    let owner =
      ProcessInfo.processInfo.environment["MON_CAPTURE_END_FIXTURE_OWNER"]
      ?? "539640c5-f04c-4d91-96f2-36e37008fe1d"
    var manifest = Manifest(
      schemaVersion: 2, scenario: "synthetic-acquisition-end", captureMode: "system",
      backend: "screencapturekit", targetPID: nil, targetPIDs: [], startedAtUTC: "synthetic",
      requestedDurationSeconds: nil, osVersion: "synthetic", architecture: "arm64", state: "saved",
      microphone: mic, system: system, errors: [], recordingOwnerId: owner)
    let originals = try ["microphone.caf", "system.caf"].map {
      try Data(contentsOf: directory.appendingPathComponent($0))
    }
    for badOwner in [nil, "foreign-owner"] as [String?] {
      var rejected = manifest
      rejected.recordingOwnerId = badOwner
      XCTAssertThrowsError(
        try rejected.finalizeStoppedClock(
          clock, microphoneClosure: micClosure, systemClosure: pcClosure))
      XCTAssertNil(rejected.captureEnd)
      XCTAssertEqual(rejected.schemaVersion, 2)
    }
    for (micProof, systemProof) in [(nil, pcClosure), (micClosure, nil), (pcClosure, micClosure)]
      as [(CaptureClosure?, CaptureClosure?)]
    {
      var rejected = manifest
      XCTAssertThrowsError(
        try rejected.finalizeStoppedClock(
          clock, microphoneClosure: micProof, systemClosure: systemProof))
      XCTAssertNil(rejected.captureEnd)
      XCTAssertEqual(rejected.schemaVersion, 2)
    }
    let foreignClock = try CaptureClockDomain(units: MachTimeUnits(numerator: 1, denominator: 1)) {
      box.read()
    }
    var foreign = manifest
    XCTAssertThrowsError(
      try foreign.finalizeStoppedClock(
        clock, microphoneClosure: micClosure, systemClosure: foreignClock.close(.system)))
    var beyond = manifest
    beyond.microphone.recordSavedClock(
      .timestamp(1_100_000_010), frames: 480, sourceFrame: 480)
    beyond.microphone.frames += 480
    XCTAssertThrowsError(
      try beyond.finalizeStoppedClock(
        clock, microphoneClosure: micClosure, systemClosure: pcClosure))
    XCTAssertNil(beyond.captureEnd)
    box.set(
      CaptureClockReading(
        hostBeforeNs: 30_000_000_000, continuousNs: 40_000_000_010,
        utcMs: 1_900_000_000_000, hostAfterNs: 30_000_000_020))
    try manifest.finalizeStoppedClock(
      clock, microphoneClosure: micClosure, systemClosure: pcClosure)
    XCTAssertEqual(manifest.schemaVersion, 3)
    XCTAssertEqual(manifest.captureEnd?.microphoneClosedUs, 20000)
    XCTAssertEqual(manifest.captureEnd?.systemClosedUs, 50000)
    XCTAssertEqual(manifest.captureEnd?.endUs, 50000)
    XCTAssertEqual(manifest.microphone.clockSegments?.last?.timelineEndUs, 10000)
    XCTAssertEqual(manifest.system.clockSegments?.first?.timelineStartUs, 5000)
    XCTAssertEqual(manifest.system.clockSegments?.last?.timelineEndUs, 15000)
    let bytes = try manifest.encodedData()
    try bytes.write(to: directory.appendingPathComponent("manifest.json"), options: .atomic)
    XCTAssertEqual(
      try ["microphone.caf", "system.caf"].map {
        try Data(contentsOf: directory.appendingPathComponent($0))
      }, originals)
    if let path = ProcessInfo.processInfo.environment["MON_CAPTURE_END_FIXTURE_DIR"] {
      let destination = URL(fileURLWithPath: path, isDirectory: true)
      XCTAssertTrue(try FileManager.default.contentsOfDirectory(atPath: path).isEmpty)
      for name in ["manifest.json", "microphone.caf", "system.caf"] {
        try FileManager.default.copyItem(
          at: directory.appendingPathComponent(name), to: destination.appendingPathComponent(name))
      }
    }
  }

  func testFailedStreamStopCannotProduceRecordingClosureEvenOnReplay() async throws {
    let directory = try directory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let source = SilentSystemSource(stopFailure: true)
    let clock = try clock()
    let recorder = SystemAudioRecorder(
      outputURL: directory.appendingPathComponent("system.caf"),
      clock: clock, retries: true, factory: { _, _ in source })
    try await recorder.start(targetPID: nil)
    recorder.receive(from: source, sampleBuffer: try sample(), observation: .timestamp(999_000_000))
    for _ in 0..<2 {
      do {
        _ = try await recorder.stopRecording()
        XCTFail("failed stream stop minted closure")
      } catch {}
    }
    XCTAssertEqual(
      try AVAudioFile(forReading: directory.appendingPathComponent("system.caf")).length, 480)
  }

  func testConcurrentRecordingStopsShareOneCompletedCAFAndClosure() async throws {
    let directory = try directory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let source = SilentSystemSource()
    let clock = try clock()
    let recorder = SystemAudioRecorder(
      outputURL: directory.appendingPathComponent("system.caf"),
      clock: clock, retries: true, factory: { _, _ in source })
    try await recorder.start(targetPID: nil)
    recorder.receive(from: source, sampleBuffer: try sample(), observation: .timestamp(999_000_000))
    async let first = recorder.stopRecording()
    async let second = recorder.stopRecording()
    let (a, b) = try await (first, second)
    XCTAssertEqual(a, b)
    XCTAssertEqual(source.counts.1, 1)
    try verifyRecordedCAF(
      directory.appendingPathComponent("system.caf"), expected: recorder.snapshot())
  }

  func testLegacyStopCannotMintAClosureAfterCAFWasAlreadyFinalized() async throws {
    let directory = try directory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let source = SilentSystemSource()
    let recorder = try recorder(directory, SilentSystemFactory([source]), SystemTestTime())
    try await recorder.start(targetPID: nil)
    recorder.receive(from: source, sampleBuffer: try sample(), observation: .timestamp(999_000_000))
    try await recorder.stop()
    do {
      _ = try await recorder.stopRecording()
      XCTFail("legacy stop was upgraded from a later timestamp")
    } catch {}
  }

  func testNeverStartedTapCannotMintClosureBeforeOrAfterCleanupWithoutTouchingDevices() throws {
    let directory = try directory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let tap = CoreAudioTapRecorder(
      outputURL: directory.appendingPathComponent("system.caf"), clock: try clock())
    XCTAssertThrowsError(try tap.stopRecording())
    try tap.stop()
    XCTAssertThrowsError(try tap.stopRecording())
    XCTAssertThrowsError(try tap.start(targetPIDs: []))
    XCTAssertFalse(
      FileManager.default.fileExists(atPath: directory.appendingPathComponent("system.caf").path))
  }
  private let identity = CaptureApplicationIdentity(
    pid: 321, bundleIdentifier: "com.example.confirmed", launchTimeUs: 1_700_000_000_000_000)

  private func clock() throws -> CaptureClockDomain {
    try CaptureClockDomain(units: MachTimeUnits(numerator: 1, denominator: 1)) {
      CaptureClockReading(
        hostBeforeNs: 999_999_990, continuousNs: 1_000_000_000,
        utcMs: 1_700_000_000_000, hostAfterNs: 1_000_000_010)
    }
  }
  private func wait(_ predicate: () -> Bool) async throws {
    let deadline = ContinuousClock.now.advanced(by: .seconds(3))
    while !predicate(), ContinuousClock.now < deadline {
      try await Task.sleep(for: .milliseconds(1))
    }
    XCTAssertTrue(predicate(), "owned asynchronous operation did not reach the expected state")
  }
  private func sample(
    frames: Int = 480, value: Float = 0.25, rate: Double = 48000, rightValue: Float? = nil
  ) throws
    -> CMSampleBuffer
  {
    var format = AudioStreamBasicDescription(
      mSampleRate: rate, mFormatID: kAudioFormatLinearPCM,
      mFormatFlags: kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked,
      mBytesPerPacket: 8, mFramesPerPacket: 1, mBytesPerFrame: 8,
      mChannelsPerFrame: 2, mBitsPerChannel: 32, mReserved: 0)
    var description: CMAudioFormatDescription?
    XCTAssertEqual(
      CMAudioFormatDescriptionCreate(
        allocator: kCFAllocatorDefault, asbd: &format, layoutSize: 0, layout: nil,
        magicCookieSize: 0, magicCookie: nil, extensions: nil,
        formatDescriptionOut: &description), noErr)
    let values = (0..<frames * 2).map { $0 % 2 == 0 ? value : (rightValue ?? -value) }
    var block: CMBlockBuffer?
    XCTAssertEqual(
      CMBlockBufferCreateWithMemoryBlock(
        allocator: kCFAllocatorDefault, memoryBlock: nil, blockLength: values.count * 4,
        blockAllocator: kCFAllocatorDefault, customBlockSource: nil, offsetToData: 0,
        dataLength: values.count * 4, flags: 0, blockBufferOut: &block), noErr)
    let data = try XCTUnwrap(block)
    values.withUnsafeBytes {
      XCTAssertEqual(
        CMBlockBufferReplaceDataBytes(
          with: $0.baseAddress!, blockBuffer: data, offsetIntoDestination: 0, dataLength: $0.count),
        noErr)
    }
    var sample: CMSampleBuffer?
    XCTAssertEqual(
      CMAudioSampleBufferCreateReadyWithPacketDescriptions(
        allocator: kCFAllocatorDefault, dataBuffer: data,
        formatDescription: try XCTUnwrap(description),
        sampleCount: frames, presentationTimeStamp: CMTime(value: 77, timescale: 1),
        packetDescriptions: nil, sampleBufferOut: &sample), noErr)
    return try XCTUnwrap(sample)
  }
  private func directory() throws -> URL {
    let url = FileManager.default.temporaryDirectory.appendingPathComponent(
      "mon-system-recovery-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: url, withIntermediateDirectories: false)
    return url
  }
  private func recorder(
    _ directory: URL, _ factory: SilentSystemFactory, _ time: SystemTestTime,
    retries: Bool = true,
    requireIdentity: @escaping @Sendable (CaptureApplicationIdentity) throws -> Void = { _ in }
  ) throws -> SystemAudioRecorder {
    SystemAudioRecorder(
      outputURL: directory.appendingPathComponent("system.caf"), clock: try clock(),
      levelsEnabled: true, retries: retries, factory: { try factory.make($0, $1) },
      now: time.read, requireIdentity: requireIdentity, mayContinue: time.mayContinue)
  }

  func testFrozenApplicationScopeRejectsGlobalFallbackPIDReuseAndUnconfirmedRetry() throws {
    XCTAssertThrowsError(
      try SystemAudioRecovery(targetPID: identity.pid, applicationIdentity: nil, retries: true))
    XCTAssertThrowsError(
      try SystemAudioRecovery(targetPID: 999, applicationIdentity: identity, retries: true))
    let policy = try SystemAudioRecovery(
      targetPID: identity.pid, applicationIdentity: identity, retries: true)
    try policy.requireScope(targetPID: identity.pid, identity: identity)
    for changed in [
      nil,
      CaptureApplicationIdentity(
        pid: identity.pid, bundleIdentifier: identity.bundleIdentifier,
        launchTimeUs: identity.launchTimeUs + 1),
    ] {
      XCTAssertThrowsError(try policy.requireScope(targetPID: identity.pid, identity: changed))
    }
    XCTAssertThrowsError(try policy.requireScope(targetPID: nil, identity: nil))
    let system = try SystemAudioRecovery(targetPID: nil, applicationIdentity: nil, retries: true)
    XCTAssertThrowsError(try system.requireScope(targetPID: identity.pid, identity: identity))
  }

  func testOnlyUnboundedConfirmedScreenCaptureOptionsEnableRecovery() throws {
    let output = ["--output", "/tmp/mon-system-option-test"]
    let controlled = ["--stop-on-file", "--parent-pid", "1234", "--until-stopped"]
    let json = String(decoding: try JSONEncoder().encode(identity), as: UTF8.self)
    XCTAssertTrue(try Options.parse(output + controlled).recoversSystemAudio)
    XCTAssertTrue(
      try Options.parse(
        output + controlled + [
          "--mode", "application", "--pid", "321", "--application-identity", json,
        ]
      ).recoversSystemAudio)
    for args in [
      [], ["--duration", "14400"], controlled + ["--mode", "application", "--pid", "321"],
      controlled + ["--backend", "coreaudio", "--mode", "application", "--pid", "321"],
      [
        "--stop-on-file", "--parent-pid", "1234", "--microphone-uid", "fixed-mic",
        "--five-second-test", "12345678-1234-4234-8234-123456789abc",
      ],
    ] {
      XCTAssertFalse(try Options.parse(output + args).recoversSystemAudio)
    }
  }

  func testApplicationReacquisitionCannotFollowARestartedProcessOrFallbackToWholePC() async throws {
    let directory = try directory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let old = SilentSystemSource()
    let current = SilentSystemSource()
    let factory = SilentSystemFactory([old, current])
    let time = SystemTestTime()
    let application = SilentApplicationIdentity(identity)
    let recorder = try recorder(directory, factory, time, requireIdentity: application.require)
    try await recorder.start(targetPID: identity.pid, applicationIdentity: identity)
    recorder.receive(from: old, sampleBuffer: try sample(), observation: .timestamp(1_000_000_000))
    recorder.sourceEnded(old)
    time.set(2_000_000_000)
    recorder.maintain()
    try await wait { !recorder.acquisitionPending }
    XCTAssertEqual(factory.calls, 2)
    XCTAssertTrue(factory.observedScopes.allSatisfy { $0.0 == identity.pid && $0.1 == identity })
    let reused = CaptureApplicationIdentity(
      pid: identity.pid, bundleIdentifier: identity.bundleIdentifier,
      launchTimeUs: identity.launchTimeUs + 1)
    application.set(reused)
    time.set(3_000_000_000)
    recorder.maintain()
    try await wait { !recorder.acquisitionPending }
    recorder.receive(
      from: current, sampleBuffer: try sample(value: 9), observation: .timestamp(1_030_000_000))
    XCTAssertEqual(factory.calls, 2)
    XCTAssertEqual(recorder.snapshot().frames, 480)
    XCTAssertEqual(recorder.snapshot().systemRecovery?.reason, "application_unavailable")
    XCTAssertEqual(recorder.confirmedScope().0, identity.pid)
    XCTAssertEqual(recorder.confirmedScope().1, identity)
    do {
      try await recorder.stop()
      XCTFail("changed application cannot publish complete capture")
    } catch {}
    XCTAssertEqual(
      try AVAudioFile(forReading: directory.appendingPathComponent("system.caf")).length, 480)
  }

  func testStopDrainsFinalPartialPacketButRejectsSubsequentSamples() async throws {
    let directory = try directory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let source = SilentSystemSource()
    let factory = SilentSystemFactory([source])
    let time = SystemTestTime()
    let recorder = try recorder(directory, factory, time)
    try await recorder.start(targetPID: nil)
    recorder.receive(
      from: source, sampleBuffer: try sample(), observation: .timestamp(1_000_000_000))
    try await wait { recorder.readyForPCM }
    // Build inside the callback to avoid transferring a non-Sendable buffer.
    source.onStop { [self] in
      do {
        recorder.receive(
          from: source, sampleBuffer: try sample(frames: 17, value: 0.5),
          observation: .timestamp(1_010_000_000))
      } catch { XCTFail("synthetic final packet failed: \(error)") }
    }
    try await recorder.stop()
    XCTAssertEqual(recorder.snapshot().frames, 497)
    recorder.receive(
      from: source, sampleBuffer: try sample(value: 9), observation: .timestamp(1_020_000_000))
    XCTAssertEqual(recorder.snapshot().frames, 497)
    XCTAssertEqual(
      try AVAudioFile(forReading: directory.appendingPathComponent("system.caf")).length, 497)
    try await recorder.stop()
    XCTAssertEqual(source.counts.1, 1)
  }

  func testFailedRestartRemainsOneEpisodeAndNextConfirmedAttemptCanRecover() async throws {
    let directory = try directory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let old = SilentSystemSource()
    let failed = SilentSystemSource(startFailure: true)
    let current = SilentSystemSource()
    let factory = SilentSystemFactory([old, failed, current])
    let time = SystemTestTime()
    let recorder = try recorder(directory, factory, time)
    try await recorder.start(targetPID: nil)
    recorder.receive(from: old, sampleBuffer: try sample(), observation: .timestamp(1_000_000_000))
    recorder.sourceEnded(old)
    time.set(2_000_000_000)
    recorder.maintain()
    try await wait { !recorder.acquisitionPending }
    XCTAssertEqual(recorder.snapshot().systemRecovery?.reason, "restart_failed")
    XCTAssertEqual(failed.counts.1, 1)
    for _ in 0..<10 { recorder.maintain() }
    XCTAssertEqual(factory.calls, 2)
    time.set(3_000_000_000)
    recorder.maintain()
    try await wait { !recorder.acquisitionPending }
    XCTAssertEqual(recorder.snapshot().systemRecovery?.state, "connected")
    XCTAssertEqual(recorder.snapshot().systemRecovery?.interruptions, 1)
    recorder.sourceEnded(failed)
    try await wait { recorder.readyForPCM }
    recorder.receive(
      from: current, sampleBuffer: try sample(value: 0.5), observation: .timestamp(1_030_000_000))
    try await recorder.stop()
    XCTAssertEqual(recorder.snapshot().frames, 960)
    XCTAssertEqual(recorder.snapshot().systemRecovery?.sourceEpoch, 3)
  }

  func testTimeoutRetainsSingleFlightAndLateResultCannotActivate() throws {
    var policy = try SystemAudioRecovery(targetPID: nil, applicationIdentity: nil, retries: true)
    let token = try policy.begin(at: 1_000_000_000)
    XCTAssertFalse(policy.expire(at: 5_999_999_999))
    XCTAssertTrue(policy.expire(at: 6_000_000_000))
    XCTAssertEqual(policy.operation, token)
    XCTAssertThrowsError(try policy.activate(token))
    XCTAssertThrowsError(try policy.begin(at: 7_000_000_000))
    for now in [7_000_000_000, 8_000_000_000] { XCTAssertTrue(policy.expire(at: Int64(now))) }
    XCTAssertEqual(policy.status.interruptions, 1)
    policy.operationFinished(token)
    let replacement = try policy.begin(at: 9_000_000_000)
    policy.operationFinished(token)
    XCTAssertEqual(policy.operation, replacement)
    XCTAssertThrowsError(try policy.activate(token))
    try policy.activate(replacement)
    policy.operationFinished(replacement)
    XCTAssertTrue(policy.accepts(replacement))
    XCTAssertFalse(policy.accepts(token))
  }

  func testRetryRateStopAndFormatMismatchNeverCreateNewSource() throws {
    var policy = try SystemAudioRecovery(targetPID: nil, applicationIdentity: nil, retries: true)
    let original = try policy.begin(at: 1_000_000_000)
    try policy.activate(original)
    policy.operationFinished(original)
    policy.interrupt("stream_stopped")
    XCTAssertThrowsError(try policy.begin(at: 1_999_999_999))
    XCTAssertThrowsError(try policy.begin(at: 0))
    let next = try policy.begin(at: 2_000_000_000)
    try policy.activate(next)
    policy.operationFinished(next)
    policy.interrupt("format_changed", unavailable: true)
    policy.interrupt("application_unavailable", unavailable: true)
    XCTAssertEqual(policy.status.reason, "format_changed")
    XCTAssertThrowsError(try policy.begin(at: 3_000_000_000))
    policy.stop()
    let stopped = policy.status
    policy.interrupt("stream_stopped")
    XCTAssertFalse(policy.expire(at: 9_000_000_000))
    XCTAssertEqual(policy.status, stopped)
    XCTAssertThrowsError(try policy.begin(at: 10_000_000_000))
    XCTAssertThrowsError(try policy.activate(next))
  }

  func testNormalRecorderRebindsWithoutAdoptingOldErrorsOrPCMAndExportsRealCAF() async throws {
    let directory = try directory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let old = SilentSystemSource()
    let current = SilentSystemSource()
    let factory = SilentSystemFactory([old, current])
    let time = SystemTestTime()
    let recorder = try recorder(directory, factory, time)
    try await recorder.start(targetPID: nil)
    recorder.receive(from: old, sampleBuffer: try sample(), observation: .timestamp(1_000_000_000))
    XCTAssertEqual(recorder.snapshot().frames, 480)
    recorder.sourceEnded(old)
    XCTAssertEqual(recorder.snapshot().systemRecovery?.reason, "stream_stopped")
    recorder.receive(
      from: old, sampleBuffer: try sample(value: 9), observation: .timestamp(1_010_000_000))
    time.set(2_000_000_000)
    recorder.maintain()
    try await wait { !recorder.acquisitionPending }
    XCTAssertEqual(factory.calls, 2)
    XCTAssertTrue(factory.observedScopes.allSatisfy { $0.0 == nil && $0.1 == nil })
    XCTAssertEqual(recorder.snapshot().systemRecovery?.sourceEpoch, 2)
    recorder.sourceEnded(old)
    recorder.receive(
      from: old, sampleBuffer: try sample(value: 9), observation: .timestamp(1_020_000_000))
    try await wait { recorder.readyForPCM }
    recorder.receive(
      from: current, sampleBuffer: try sample(value: 0.5), observation: .timestamp(1_030_000_000))
    XCTAssertEqual(recorder.snapshot().frames, 960)
    XCTAssertEqual(recorder.snapshot().appendFailures, 0)
    let active = recorder.snapshot()
    let history = try XCTUnwrap(active.inputEventHistory)
    XCTAssertEqual(history.events.map(\.kind), ["interrupted", "recovered"])
    XCTAssertEqual(history.events.map { $0.status.sourceEpoch }, [1, 2])
    XCTAssertTrue(history.matches(try XCTUnwrap(active.systemRecovery), input: "system"))
    // Replayed old stream termination and PCM cannot manufacture a new event.
    XCTAssertEqual(history.lastSequence, 2)
    try await recorder.stop()
    XCTAssertEqual(recorder.snapshot().inputEventHistory, history)
    XCTAssertEqual(current.counts.1, 1)
    let micURL = directory.appendingPathComponent("microphone.caf")
    let format = try XCTUnwrap(AVAudioFormat(standardFormatWithSampleRate: 48000, channels: 2))
    var micFile: AVAudioFile? = try AVAudioFile(
      forWriting: micURL, settings: format.settings,
      commonFormat: .pcmFormatFloat32, interleaved: false)
    let buffer = try XCTUnwrap(AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 1920))
    buffer.frameLength = 1920
    for channel in 0..<2 {
      for frame in 0..<1920 {
        buffer.floatChannelData![channel][frame] = channel == 0 ? 0.125 : -0.125
      }
    }
    try XCTUnwrap(micFile).write(from: buffer)
    micFile = nil
    var mic = TrackStats(frames: 1920, sampleRate: 48000, channels: 2)
    mic.recordSavedClock(.timestamp(1_000_000_000), frames: 1920, sourceFrame: nil)
    var manifest = Manifest(
      schemaVersion: 2, scenario: "synthetic-system-recovery", captureMode: "system",
      backend: "screencapturekit", targetPID: nil, targetPIDs: [],
      startedAtUTC: "2023-11-14T22:13:20Z",
      requestedDurationSeconds: nil, osVersion: "synthetic", architecture: "arm64", state: "saved",
      microphone: mic, system: recorder.snapshot(), errors: [])
    try manifest.finalizeClock(clock())
    XCTAssertEqual(manifest.system.clockSegments?.count, 2)
    XCTAssertEqual(manifest.system.clockSegments?[0].timelineEndUs, 10000)
    XCTAssertEqual(manifest.system.clockSegments?[1].timelineStartUs, 30000)
    try verifyRecordedCAF(directory.appendingPathComponent("system.caf"), expected: manifest.system)
    let reader = try AVAudioFile(forReading: directory.appendingPathComponent("system.caf"))
    XCTAssertEqual(reader.length, 960)
    let output = try XCTUnwrap(
      AVAudioPCMBuffer(pcmFormat: reader.processingFormat, frameCapacity: 960))
    try reader.read(into: output)
    for channel in 0..<2 {
      for frame in 0..<960 {
        XCTAssertEqual(
          output.floatChannelData![channel][frame],
          (frame < 480 ? Float(0.25) : 0.5) * (channel == 0 ? 1 : -1))
      }
    }
    try manifest.encodedData().write(
      to: directory.appendingPathComponent("manifest.json"), options: .withoutOverwriting)
    if let destination = ProcessInfo.processInfo.environment["MON_SYSTEM_RECOVERY_FIXTURE_DIR"] {
      XCTAssertTrue(try FileManager.default.contentsOfDirectory(atPath: destination).isEmpty)
      for name in ["manifest.json", "microphone.caf", "system.caf"] {
        try FileManager.default.copyItem(
          at: directory.appendingPathComponent(name),
          to: URL(fileURLWithPath: destination).appendingPathComponent(name))
      }
    }
  }

  func testNoSamplesDoesNotInferAStreamFailureAndBoundedProbeDoesNotRetry() async throws {
    for retries in [true, false] {
      let directory = try directory()
      defer { try? FileManager.default.removeItem(at: directory) }
      let source = SilentSystemSource()
      let factory = SilentSystemFactory([source])
      let time = SystemTestTime()
      let recorder = try recorder(directory, factory, time, retries: retries)
      try await recorder.start(targetPID: nil)
      time.set(100_000_000_000)
      for _ in 0..<10 { recorder.maintain() }
      XCTAssertEqual(factory.calls, 1)
      XCTAssertEqual(recorder.snapshot().systemRecovery?.state, "connected")
      if !retries {
        recorder.sourceEnded(source)
        recorder.maintain()
        XCTAssertEqual(factory.calls, 1)
        XCTAssertEqual(recorder.snapshot().appendFailures, 1)
      }
      do {
        try await recorder.stop()
        XCTFail("empty audio cannot finalize")
      } catch {}
      XCTAssertFalse(
        FileManager.default.fileExists(atPath: directory.appendingPathComponent("system.caf").path))
    }
  }

  func testExpiredStartStaysOwnedUntilReturnAndCannotActivateOrAppend() async throws {
    let directory = try directory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let old = SilentSystemSource()
    let held = SilentSystemSource(held: true)
    let factory = SilentSystemFactory([old, held])
    let time = SystemTestTime()
    let recorder = try recorder(directory, factory, time)
    try await recorder.start(targetPID: nil)
    recorder.receive(from: old, sampleBuffer: try sample(), observation: .timestamp(1_000_000_000))
    recorder.sourceEnded(old)
    time.set(2_000_000_000)
    recorder.maintain()
    try await wait { held.counts.0 == 1 }
    time.set(7_000_000_000)
    for _ in 0..<100 { recorder.maintain() }
    XCTAssertTrue(recorder.acquisitionPending)
    XCTAssertEqual(factory.calls, 2)
    XCTAssertEqual(recorder.snapshot().systemRecovery?.reason, "acquisition_timeout")
    recorder.receive(
      from: held, sampleBuffer: try sample(value: 9), observation: .timestamp(1_030_000_000))
    held.release()
    try await wait { !recorder.acquisitionPending }
    XCTAssertEqual(held.counts.1, 1)
    XCTAssertEqual(recorder.snapshot().frames, 480)
    XCTAssertEqual(recorder.snapshot().systemRecovery?.interruptions, 1)
    do {
      try await recorder.stop()
      XCTFail("unrecovered tail cannot publish saved")
    } catch {}
    let reader = try AVAudioFile(forReading: directory.appendingPathComponent("system.caf"))
    XCTAssertEqual(reader.length, 480)
  }

  func testStopDuringPendingRestartPreventsLateActivationAndKeepsSavedPCM() async throws {
    let directory = try directory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let old = SilentSystemSource()
    let held = SilentSystemSource(held: true)
    let factory = SilentSystemFactory([old, held])
    let time = SystemTestTime()
    let recorder = try recorder(directory, factory, time)
    try await recorder.start(targetPID: nil)
    recorder.receive(from: old, sampleBuffer: try sample(), observation: .timestamp(1_000_000_000))
    recorder.sourceEnded(old)
    time.set(2_000_000_000)
    recorder.maintain()
    try await wait { held.counts.0 == 1 }
    let stopping = Task { try await recorder.stop() }
    try await wait { recorder.snapshot().systemRecovery?.state == "stopped" }
    held.release()
    do {
      try await stopping.value
      XCTFail("interrupted tail must remain unresolved")
    } catch {}
    XCTAssertFalse(recorder.acquisitionPending)
    recorder.maintain()
    recorder.sourceEnded(held)
    recorder.receive(
      from: held, sampleBuffer: try sample(value: 9), observation: .timestamp(1_030_000_000))
    XCTAssertEqual(factory.calls, 2)
    XCTAssertEqual(recorder.snapshot().frames, 480)
    XCTAssertEqual(recorder.snapshot().systemRecovery?.state, "stopped")
  }

  func testCurrentStartingStreamPreservesEarlyPCMWithoutClaimingConnected() async throws {
    let directory = try directory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let held = SilentSystemSource(held: true)
    let factory = SilentSystemFactory([held])
    let time = SystemTestTime()
    let recorder = try recorder(directory, factory, time)
    let starting = Task { try await recorder.start(targetPID: nil) }
    try await wait { held.counts.0 == 1 }
    recorder.receive(from: held, sampleBuffer: try sample(), observation: .timestamp(1_000_000_000))
    XCTAssertEqual(recorder.snapshot().frames, 480)
    XCTAssertEqual(recorder.snapshot().systemRecovery?.state, "reconnecting")
    held.release()
    try await starting.value
    XCTAssertEqual(recorder.snapshot().systemRecovery?.state, "connected")
    try await recorder.stop()
    XCTAssertEqual(
      try AVAudioFile(forReading: directory.appendingPathComponent("system.caf")).length, 480)
  }

  func testStopRequestRejectsLateStartBeforeControlLoopObservesIt() async throws {
    let directory = try directory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let old = SilentSystemSource()
    let held = SilentSystemSource(held: true)
    let factory = SilentSystemFactory([old, held])
    let time = SystemTestTime()
    let recorder = try recorder(directory, factory, time)
    try await recorder.start(targetPID: nil)
    recorder.receive(from: old, sampleBuffer: try sample(), observation: .timestamp(1_000_000_000))
    recorder.sourceEnded(old)
    time.set(2_000_000_000)
    recorder.maintain()
    try await wait { held.counts.0 == 1 }
    time.requestStop()
    held.release()
    try await wait { !recorder.acquisitionPending }
    XCTAssertNotEqual(recorder.snapshot().systemRecovery?.state, "connected")
    XCTAssertEqual(held.counts.1, 1)
    recorder.receive(
      from: held, sampleBuffer: try sample(value: 9), observation: .timestamp(1_030_000_000))
    recorder.maintain()
    XCTAssertEqual(factory.calls, 2)
    XCTAssertEqual(recorder.snapshot().frames, 480)
    do {
      try await recorder.stop()
      XCTFail("interrupted tail must remain retained")
    } catch {}
  }

  func testUnresponsiveStartDoesNotBlockPCMFinalizationOrCreateAnotherSource() async throws {
    let directory = try directory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let old = SilentSystemSource()
    let held = SilentSystemSource(held: true)
    let factory = SilentSystemFactory([old, held])
    let time = SystemTestTime()
    let recorder = try recorder(directory, factory, time)
    try await recorder.start(targetPID: nil)
    recorder.receive(from: old, sampleBuffer: try sample(), observation: .timestamp(1_000_000_000))
    recorder.sourceEnded(old)
    time.set(2_000_000_000)
    recorder.maintain()
    try await wait { held.counts.0 == 1 }
    let began = ContinuousClock.now
    do {
      try await recorder.stop()
      XCTFail("pending OS request cannot prove complete capture")
    } catch {}
    XCTAssertLessThan(began.duration(to: .now), .seconds(7))
    XCTAssertTrue(recorder.acquisitionPending)
    XCTAssertEqual(
      try AVAudioFile(forReading: directory.appendingPathComponent("system.caf")).length, 480)
    held.release()
    try await wait { !recorder.acquisitionPending }
    recorder.maintain()
    recorder.receive(
      from: held, sampleBuffer: try sample(value: 9), observation: .timestamp(1_030_000_000))
    XCTAssertEqual(factory.calls, 2)
    XCTAssertEqual(recorder.snapshot().systemRecovery?.state, "stopped")
    XCTAssertEqual(recorder.snapshot().frames, 480)
  }

  func testLateShareableContentCannotEvenStartAStreamAfterTimeout() async throws {
    let directory = try directory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let old = SilentSystemSource()
    let current = SilentSystemSource()
    let contentGate = SilentSystemSource(held: true)
    let factory = SilentSystemFactory([old, current])
    let time = SystemTestTime()
    let recorder = SystemAudioRecorder(
      outputURL: directory.appendingPathComponent("system.caf"), clock: try clock(),
      retries: true,
      factory: { recorder, token in
        if token.epoch > 1 { try await contentGate.start() }
        return try factory.make(recorder, token)
      }, now: time.read, mayContinue: time.mayContinue)
    try await recorder.start(targetPID: nil)
    recorder.receive(from: old, sampleBuffer: try sample(), observation: .timestamp(1_000_000_000))
    recorder.sourceEnded(old)
    time.set(2_000_000_000)
    recorder.maintain()
    try await wait { contentGate.counts.0 == 1 }
    time.set(7_000_000_000)
    recorder.maintain()
    XCTAssertTrue(recorder.acquisitionPending)
    contentGate.release()
    try await wait { !recorder.acquisitionPending }
    XCTAssertEqual(current.counts.0, 0)
    XCTAssertEqual(current.counts.2, 1)
    XCTAssertEqual(recorder.snapshot().systemRecovery?.reason, "acquisition_timeout")
    do {
      try await recorder.stop()
      XCTFail("missing tail remains unresolved")
    } catch {}
  }

  func testFailedOldOutputDetachPreventsReplacementUntilTheSameStreamIsRetired() async throws {
    let directory = try directory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let old = SilentSystemSource(detachFailure: true)
    let current = SilentSystemSource()
    let factory = SilentSystemFactory([old, current])
    let time = SystemTestTime()
    let recorder = try recorder(directory, factory, time)
    try await recorder.start(targetPID: nil)
    recorder.receive(from: old, sampleBuffer: try sample(), observation: .timestamp(1_000_000_000))
    recorder.sourceEnded(old)
    time.set(2_000_000_000)
    recorder.maintain()
    try await wait { !recorder.acquisitionPending }
    XCTAssertEqual(factory.calls, 1, "a failed detach must not release the owned source")
    XCTAssertEqual(current.counts.0, 0)
    XCTAssertEqual(old.counts.2, 1)
    XCTAssertEqual(recorder.snapshot().systemRecovery?.state, "unavailable")
    XCTAssertEqual(recorder.snapshot().systemRecovery?.reason, "restart_failed")
    recorder.receive(
      from: old, sampleBuffer: try sample(value: 9), observation: .timestamp(1_010_000_000))
    XCTAssertEqual(recorder.snapshot().frames, 480)

    old.setDetachFailure(false)
    time.set(3_000_000_000)
    recorder.maintain()
    try await wait { !recorder.acquisitionPending }
    XCTAssertEqual(old.counts.2, 2)
    XCTAssertEqual(old.counts.1, 0, "delegate-confirmed termination must not be stopped again")
    XCTAssertEqual(factory.calls, 2)
    XCTAssertEqual(recorder.snapshot().systemRecovery?.sourceEpoch, 3)
    XCTAssertEqual(recorder.snapshot().systemRecovery?.state, "connected")
    recorder.sourceEnded(old)
    recorder.receive(
      from: old, sampleBuffer: try sample(value: 9), observation: .timestamp(1_020_000_000))
    XCTAssertEqual(recorder.snapshot().systemRecovery?.state, "connected")
    recorder.receive(
      from: current, sampleBuffer: try sample(value: 0.5), observation: .timestamp(1_030_000_000))
    XCTAssertEqual(recorder.snapshot().frames, 960)
    XCTAssertEqual(recorder.snapshot().appendFailures, 0)
    try await recorder.stop()
    let audio = try AVAudioFile(forReading: directory.appendingPathComponent("system.caf"))
    XCTAssertEqual(audio.length, 960)
    let buffer = try XCTUnwrap(
      AVAudioPCMBuffer(pcmFormat: audio.processingFormat, frameCapacity: 960))
    try audio.read(into: buffer)
    XCTAssertEqual(buffer.frameLength, 960)
    for frame in 0..<960 {
      let value: Float = frame < 480 ? 0.25 : 0.5
      XCTAssertEqual(buffer.floatChannelData![0][frame], value)
      XCTAssertEqual(buffer.floatChannelData![1][frame], -value)
    }
  }

  func testUnresponsiveNativeStopDoesNotEnterSynchronousDetachOrMintClosure() async throws {
    let directory = try directory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let source = SilentSystemSource(holdStop: true)
    let factory = SilentSystemFactory([source])
    let time = SystemTestTime()
    let recorder = try recorder(directory, factory, time)
    try await recorder.start(targetPID: nil)
    recorder.receive(
      from: source, sampleBuffer: try sample(), observation: .timestamp(1_000_000_000))
    let began = ContinuousClock.now
    do {
      _ = try await recorder.stopRecording()
      XCTFail("unproven native stop cannot mint closure")
    } catch { XCTAssertTrue(String(describing: error).contains("stop timed out")) }
    XCTAssertLessThan(began.duration(to: .now), .seconds(7))
    XCTAssertEqual(source.counts.1, 1)
    XCTAssertEqual(
      source.counts.2, 0, "detach must not wait synchronously behind timed-out native IO")
    source.releaseStop()
    do {
      _ = try await recorder.stopRecording()
      XCTFail("late stop cannot turn prior failure into saved")
    } catch {}
    XCTAssertEqual(source.counts.2, 0)
    XCTAssertEqual(
      try AVAudioFile(forReading: directory.appendingPathComponent("system.caf")).length, 480)
  }

  func testFormatRetirementRetriesDetachWithoutRepeatingAnAlreadyCompletedStop() async throws {
    let directory = try directory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let source = SilentSystemSource(detachFailure: true)
    let factory = SilentSystemFactory([source])
    let time = SystemTestTime()
    let recorder = try recorder(directory, factory, time)
    try await recorder.start(targetPID: nil)
    recorder.receive(
      from: source, sampleBuffer: try sample(), observation: .timestamp(1_000_000_000))
    recorder.receive(
      from: source, sampleBuffer: try sample(rate: 44100), observation: .timestamp(1_010_000_000))
    recorder.maintain()
    try await wait { !recorder.acquisitionPending }
    XCTAssertEqual(source.counts.1, 1)
    XCTAssertEqual(source.counts.2, 1)
    XCTAssertEqual(factory.calls, 1)
    XCTAssertEqual(recorder.snapshot().systemRecovery?.reason, "format_changed")
    recorder.receive(
      from: source, sampleBuffer: try sample(value: 9), observation: .timestamp(1_020_000_000))
    source.setDetachFailure(false)
    time.set(2_000_000_000)
    recorder.maintain()
    try await wait { !recorder.acquisitionPending }
    XCTAssertEqual(source.counts.1, 1)
    XCTAssertEqual(source.counts.2, 2)
    for _ in 0..<10 { recorder.maintain() }
    XCTAssertEqual(factory.calls, 1, "format retirement never permits a replacement format")
    XCTAssertEqual(recorder.snapshot().frames, 480)
    XCTAssertEqual(recorder.snapshot().appendFailures, 0)
    do {
      try await recorder.stop()
      XCTFail("format retirement cannot turn unresolved audio into a saved legacy recording")
    } catch {}
    XCTAssertEqual(
      try AVAudioFile(forReading: directory.appendingPathComponent("system.caf")).length, 480)
  }

  func testFailedRetirementStopDoesNotDetachOrReplaceTheStillOwnedStream() async throws {
    let directory = try directory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let source = SilentSystemSource(stopFailure: true)
    let factory = SilentSystemFactory([source])
    let time = SystemTestTime()
    let recorder = try recorder(directory, factory, time)
    try await recorder.start(targetPID: nil)
    recorder.receive(
      from: source, sampleBuffer: try sample(), observation: .timestamp(1_000_000_000))
    recorder.receive(
      from: source, sampleBuffer: try sample(rate: 44100), observation: .timestamp(1_010_000_000))
    for attempt in 1...2 {
      recorder.maintain()
      try await wait { !recorder.acquisitionPending }
      XCTAssertEqual(source.counts.1, attempt)
      XCTAssertEqual(source.counts.2, 0, "a failed stop must not become completed retirement")
      XCTAssertEqual(factory.calls, 1)
      XCTAssertEqual(recorder.snapshot().systemRecovery?.reason, "format_changed")
      recorder.receive(
        from: source, sampleBuffer: try sample(value: 9), observation: .timestamp(1_020_000_000))
      XCTAssertEqual(recorder.snapshot().frames, 480)
    }
    for _ in 0..<2 {
      do {
        _ = try await recorder.stopRecording()
        XCTFail("failed OS stop cannot mint a recording closure")
      } catch {}
    }
    XCTAssertEqual(factory.calls, 1)
    XCTAssertEqual(recorder.snapshot().appendFailures, 0)
    XCTAssertEqual(
      try AVAudioFile(forReading: directory.appendingPathComponent("system.caf")).length, 480)
  }

  func testChangedSourceFormatBlocksRetriesWithoutOverwritingExistingCAF() async throws {
    let directory = try directory()
    defer { try? FileManager.default.removeItem(at: directory) }
    let old = SilentSystemSource()
    let current = SilentSystemSource()
    let factory = SilentSystemFactory([old, current])
    let time = SystemTestTime()
    let recorder = try recorder(directory, factory, time)
    try await recorder.start(targetPID: nil)
    recorder.receive(from: old, sampleBuffer: try sample(), observation: .timestamp(1_000_000_000))
    recorder.sourceEnded(old)
    time.set(2_000_000_000)
    recorder.maintain()
    try await wait { !recorder.acquisitionPending }
    recorder.receive(
      from: current, sampleBuffer: try sample(rate: 44100), observation: .timestamp(1_030_000_000))
    XCTAssertEqual(recorder.snapshot().systemRecovery?.reason, "format_changed")
    time.set(10_000_000_000)
    for _ in 0..<10 { recorder.maintain() }
    try await wait { !recorder.acquisitionPending }
    XCTAssertEqual(factory.calls, 2)
    XCTAssertEqual(current.counts.1, 1)
    XCTAssertEqual(recorder.snapshot().frames, 480)
    XCTAssertEqual(recorder.snapshot().appendFailures, 0)
    do {
      try await recorder.stop()
      XCTFail("format change must retain unresolved audio")
    } catch {}
    XCTAssertEqual(
      try AVAudioFile(forReading: directory.appendingPathComponent("system.caf")).length, 480)
  }
}
