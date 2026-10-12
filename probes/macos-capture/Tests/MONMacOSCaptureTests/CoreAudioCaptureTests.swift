import AVFoundation
import CoreAudio
import CoreMedia
import Darwin
import Foundation
import XCTest

@testable import MONMacOSCapture

private final class TapTestState: @unchecked Sendable {
  private let lock = NSLock()
  private var values: [String] = []
  func append(_ value: String) { lock.withLock { values.append(value) } }
  var events: [String] { lock.withLock { values } }
}

private final class SilentTapDriver: CoreAudioCaptureDriver, @unchecked Sendable {
  let state = TapTestState()
  let startEntered = DispatchSemaphore(value: 0)
  let releaseStart = DispatchSemaphore(value: 0)
  var heldStart = false
  var startFailure = false
  private let lock = NSLock()
  private var failedStop = false
  private var sample: (@Sendable (CMSampleBuffer) -> Void)?
  private var failure: (@Sendable (Error) -> Void)?
  func setStopFailure(_ value: Bool) { lock.withLock { failedStop = value } }
  func start(
    sample: @escaping @Sendable (CMSampleBuffer) -> Void,
    failure: @escaping @Sendable (Error) -> Void
  ) throws {
    state.append(Thread.isMainThread ? "start-main" : "start-background")
    lock.withLock {
      self.sample = sample
      self.failure = failure
    }
    startEntered.signal()
    if heldStart, releaseStart.wait(timeout: .now() + 5) != .success {
      throw ProbeError.capture("synthetic held start timeout")
    }
    if startFailure { throw ProbeError.capture("synthetic tap initialization failure") }
  }
  func stop() throws {
    state.append("stop")
    if lock.withLock({ failedStop }) { throw ProbeError.capture("synthetic tap stop failure") }
  }
  func detach() throws { state.append("detach") }
  func emit(_ value: CMSampleBuffer) { lock.withLock { sample }?(value) }
  func fail(_ error: Error = ProbeError.capture("synthetic tap interruption")) {
    lock.withLock { failure }?(error)
  }
}

final class CoreAudioCaptureTests: XCTestCase, @unchecked Sendable {
  func testQuietPCStopsNormallyWithClosedEmptyRawProofButFailedStopStillFails() async throws {
    for failedStop in [false, true] {
      let created = FileManager.default.temporaryDirectory
        .appendingPathComponent("mon-quiet-tap-\(UUID())")
      try FileManager.default.createDirectory(
        at: created, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
      let path = try XCTUnwrap(realpath(created.path, nil))
      defer { free(path) }
      let root = URL(fileURLWithPath: String(cString: path), isDirectory: true)
      defer { try? FileManager.default.removeItem(at: root) }
      let raw = try RawRecordingSegments(
        output: root, owner: UUID().uuidString.lowercased(), lane: "system")
      let driver = SilentTapDriver()
      let recorder = SystemAudioRecorder(
        outputURL: root.appendingPathComponent("system.caf"), clock: try CaptureClockDomain.live(),
        retries: true, backend: "coreaudio",
        factory: { recorder, _ in
          CoreAudioCaptureStream(
            driver: driver,
            deliver: { source, sample in recorder.receive(from: source, sampleBuffer: sample) },
            ended: { source, error in recorder.sourceEnded(source, error: error) })
        }, rawSegments: raw)
      try await recorder.start(targetPID: nil)
      XCTAssertEqual(recorder.snapshot().systemRecovery?.state, "connected")
      driver.setStopFailure(failedStop)
      do {
        let ending = try await recorder.stopRecording()
        XCTAssertFalse(failedStop)
        let replay = try await recorder.stopRecording()
        XCTAssertEqual(replay, ending)
        XCTAssertTrue(raw.snapshot().producerClosed)
        XCTAssertEqual(raw.snapshot().sealedFrames, 0)
      } catch {
        XCTAssertTrue(failedStop)
        XCTAssertTrue(String(describing: error).contains("synthetic tap stop failure"))
      }
      XCTAssertEqual(recorder.snapshot().frames, 0)
      XCTAssertFalse(
        FileManager.default.fileExists(atPath: root.appendingPathComponent("system.caf").path))
    }
  }
  private let identity = CaptureApplicationIdentity(
    pid: 123, bundleIdentifier: "com.example.confirmed", launchTimeUs: 1_791_100_000_000_001)

  private func pcm(frames: Int = 480, interleaved: Bool = true) throws -> AVAudioPCMBuffer {
    let format = try XCTUnwrap(
      AVAudioFormat(
        commonFormat: .pcmFormatFloat32,
        sampleRate: 48_000, channels: 2, interleaved: interleaved))
    let buffer = try XCTUnwrap(
      AVAudioPCMBuffer(
        pcmFormat: format,
        frameCapacity: AVAudioFrameCount(frames)))
    buffer.frameLength = AVAudioFrameCount(frames)
    let planes = UnsafeMutableAudioBufferListPointer(buffer.mutableAudioBufferList)
    for (plane, audio) in planes.enumerated() {
      let values = audio.mData!.assumingMemoryBound(to: Float.self)
      for index in 0..<Int(audio.mDataByteSize) / 4 {
        values[index] = interleaved ? (index % 2 == 0 ? 0.25 : -0.5) : (plane == 0 ? 0.25 : -0.5)
      }
    }
    return buffer
  }

  private func timestamp() -> AudioTimeStamp {
    var time = AudioTimeStamp()
    time.mHostTime = mach_absolute_time()
    time.mSampleTime = 480
    time.mFlags = [.hostTimeValid, .sampleTimeValid]
    return time
  }

  private func sample() throws -> CMSampleBuffer {
    let buffer = try pcm()
    return try CoreAudioTapSample.copy(
      buffer.audioBufferList, time: timestamp(), format: buffer.format)
  }

  private func source(
    _ driver: SilentTapDriver, state: TapTestState = TapTestState(),
    deliver: (@Sendable (CoreAudioCaptureStream, CMSampleBuffer) -> Void)? = nil,
    ended: (@Sendable (CoreAudioCaptureStream, Error) -> Void)? = nil
  ) -> CoreAudioCaptureStream {
    CoreAudioCaptureStream(
      driver: driver, deliver: deliver ?? { _, _ in state.append("sample") },
      ended: ended ?? { _, _ in state.append("ended") })
  }

  func testGlobalAndApplicationScopesCannotBeConfused() throws {
    let rows: [(pid: pid_t, objectID: AudioObjectID)] = [
      (123, 21), (123, 22), (456, 23), (999, 24),
    ]
    let global = try CoreAudioTapScope.system.description(objects: rows, ownPID: 999)
    XCTAssertTrue(global.isExclusive)
    XCTAssertEqual(global.processes, [24])
    XCTAssertTrue(global.isPrivate)
    XCTAssertEqual(global.muteBehavior, .unmuted)
    let selected = try CoreAudioTapScope.application(identity).description(
      objects: rows, ownPID: 999)
    XCTAssertFalse(selected.isExclusive)
    XCTAssertEqual(selected.processes, [21, 22])
    XCTAssertThrowsError(
      try CoreAudioTapScope.application(identity).description(objects: [], ownPID: 999))
    XCTAssertThrowsError(
      try CoreAudioTapScope.application(identity).description(objects: [(456, 21)], ownPID: 999))
    XCTAssertThrowsError(
      try CoreAudioTapScope.application(identity).description(objects: rows, ownPID: 123))
    XCTAssertTrue(try CoreAudioTapScope.system.description(objects: [], ownPID: 999).isExclusive)
  }

  func testTapScopeCatalogIsBoundedAndRejectsAmbiguousObjects() {
    for rows: [(pid: pid_t, objectID: AudioObjectID)] in [
      [(0, 21)], [(123, 0)], [(123, 21), (456, 21)],
      (1...4097).map { (pid_t($0), AudioObjectID($0)) },
    ] {
      XCTAssertThrowsError(try CoreAudioTapScope.system.description(objects: rows, ownPID: 9999))
    }
  }

  func testTapCopiesOwnedInterleavedAndPlanarPCMWithHardwareHostClock() throws {
    for interleaved in [true, false] {
      let buffer = try pcm(interleaved: interleaved)
      let time = timestamp()
      let owned = try CoreAudioTapSample.copy(
        buffer.audioBufferList, time: time, format: buffer.format)
      XCTAssertEqual(owned.numSamples, 480)
      XCTAssertEqual(
        owned.presentationTimeStamp, CMClockMakeHostTimeFromSystemUnits(time.mHostTime))
      // Poison borrowed input after the IO callback returns: owned PCM remains.
      for audio in UnsafeMutableAudioBufferListPointer(buffer.mutableAudioBufferList) {
        memset(audio.mData!, 0, Int(audio.mDataByteSize))
      }
      let block = try XCTUnwrap(owned.dataBuffer)
      var values = [Float](repeating: 0, count: 960)
      XCTAssertEqual(
        values.withUnsafeMutableBytes {
          CMBlockBufferCopyDataBytes(
            block, atOffset: 0, dataLength: $0.count, destination: $0.baseAddress!)
        }, noErr)
      XCTAssertEqual(values[0], 0.25)
      XCTAssertEqual(values[interleaved ? 1 : 480], -0.5)
      let trimmed = try testPCMFrames(owned, range: 100..<200)
      XCTAssertEqual(trimmed.numSamples, 100)
    }
  }

  func testTapRejectsMissingInvalidTimestampsAndOversizedOrMismatchedPCM() throws {
    let buffer = try pcm()
    let good = timestamp()
    var times = [AudioTimeStamp]()
    var invalid = good
    invalid.mFlags = [.sampleTimeValid]
    times.append(invalid)
    invalid = good
    invalid.mHostTime = 0
    times.append(invalid)
    invalid = good
    invalid.mFlags = [.hostTimeValid]
    times.append(invalid)
    for value in [-1.0, Double.nan, Double.infinity, 0.5] {
      invalid = good
      invalid.mSampleTime = value
      times.append(invalid)
    }
    for time in times {
      XCTAssertThrowsError(
        try CoreAudioTapSample.copy(
          buffer.audioBufferList,
          time: time, format: buffer.format))
    }
    let tooLarge = try pcm(frames: CoreAudioTapSample.maximumFrames + 1)
    XCTAssertThrowsError(
      try CoreAudioTapSample.copy(tooLarge.audioBufferList, time: good, format: tooLarge.format))
    let list = buffer.mutableAudioBufferList
    let planes = UnsafeMutableAudioBufferListPointer(list)
    planes[0].mDataByteSize -= 1
    XCTAssertThrowsError(
      try CoreAudioTapSample.copy(UnsafePointer(list), time: good, format: buffer.format))
  }

  func testStartAndStopAreSerializedOffMainThreadAndStoppedSourceCannotRestart() async throws {
    let driver = SilentTapDriver()
    let source = source(driver)
    XCTAssertNotNil(source.synchronizationClock)
    XCTAssertNil(source.native)
    try await source.start()
    try await source.stop()
    try source.detach()
    try source.detach()
    XCTAssertEqual(driver.state.events, ["start-background", "stop", "stop", "detach"])
    do {
      try await source.start()
      XCTFail("stopped source restarted")
    } catch {}
  }

  func testLateStartCannotReactivateAfterStopRequest() async throws {
    let driver = SilentTapDriver()
    driver.heldStart = true
    let source = source(driver)
    let start = Task { try await source.start() }
    XCTAssertEqual(driver.startEntered.wait(timeout: .now() + 2), .success)
    let stopped = expectation(description: "native stopped after held start")
    source.requestStop { error in
      XCTAssertNil(error)
      stopped.fulfill()
    }
    driver.releaseStart.signal()
    do {
      try await start.value
      XCTFail("obsolete start activated")
    } catch {}
    await fulfillment(of: [stopped], timeout: 2)
    try source.detach()
    XCTAssertEqual(driver.state.events, ["start-background", "stop", "stop", "detach"])
  }

  func testInitializationFailureStillStopsAndDetachesPartiallyCreatedResources() async throws {
    let driver = SilentTapDriver()
    driver.startFailure = true
    let source = source(driver)
    do {
      try await source.start()
      XCTFail("initialization should fail")
    } catch {}
    try source.detach()
    XCTAssertEqual(driver.state.events, ["start-background", "stop", "detach"])
  }

  func testStopReceiptWaitsForAdmittedPCMAndRejectsLateCallbacks() async throws {
    let driver = SilentTapDriver()
    let state = TapTestState()
    let entered = DispatchSemaphore(value: 0)
    let release = DispatchSemaphore(value: 0)
    let source = source(
      driver, state: state,
      deliver: { _, _ in
        entered.signal()
        _ = release.wait(timeout: .now() + 5)
        state.append("sample")
      })
    try await source.start()
    let value = try sample()
    driver.emit(value)
    XCTAssertEqual(entered.wait(timeout: .now() + 2), .success)
    let stopped = expectation(description: "stop drains PCM")
    source.requestStop { error in
      XCTAssertNil(error)
      state.append("receipt")
      stopped.fulfill()
    }
    release.signal()
    await fulfillment(of: [stopped], timeout: 2)
    driver.emit(value)
    try source.detach()
    XCTAssertEqual(state.events, ["sample", "receipt"])
  }

  func testFailureNotificationIsSingleAndNativeStopFailurePreventsDetach() async throws {
    let driver = SilentTapDriver()
    driver.setStopFailure(true)
    let state = TapTestState()
    let ended = expectation(description: "one interruption")
    let source = source(
      driver,
      ended: { _, _ in
        state.append("ended")
        ended.fulfill()
      })
    try await source.start()
    driver.fail()
    driver.fail()
    await fulfillment(of: [ended], timeout: 2)
    XCTAssertEqual(driver.state.events, ["start-background", "stop"])
    XCTAssertThrowsError(try source.detach())
    XCTAssertFalse(driver.state.events.contains("detach"))
    driver.setStopFailure(false)
    try source.detach()
    XCTAssertEqual(state.events, ["ended"])
    XCTAssertEqual(driver.state.events.last, "detach")
  }

  func testBoundedDeliveryBacklogFailsWithoutInventingAudio() async throws {
    let driver = SilentTapDriver()
    let state = TapTestState()
    let entered = DispatchSemaphore(value: 0)
    let release = DispatchSemaphore(value: 0)
    let ended = expectation(description: "bounded capacity failure")
    let source = source(
      driver,
      deliver: { _, _ in
        entered.signal()
        _ = release.wait(timeout: .now() + 5)
        state.append("sample")
      },
      ended: { _, error in
        XCTAssertTrue(String(describing: error).contains("bounded capacity"))
        state.append("ended")
        ended.fulfill()
      })
    try await source.start()
    let value = try sample()
    driver.emit(value)
    XCTAssertEqual(entered.wait(timeout: .now() + 2), .success)
    for _ in 0..<70 { driver.emit(value) }
    release.signal()
    await fulfillment(of: [ended], timeout: 2)
    try source.detach()
    XCTAssertEqual(state.events, ["sample", "ended"])
  }

  func testOwnedTapPCMUsesSharedFiveSecondWindowAndCAFWriterWithoutHardware() async throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(
      "mon-tap-fixture-\(UUID())")
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
    defer { try? FileManager.default.removeItem(at: directory) }
    let driver = SilentTapDriver()
    let window = try CaptureTestWindow(ownerId: "735027d4-a03e-43d3-830a-21e767dfaf88")
    let recorder = SystemAudioRecorder(
      outputURL: directory.appendingPathComponent("system.caf"),
      clock: try CaptureClockDomain.live(), levelsEnabled: true, testWindow: window,
      backend: "coreaudio",
      factory: { recorder, _ in
        CoreAudioCaptureStream(
          driver: driver,
          deliver: { source, sample in
            recorder.receive(from: source, sampleBuffer: sample)
          }, ended: { source, _ in recorder.sourceEnded(source) })
      })
    try await recorder.start(targetPID: nil)
    // Startup audio cannot be saved before the user's common test window.
    driver.emit(try sample())
    try await Task.sleep(for: .milliseconds(20))
    XCTAssertEqual(recorder.snapshot().frames, 0)
    try window.open(at: PCMInputLevel.nowNanoseconds())
    let buffer = try pcm(interleaved: false)
    driver.emit(
      try CoreAudioTapSample.copy(buffer.audioBufferList, time: timestamp(), format: buffer.format))
    let deadline = ContinuousClock.now.advanced(by: .seconds(2))
    while recorder.snapshot().frames == 0, ContinuousClock.now < deadline {
      try await Task.sleep(for: .milliseconds(10))
    }
    XCTAssertEqual(recorder.snapshot().frames, 480)
    try await recorder.stop()
    let stats = recorder.snapshot()
    XCTAssertEqual(stats.appendFailures, 0)
    XCTAssertNotNil(stats.firstCaptureNanoseconds)
    try verifyRecordedCAF(directory.appendingPathComponent("system.caf"), expected: stats)
    let file = try AVAudioFile(forReading: directory.appendingPathComponent("system.caf"))
    let saved = try XCTUnwrap(
      AVAudioPCMBuffer(pcmFormat: file.processingFormat, frameCapacity: 480))
    try file.read(into: saved)
    XCTAssertEqual(saved.floatChannelData![0][0], 0.25)
    XCTAssertEqual(saved.floatChannelData![1][0], -0.5)
  }

  func testFailedTapKeepsOriginalReasonBeforeEmptyWriterCleanupWithoutHardware() async throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(
      "mon-tap-failure-fixture-\(UUID())")
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false)
    defer { try? FileManager.default.removeItem(at: directory) }
    let driver = SilentTapDriver()
    let recorder = SystemAudioRecorder(
      outputURL: directory.appendingPathComponent("system.caf"),
      clock: try CaptureClockDomain.live(), backend: "coreaudio",
      factory: { recorder, _ in
        CoreAudioCaptureStream(
          driver: driver,
          deliver: { source, sample in
            recorder.receive(from: source, sampleBuffer: sample)
          }, ended: { source, error in recorder.sourceEnded(source, error: error) })
      })
    try await recorder.start(targetPID: nil)
    let reason = "tap PCM format or hardware timestamp is invalid; timeFlags=1, sampleTime=-512"
    driver.fail(ProbeError.capture(reason))
    let deadline = ContinuousClock.now.advanced(by: .seconds(2))
    while recorder.snapshot().appendFailures == 0, ContinuousClock.now < deadline {
      try await Task.sleep(for: .milliseconds(10))
    }
    XCTAssertEqual(recorder.snapshot().frames, 0)
    XCTAssertEqual(recorder.snapshot().appendFailures, 1)
    XCTAssertEqual(recorder.snapshot().captureFailure, "capture_audio_write_failed")
    XCTAssertThrowsError(try recorder.snapshot().requireWritable()) { error in
      XCTAssertTrue(String(describing: error).contains(reason))
    }
    driver.fail(ProbeError.capture("secondary failure must not replace the first"))
    do {
      try await recorder.stop()
      XCTFail("empty input must not become saved")
    } catch {
      XCTAssertTrue(String(describing: error).contains(reason))
      XCTAssertFalse(String(describing: error).contains("no system audio samples were received"))
    }
    XCTAssertEqual(recorder.snapshot().captureFailureDetail, reason)
    XCTAssertFalse(
      FileManager.default.fileExists(atPath: directory.appendingPathComponent("system.caf").path))
  }

  func testTapFailureMetadataExplainsRejectedTimeAndLayoutWithoutPCM() throws {
    let buffer = try pcm()
    var time = timestamp()
    time.mSampleTime = -512
    XCTAssertThrowsError(
      try CoreAudioTapSample.copy(buffer.audioBufferList, time: time, format: buffer.format)
    ) { error in
      let message = String(describing: error)
      XCTAssertTrue(message.contains("sampleTime=-512"))
      XCTAssertTrue(message.contains("timeFlags="))
      XCTAssertTrue(message.contains("planes=1"))
      XCTAssertFalse(message.contains("0.25"))
    }
    let list = buffer.mutableAudioBufferList
    let planes = UnsafeMutableAudioBufferListPointer(list)
    planes[0].mNumberChannels = 1
    XCTAssertThrowsError(
      try CoreAudioTapSample.copy(UnsafePointer(list), time: timestamp(), format: buffer.format)
    ) { error in
      let message = String(describing: error)
      XCTAssertTrue(message.contains("planeChannels=1"))
      XCTAssertTrue(message.contains("channels=2"))
      XCTAssertTrue(message.contains("hasData=true"))
    }
  }
}
