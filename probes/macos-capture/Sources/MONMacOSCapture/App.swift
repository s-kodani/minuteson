import AVFoundation
import AudioToolbox
import CoreAudio
import CoreMedia
import Darwin
import Foundation
import ScreenCaptureKit

struct Options {
  let output: URL
  let duration: TimeInterval?
  let scenario: String
  let mode: String
  let backend: String
  let targetPIDs: [pid_t]
  let stopOnFile: Bool
  let parentPID: pid_t?
  var microphoneUID: String? = nil
  var applicationIdentity: CaptureApplicationIdentity? = nil
  var inputLevelOwner: String? = nil
  var publishesInputStatus: Bool {
    duration == nil && captureTestOwner == nil && inputLevelOwner != nil
  }
  var recoversSystemAudio: Bool {
    duration == nil && captureTestOwner == nil && backend == "screencapturekit"
      && (targetPIDs.isEmpty || applicationIdentity != nil)
  }
  var captureTestOwner: String? = nil
  var captureTestLeaseFD: Int32? = nil

  static func parse(_ arguments: [String]) throws -> Self {
    var output: URL?
    var duration: TimeInterval = 30
    var durationSpecified = false
    var untilStopped = false
    var scenario = "manual"
    var mode = "system"
    var backend = "screencapturekit"
    var targetPIDs: [pid_t] = []
    var stopOnFile = false
    var parentPID: pid_t?
    var microphoneUID: String?
    var applicationIdentity: CaptureApplicationIdentity?
    var inputLevelOwner: String?
    var captureTestOwner: String?
    var captureTestLeaseFD: Int32?
    var index = 0
    while index < arguments.count {
      switch arguments[index] {
      case "--output":
        index += 1
        guard index < arguments.count else { throw ProbeError.usage("--output requires a path") }
        output = URL(fileURLWithPath: arguments[index], isDirectory: true)
      case "--duration":
        index += 1
        guard index < arguments.count, let value = Double(arguments[index]), value > 0,
          value <= 14_400
        else {
          throw ProbeError.usage("--duration must be in (0, 14400]")
        }
        duration = value
        durationSpecified = true
      case "--scenario":
        index += 1
        guard index < arguments.count, !arguments[index].isEmpty else {
          throw ProbeError.usage("--scenario requires an ID")
        }
        scenario = arguments[index]
      case "--mode":
        index += 1
        guard index < arguments.count, ["system", "application"].contains(arguments[index]) else {
          throw ProbeError.usage("--mode must be system or application")
        }
        mode = arguments[index]
      case "--pid":
        index += 1
        guard index < arguments.count, let value = pid_t(arguments[index]), value > 0 else {
          throw ProbeError.usage("--pid must be a positive process ID")
        }
        targetPIDs.append(value)
      case "--backend":
        index += 1
        guard index < arguments.count,
          ["screencapturekit", "coreaudio"].contains(arguments[index])
        else {
          throw ProbeError.usage("--backend must be screencapturekit or coreaudio")
        }
        backend = arguments[index]
      case "--microphone-uid":
        index += 1
        guard index < arguments.count, microphoneUID == nil,
          InputDevices.validUID(arguments[index])
        else { throw ProbeError.usage("--microphone-uid requires one valid device UID") }
        microphoneUID = arguments[index]
      case "--application-identity":
        index += 1
        guard index < arguments.count, applicationIdentity == nil,
          arguments[index].utf8.count <= 4096
        else { throw ProbeError.usage("--application-identity requires one bounded JSON identity") }
        let parsed = try JSONDecoder().decode(
          CaptureApplicationIdentity.self, from: Data(arguments[index].utf8))
        guard parsed.valid else { throw ProbeError.usage("application identity is invalid") }
        applicationIdentity = parsed
      case "--stop-on-file":
        stopOnFile = true
      case "--input-level-owner":
        index += 1
        guard index < arguments.count, inputLevelOwner == nil,
          InputLevelReport.validOwner(arguments[index])
        else { throw ProbeError.usage("--input-level-owner requires one canonical UUID v4") }
        inputLevelOwner = arguments[index]
      case "--five-second-test":
        index += 1
        guard index < arguments.count, captureTestOwner == nil,
          InputLevelReport.validOwner(arguments[index])
        else { throw ProbeError.usage("--five-second-test requires one canonical UUID v4") }
        captureTestOwner = arguments[index]
      case "--capture-test-lease-fd":
        index += 1
        guard index < arguments.count, captureTestLeaseFD == nil,
          let descriptor = Int32(arguments[index]), descriptor >= 3,
          String(descriptor) == arguments[index]
        else { throw ProbeError.usage("--capture-test-lease-fd requires one descriptor >= 3") }
        captureTestLeaseFD = descriptor
      case "--until-stopped":
        untilStopped = true
      case "--parent-pid":
        index += 1
        guard index < arguments.count, let value = pid_t(arguments[index]), value > 0 else {
          throw ProbeError.usage("--parent-pid must be a positive process ID")
        }
        parentPID = value
      case "--help", "-h":
        throw ProbeError.help
      default:
        throw ProbeError.usage("unknown argument: \(arguments[index])")
      }
      index += 1
    }
    guard let output else { throw ProbeError.usage("--output is required") }
    if mode == "application" && targetPIDs.isEmpty {
      throw ProbeError.usage("at least one --pid is required for application mode")
    }
    if mode == "system" && !targetPIDs.isEmpty {
      throw ProbeError.usage("--pid is only valid for application mode")
    }
    if backend == "coreaudio" && mode != "application" {
      throw ProbeError.usage("the coreaudio backend currently requires application mode")
    }
    if backend == "screencapturekit" && targetPIDs.count > 1 {
      throw ProbeError.usage("multiple --pid values require the coreaudio backend")
    }
    if Set(targetPIDs).count != targetPIDs.count {
      throw ProbeError.usage("duplicate --pid values are not allowed")
    }
    if let identity = applicationIdentity {
      guard mode == "application", backend == "screencapturekit", targetPIDs == [identity.pid]
      else {
        throw ProbeError.usage(
          "application identity requires the exact ScreenCaptureKit target PID")
      }
    }
    if parentPID != nil && !stopOnFile {
      throw ProbeError.usage("--parent-pid requires --stop-on-file")
    }
    if untilStopped && (!stopOnFile || parentPID == nil || durationSpecified) {
      throw ProbeError.usage(
        "--until-stopped requires --stop-on-file and --parent-pid, without --duration")
    }
    if inputLevelOwner != nil && (!stopOnFile || parentPID == nil) {
      throw ProbeError.usage("input levels require stop control and a parent PID")
    }
    if let captureTestOwner {
      guard stopOnFile, parentPID != nil, microphoneUID != nil,
        !durationSpecified, !untilStopped, backend == "screencapturekit", inputLevelOwner == nil,
        mode == "system" || applicationIdentity != nil
      else {
        throw ProbeError.usage(
          "five-second test requires stop control, parent PID, microphone UID and exact application identity when selected; duration, until-stopped, input-level-owner and Core Audio are not accepted"
        )
      }
      duration = 5
      inputLevelOwner = captureTestOwner
    }
    if captureTestLeaseFD != nil && captureTestOwner == nil {
      throw ProbeError.usage("an inherited test lease is only valid for a five-second test")
    }
    return Self(
      output: output.standardizedFileURL, duration: untilStopped ? nil : duration,
      scenario: scenario,
      mode: mode, backend: backend, targetPIDs: targetPIDs, stopOnFile: stopOnFile,
      parentPID: parentPID, microphoneUID: microphoneUID, applicationIdentity: applicationIdentity,
      inputLevelOwner: inputLevelOwner, captureTestOwner: captureTestOwner,
      captureTestLeaseFD: captureTestLeaseFD)
  }

  func shouldContinueRecording(
    startedAt: ContinuousClock.Instant, now: ContinuousClock.Instant,
    stopRequested: Bool, parentExited: Bool
  ) -> Bool {
    !stopRequested && !parentExited
      && (duration.map { now < startedAt.advanced(by: .seconds($0)) } ?? true)
  }
}

enum ProbeError: Error, CustomStringConvertible {
  case help
  case usage(String)
  case permission(String)
  case capture(String)

  var description: String {
    switch self {
    case .help: return ""
    case .usage(let message), .permission(let message), .capture(let message): return message
    }
  }
}

struct TrackStats: Encodable {
  var callbackCount = 0
  var frames: Int64 = 0
  var firstPresentationTimeSeconds: Double?
  var lastPresentationTimeSeconds: Double?
  var appendFailures = 0
  var sampleRate: Double?
  var channels: UInt32?
  var clockSegments: [CapturedClockSegment]?
  var clockFailure: String?
  var timing: CaptureTrackTiming?
  var inputLevel = PCMInputLevel()
  var microphoneRecovery: MicrophoneRecovery.Status?
  var systemRecovery: SystemAudioRecovery.Status?

  enum CodingKeys: String, CodingKey {
    case callbackCount, frames, firstPresentationTimeSeconds, lastPresentationTimeSeconds
    case appendFailures, sampleRate, channels, clockSegments, clockFailure
    case microphoneRecovery, systemRecovery
  }

  var firstCaptureNanoseconds: Int64? { timing?.firstCaptureNanoseconds }

  mutating func recordSavedClock(
    _ observation: CaptureClockObservation, frames: Int64, sourceFrame: Int64?,
    sourceEpoch: UInt64 = 0
  ) {
    guard clockFailure == nil else { return }
    do {
      if timing == nil {
        guard let sampleRate else { throw ProbeError.capture("capture sample rate is missing") }
        timing = try CaptureTrackTiming(sampleRate: sampleRate)
      }
      try timing?.recordSaved(
        frames: frames, observation: observation, sourceFrame: sourceFrame,
        sourceEpoch: sourceEpoch)
    } catch { clockFailure = String(describing: error) }
  }

  mutating func finishClock(originNanoseconds: Int64) throws {
    guard clockFailure == nil, timing != nil else {
      throw ProbeError.capture(clockFailure ?? "capture clock has no saved observations")
    }
    clockSegments = try timing?.finish(originNanoseconds: originNanoseconds, expectedFrames: frames)
  }
}

struct Manifest: Encodable {
  var schemaVersion = 1
  let scenario: String
  let captureMode: String
  let backend: String
  let targetPID: pid_t?
  let targetPIDs: [pid_t]
  var targetApplicationName: String?
  var targetBundleIdentifier: String?
  let startedAtUTC: String
  var endedAtUTC: String?
  let requestedDurationSeconds: Double?
  let osVersion: String
  let architecture: String
  var state: String
  var microphone: TrackStats
  var system: TrackStats
  var power = RecordingPowerStatus()
  var errors: [String]
  var captureClock: CaptureClockMetadata?
  var microphoneDeviceUID: String?
  var targetApplicationIdentity: CaptureApplicationIdentity?
  var captureTest: CaptureTestMetadata? = nil
  var recordingOwnerId: String? = nil

  mutating func finalizeClock(_ clock: CaptureClockDomain) throws {
    guard schemaVersion == 2,
      let micStart = microphone.firstCaptureNanoseconds,
      let systemStart = system.firstCaptureNanoseconds
    else { throw ProbeError.capture("both capture clocks are required for schema 2") }
    let origin = min(micStart, systemStart)
    try microphone.finishClock(originNanoseconds: origin)
    try system.finishClock(originNanoseconds: origin)
    captureClock = try clock.metadata(originNanoseconds: origin)
  }

  func encodedData() throws -> Data {
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
    let data = try encoder.encode(self)
    guard data.count <= 256 * 1024 else {
      throw ProbeError.capture("capture manifest exceeds the bounded reader capacity")
    }
    return data
  }
}

final class SystemAudioRecorder: NSObject, SCStreamOutput, SCStreamDelegate, @unchecked Sendable {
  private enum AcquisitionError: Error { case applicationUnavailable }
  typealias Factory =
    @Sendable (SystemAudioRecorder, SystemAudioRecovery.Binding) async throws ->
    any SystemCaptureStream
  private let outputURL: URL
  private let queue = DispatchQueue(label: "app.mon.capture.system")
  private let lock = NSLock()
  private let pcm: SystemPCMWriter
  private var stream: (any SystemCaptureStream)?
  private var streamToken: SystemAudioRecovery.Binding?
  private var streamEnded = false
  private var recovery: SystemAudioRecovery?
  private var operationTask: Task<Void, Never>?
  private var drainingToken: SystemAudioRecovery.Binding?
  private var captureStarted = false
  private var finished = false
  private var stopping = false
  private let retries: Bool
  private let clock: CaptureClockDomain?
  private let levelsEnabled: Bool
  private let testWindow: CaptureTestWindow?
  private let factory: Factory?
  private let now: @Sendable () -> Int64?
  private let requireIdentity: @Sendable (CaptureApplicationIdentity) throws -> Void
  private let mayContinue: @Sendable () -> Bool
  private(set) var targetApplicationName: String?
  private(set) var targetBundleIdentifier: String?

  init(
    outputURL: URL, clock: CaptureClockDomain? = nil, levelsEnabled: Bool = false,
    testWindow: CaptureTestWindow? = nil, retries: Bool = false,
    factory: Factory? = nil, now: @escaping @Sendable () -> Int64? = PCMInputLevel.nowNanoseconds,
    requireIdentity: @escaping @Sendable (CaptureApplicationIdentity) throws -> Void =
      CaptureApplications.current,
    mayContinue: @escaping @Sendable () -> Bool = { true }
  ) {
    self.outputURL = outputURL
    self.pcm = SystemPCMWriter(outputURL: outputURL)
    self.clock = clock
    self.levelsEnabled = levelsEnabled
    self.testWindow = testWindow
    self.retries = retries && clock != nil && testWindow == nil
    self.factory = factory
    self.now = now
    self.requireIdentity = requireIdentity
    self.mayContinue = mayContinue
  }

  private var applicationIdentity: CaptureApplicationIdentity?

  func start(targetPID: pid_t?, applicationIdentity: CaptureApplicationIdentity? = nil) async throws
  {
    let policy = try SystemAudioRecovery(
      targetPID: targetPID, applicationIdentity: applicationIdentity, retries: retries)
    let token = try lock.withLock {
      guard recovery == nil, !stopping else {
        throw ProbeError.capture("system start is already owned")
      }
      recovery = policy
      self.applicationIdentity = applicationIdentity
      return try recovery!.begin(at: now() ?? -1)
    }
    defer { lock.withLock { recovery?.operationFinished(token) } }
    do { try await acquire(token) } catch {
      lock.withLock {
        if recovery?.owns(token) == true {
          recovery?.interrupt(failureReason(error), unavailable: true)
        }
        pcm.invalidateLevels()
      }
      if lock.withLock({ streamToken == token }) { try? await retireStream() }
      throw error
    }
  }

  private func requireOwned(_ token: SystemAudioRecovery.Binding) throws {
    try Task.checkCancellation()
    guard mayContinue(), lock.withLock({ !stopping && recovery?.owns(token) == true }) else {
      throw ProbeError.capture("system acquisition is obsolete or stopped")
    }
  }

  private func verifyIdentity(_ identity: CaptureApplicationIdentity) throws {
    do { try requireIdentity(identity) } catch { throw AcquisitionError.applicationUnavailable }
  }

  private func failureReason(_ error: Error) -> String {
    if error is AcquisitionError { return "application_unavailable" }
    if case ProbeError.permission = error { return "permission_unavailable" }
    return "restart_failed"
  }

  private func acquire(_ token: SystemAudioRecovery.Binding) async throws {
    try requireOwned(token)
    if let identity = lock.withLock({ recovery?.applicationIdentity }) {
      try verifyIdentity(identity)
    }
    let source: any SystemCaptureStream
    if let factory {
      source = try await factory(self, token)
    } else {
      source = try await makeStream(token)
    }
    do {
      try requireOwned(token)
      if let identity = lock.withLock({ recovery?.applicationIdentity }) {
        try verifyIdentity(identity)
      }
      try lock.withLock {
        guard !stopping, recovery?.owns(token) == true, stream == nil else {
          throw ProbeError.capture("system stream cannot replace an owned stream")
        }
        stream = source
        streamToken = token
        streamEnded = false
        captureStarted = true
        targetApplicationName = source.applicationName
        targetBundleIdentifier = source.bundleIdentifier
      }
    } catch {
      try? source.detach()
      throw error
    }
    try requireOwned(token)
    try await source.start()
    try requireOwned(token)
    if let identity = lock.withLock({ recovery?.applicationIdentity }) {
      try verifyIdentity(identity)
    }
    try lock.withLock {
      guard !stopping, stream === source, !streamEnded else {
        throw ProbeError.capture("system stream ended before activation")
      }
      try recovery?.activate(token)
    }
  }

  private func makeStream(_ token: SystemAudioRecovery.Binding) async throws
    -> any SystemCaptureStream
  {
    try requireOwned(token)
    let (targetPID, applicationIdentity) = lock.withLock {
      (recovery?.targetPID, recovery?.applicationIdentity)
    }
    if let applicationIdentity { try verifyIdentity(applicationIdentity) }
    let content: SCShareableContent
    do {
      content = try await SCShareableContent.excludingDesktopWindows(
        false, onScreenWindowsOnly: false)
    } catch {
      throw ProbeError.permission("system audio permission or shareable content failed: \(error)")
    }
    try requireOwned(token)
    if let applicationIdentity { try verifyIdentity(applicationIdentity) }
    guard let display = content.displays.first else {
      throw ProbeError.capture("no display available for system audio capture")
    }
    let filter: SCContentFilter
    var name: String?
    var bundle: String?
    if let targetPID {
      let matches = content.applications.filter { $0.processID == targetPID }
      guard matches.count == 1, let application = matches.first
      else {
        throw ProbeError.capture("selected process is not available in shareable applications")
      }
      name = application.applicationName
      bundle = application.bundleIdentifier
      if let applicationIdentity {
        guard application.processID == applicationIdentity.pid,
          application.bundleIdentifier == applicationIdentity.bundleIdentifier
        else {
          throw ProbeError.capture("shareable application does not match the confirmed identity")
        }
        try verifyIdentity(applicationIdentity)
      }
      filter = SCContentFilter(
        display: display, including: [application], exceptingWindows: [])
    } else {
      filter = SCContentFilter(display: display, excludingWindows: [])
    }
    let configuration = SCStreamConfiguration()
    configuration.width = 2
    configuration.height = 2
    configuration.minimumFrameInterval = CMTime(value: 1, timescale: 1)
    configuration.showsCursor = false
    configuration.capturesAudio = true
    configuration.excludesCurrentProcessAudio = true
    configuration.sampleRate = 48_000
    configuration.channelCount = 2
    let stream = SCStream(filter: filter, configuration: configuration, delegate: self)
    try stream.addStreamOutput(self, type: .audio, sampleHandlerQueue: queue)
    return ScreenCaptureStream(
      stream: stream, output: self, applicationName: name, bundleIdentifier: bundle)
  }

  /// Never stop ScreenCaptureKit on its delegate/sample queue. Awaiting OS
  /// work happens in this owned task, leaving the microphone/control loop free.
  private func retireStream() async throws {
    let old = lock.withLock { (stream, captureStarted && !streamEnded) }
    if let stream = old.0 {
      if old.1 { try await stream.stop() }
      try? stream.detach()
      queue.sync {}
      lock.withLock {
        if self.stream === stream {
          self.stream = nil
          streamToken = nil
          captureStarted = false
          streamEnded = true
        }
      }
    }
  }

  func maintain() {
    guard retries, mayContinue(), lock.withLock({ !stopping }) else { return }
    if let identity = lock.withLock({ applicationIdentity }) {
      do { try requireIdentity(identity) } catch {
        lock.withLock {
          recovery?.interrupt("application_unavailable", unavailable: true)
          pcm.invalidateLevels()
        }
      }
    }
    guard let now = now() else { return }
    lock.withLock {
      guard !stopping, let policy = recovery else { return }
      if recovery?.expire(at: now) == true {
        pcm.invalidateLevels()
        operationTask?.cancel()
      }
      // A timed-out request retains operation ownership. Do not accumulate
      // simultaneous SCStreams, tasks or permission prompts on every poll.
      let retirementOnly = policy.status.reason == "format_changed" && stream != nil
      guard policy.status.state != "connected", recovery?.operation == nil,
        let token = try?
          (retirementOnly ? recovery?.beginRetirement(at: now) : recovery?.begin(at: now))
      else { return }
      operationTask = Task { [self] in
        defer {
          lock.withLock {
            recovery?.operationFinished(token)
            operationTask = nil
          }
        }
        do {
          try await retireStream()
          if retirementOnly { return }
          try requireOwned(token)
          if let identity = lock.withLock({ applicationIdentity }) { try verifyIdentity(identity) }
          try await acquire(token)
        } catch {
          lock.withLock {
            if recovery?.owns(token) == true {
              recovery?.interrupt(failureReason(error), unavailable: true)
            }
            pcm.invalidateLevels()
          }
          // Also owns the cleanup of a late start. A hanging cleanup remains
          // single-flight and can never activate a replacement source.
          if lock.withLock({ streamToken == token }) { try? await retireStream() }
        }
      }
    }
  }

  func stream(_ stream: SCStream, didStopWithError error: Error) {
    if let source = lock.withLock({ self.stream?.native === stream ? self.stream : nil }) {
      sourceEnded(source)
    }
  }

  func sourceEnded(_ stream: any SystemCaptureStream) {
    lock.withLock {
      guard self.stream === stream, let token = streamToken else { return }
      streamEnded = true
      guard !stopping, recovery?.owns(token) == true else { return }
      recovery?.interrupt("stream_stopped")
      pcm.invalidateLevels()
      if !retries { pcm.recordFailure() }
    }
  }

  private final class StopReceipt: @unchecked Sendable {
    private let lock = NSLock()
    private var value: (Bool, String?) = (false, nil)
    func complete(_ error: Error?) {
      lock.withLock { value = (true, error.map { String(describing: $0) }) }
    }
    func read() -> (Bool, String?) { lock.withLock { value } }
  }

  func stop() async throws {
    let previous = lock.withLock { () -> SystemAudioRecovery.Status? in
      guard !finished else { return nil }
      stopping = true
      let status = recovery?.status
      if let token = streamToken, recovery?.accepts(token) == true { drainingToken = token }
      recovery?.stop()
      operationTask?.cancel()
      return status
    }
    if lock.withLock({ finished }) { return }
    var streamStopError: String?
    let deadline = ContinuousClock.now.advanced(by: .seconds(5))
    while lock.withLock({ recovery?.operation != nil }), ContinuousClock.now < deadline {
      try? await Task.sleep(for: .milliseconds(10))
    }
    if lock.withLock({ recovery?.operation != nil }) {
      streamStopError =
        "system acquisition is still pending; retained audio is not published as saved"
    } else if let active = lock.withLock({ captureStarted && !streamEnded ? stream : nil }) {
      let receipt = StopReceipt()
      active.requestStop { receipt.complete($0) }
      let deadline = ContinuousClock.now.advanced(by: .seconds(5))
      while !receipt.read().0, ContinuousClock.now < deadline {
        try? await Task.sleep(for: .milliseconds(10))
      }
      let result = receipt.read()
      streamStopError = result.0 ? result.1 : "system stream stop timed out"
    }
    queue.sync {}
    lock.withLock {
      drainingToken = nil
      captureStarted = false
      pcm.invalidateLevels()
      finished = true
    }
    if let source = lock.withLock({ stream }), lock.withLock({ recovery?.operation == nil }) {
      try? source.detach()
    }
    try await pcm.finish()
    if let streamStopError {
      throw ProbeError.capture("system stop failed after CAF finalization: \(streamStopError)")
    }
    if retries, previous?.state != "connected" {
      throw ProbeError.capture(
        "system input did not recover before stop; trailing missing time is unresolved")
    }
  }

  func snapshot() -> TrackStats {
    lock.lock()
    defer { lock.unlock() }
    var result = pcm.stats
    result.systemRecovery = recovery?.status
    return result
  }

  var acquisitionPending: Bool { lock.withLock { recovery?.operation != nil } }
  var readyForPCM: Bool { lock.withLock { pcm.ready } }

  func confirmedScope() -> (pid_t?, CaptureApplicationIdentity?) {
    lock.withLock { (recovery?.targetPID, recovery?.applicationIdentity) }
  }

  func stream(
    _ stream: SCStream, didOutputSampleBuffer sampleBuffer: CMSampleBuffer,
    of outputType: SCStreamOutputType
  ) {
    guard outputType == .audio, sampleBuffer.isValid, sampleBuffer.numSamples > 0 else { return }
    if let source = lock.withLock({ self.stream?.native === stream ? self.stream : nil }) {
      receive(from: source, sampleBuffer: sampleBuffer)
    }
  }

  func receive(
    from stream: any SystemCaptureStream, sampleBuffer: CMSampleBuffer,
    observation suppliedObservation: CaptureClockObservation? = nil
  ) {
    guard sampleBuffer.isValid, sampleBuffer.numSamples > 0 else { return }
    let receivedAt = levelsEnabled ? PCMInputLevel.nowNanoseconds() : nil
    lock.lock()
    defer { lock.unlock() }
    guard self.stream === stream, let token = streamToken,
      !streamEnded,
      (!stopping && mayContinue() && recovery?.acceptsPCM(token) == true)
        || (stopping && drainingToken == token)
    else { return }
    do {
      if let testWindow, !testWindow.accepting { return }
      if let identity = applicationIdentity {
        do { try requireIdentity(identity) } catch {
          recovery?.interrupt("application_unavailable", unavailable: true)
          pcm.invalidateLevels()
          if !retries { pcm.recordFailure() }
          return
        }
      }
      var sampleBuffer = sampleBuffer
      var testObservation: CaptureClockObservation?
      if let testWindow {
        guard let clock, let description = sampleBuffer.formatDescription,
          let format = CMAudioFormatDescriptionGetStreamBasicDescription(description)?.pointee
        else { throw ProbeError.capture("five-second test clock or audio format is missing") }
        let observation = clock.observe(
          presentation: sampleBuffer.presentationTimeStamp,
          clock: stream.native?.synchronizationClock)
        guard
          let selected = try testWindow.selection(
            observation: observation, frames: sampleBuffer.numSamples,
            sampleRate: format.mSampleRate, savedFrames: pcm.stats.frames)
        else { return }
        sampleBuffer = try testPCMFrames(sampleBuffer, range: selected.frames)
        testObservation = selected.observation
      }
      let observation =
        testObservation
        ?? suppliedObservation
        ?? clock?.observe(
          presentation: sampleBuffer.presentationTimeStamp,
          clock: stream.native?.synchronizationClock)
      try pcm.append(
        sampleBuffer, observation: observation, sourceEpoch: token.epoch,
        levelsEnabled: levelsEnabled, receivedAt: receivedAt)
    } catch SystemPCMWriter.AppendError.formatChanged {
      recovery?.interrupt("format_changed", unavailable: true)
      pcm.invalidateLevels()
      if !retries { pcm.recordFailure() }
    } catch {
      pcm.recordFailure()
    }
  }

}

private final class MicrophoneRecorder: @unchecked Sendable {
  private enum RebindError: Error { case formatChanged }
  private var engine: AVAudioEngine?
  private var configurationObserver: NSObjectProtocol?
  private var configurationChanged = false
  private var recovery: MicrophoneRecovery?
  private var lastRecoveryAttempt: ContinuousClock.Instant?
  private let outputURL: URL
  private let lock = NSLock()
  private var file: AVAudioFile?
  private var tapInstalled = false
  private var stats = TrackStats()
  private let clock: CaptureClockDomain?
  private let levelsEnabled: Bool
  private let testWindow: CaptureTestWindow?

  private let requestedUID: String?
  private var inputUnit: AudioUnit?
  private var selectedDeviceID: AudioObjectID?
  private(set) var deviceUID: String?

  init(
    outputURL: URL, clock: CaptureClockDomain? = nil, microphoneUID: String? = nil,
    levelsEnabled: Bool = false, testWindow: CaptureTestWindow? = nil
  ) {
    self.outputURL = outputURL
    self.clock = clock
    self.requestedUID = microphoneUID
    self.levelsEnabled = levelsEnabled
    self.testWindow = testWindow
  }

  func start() async throws {
    let selected = try InputDevices.choose(requestedUID, from: InputDevices.live().devices)
    var permission = AVCaptureDevice.authorizationStatus(for: .audio)
    if permission == .notDetermined {
      _ = await AVCaptureDevice.requestAccess(for: .audio)
      permission = AVCaptureDevice.authorizationStatus(for: .audio)
    }
    guard permission == .authorized else {
      throw ProbeError.permission(
        "microphone permission is \(String(describing: permission)); grant it in System Settings and retry"
      )
    }
    recovery = try MicrophoneRecovery(uid: selected.uid)
    deviceUID = selected.uid
    try configure(selected)
  }

  private func configure(_ selected: CaptureInputDevice) throws {
    let token = try lock.withLock {
      guard let token = try recovery?.begin(selected) else {
        throw ProbeError.capture("microphone recovery owner is missing")
      }
      return token
    }
    let engine = AVAudioEngine()
    self.engine = engine
    let node = engine.inputNode
    guard let unit = node.audioUnit else {
      throw ProbeError.capture("microphone input unit is unavailable")
    }
    var device = selected.objectID
    let status = AudioUnitSetProperty(
      unit, kAudioOutputUnitProperty_CurrentDevice,
      kAudioUnitScope_Global, 0, &device, UInt32(MemoryLayout<AudioObjectID>.stride))
    guard status == noErr else {
      throw ProbeError.capture("could not select microphone: \(status)")
    }
    try InputDevices.verifyDevice(unit, expected: selected.objectID, expectedUID: token.uid)
    let format = node.outputFormat(forBus: 0)
    guard format.sampleRate > 0, format.channelCount > 0 else {
      throw ProbeError.capture("microphone has no usable audio format")
    }
    try lock.withLock {
      if let file {
        guard file.processingFormat.isEqual(format), stats.sampleRate == format.sampleRate,
          stats.channels == format.channelCount
        else {
          throw RebindError.formatChanged
        }
      } else {
        guard stats.frames == 0 else { throw ProbeError.capture("microphone writer was lost") }
        file = try AVAudioFile(
          forWriting: outputURL, settings: format.settings, commonFormat: .pcmFormatFloat32,
          interleaved: false)
        stats.sampleRate = format.sampleRate
        stats.channels = format.channelCount
      }
      inputUnit = unit
      selectedDeviceID = selected.objectID
      configurationChanged = false
    }
    configurationObserver = NotificationCenter.default.addObserver(
      forName: .AVAudioEngineConfigurationChange, object: engine, queue: nil
    ) { [weak self] _ in
      // Apple's callback runs on an internal queue. Never stop, rebuild or
      // deallocate an engine here; the serialized control loop does that.
      self?.lock.withLock {
        guard self?.recovery?.owns(token) == true else { return }
        if self?.configurationChanged == false,
          self?.clock == nil || self?.testWindow != nil
        {
          self?.stats.appendFailures += 1
        }
        self?.configurationChanged = true
        self?.stats.inputLevel.invalidate()
      }
    }
    node.installTap(onBus: 0, bufferSize: 1024, format: format) { [weak self] buffer, when in
      guard let self else { return }
      let receivedAt = self.levelsEnabled ? PCMInputLevel.nowNanoseconds() : nil
      self.lock.lock()
      defer { self.lock.unlock() }
      guard self.recovery?.accepts(token) == true, !self.configurationChanged else { return }
      do {
        guard let unit = self.inputUnit, let device = self.selectedDeviceID else {
          throw ProbeError.capture("microphone selection is missing")
        }
        try InputDevices.verifyDevice(unit, expected: device, expectedUID: token.uid)
      } catch {
        if self.clock == nil || self.testWindow != nil { self.stats.appendFailures += 1 }
        self.recovery?.interrupt("identity_changed")
        self.configurationChanged = true
        self.stats.inputLevel.invalidate()
        return
      }
      do {
        if let testWindow = self.testWindow, !testWindow.accepting { return }
        var observation = self.clock?.observe(hostTicks: when.isHostTimeValid ? when.hostTime : nil)
        var buffer = buffer
        var sourceFrameOffset: Int64 = 0
        if let testWindow = self.testWindow {
          guard let currentObservation = observation else {
            throw ProbeError.capture("five-second test microphone clock is missing")
          }
          guard
            let selected = try testWindow.selection(
              observation: currentObservation, frames: Int(buffer.frameLength),
              sampleRate: buffer.format.sampleRate, savedFrames: self.stats.frames)
          else { return }
          buffer = try testPCMFrames(buffer, range: selected.frames)
          sourceFrameOffset = Int64(selected.frames.lowerBound)
          // Keep the trimmed first frame on the same proven capture clock.
          // Do not substitute callback arrival time for hardware timestamps.
          observation = selected.observation
        }
        guard let file = self.file else {
          throw ProbeError.capture("microphone CAF writer is closed")
        }
        guard buffer.format.sampleRate == self.stats.sampleRate,
          buffer.format.channelCount == self.stats.channels,
          !when.isSampleTimeValid || when.sampleRate == self.stats.sampleRate
        else { throw ProbeError.capture("microphone format changed during capture") }
        var savedSourceFrame: Int64?
        if when.isSampleTimeValid {
          let (frame, overflow) = when.sampleTime.addingReportingOverflow(sourceFrameOffset)
          guard !overflow else { throw ProbeError.capture("microphone frame offset overflows") }
          savedSourceFrame = frame
        }
        if self.levelsEnabled { self.stats.inputLevel.receive(buffer, at: receivedAt) }
        try file.write(from: buffer)
        self.stats.callbackCount += 1
        self.stats.frames += Int64(buffer.frameLength)
        if when.isSampleTimeValid, when.sampleRate > 0 {
          let seconds = Double(savedSourceFrame ?? when.sampleTime) / when.sampleRate
          if self.stats.firstPresentationTimeSeconds == nil {
            self.stats.firstPresentationTimeSeconds = seconds
          }
          self.stats.lastPresentationTimeSeconds = seconds
        }
        if let observation {
          self.stats.recordSavedClock(
            observation, frames: Int64(buffer.frameLength),
            sourceFrame: savedSourceFrame, sourceEpoch: token.epoch)
        }
      } catch {
        self.stats.appendFailures += 1
      }
    }
    tapInstalled = true
    engine.prepare()
    try engine.start()
    try InputDevices.verifyDevice(unit, expected: selected.objectID, expectedUID: token.uid)
    try lock.withLock {
      guard !configurationChanged else {
        throw ProbeError.capture("microphone changed during restart")
      }
      try recovery?.activate(token)
    }
  }

  private func releaseEngine() {
    if let configurationObserver {
      NotificationCenter.default.removeObserver(configurationObserver)
      self.configurationObserver = nil
    }
    if tapInstalled, let engine {
      engine.inputNode.removeTap(onBus: 0)
      tapInstalled = false
    }
    engine?.stop()
    engine = nil
  }

  private func suspend(_ reason: String, unavailable: Bool = false) {
    lock.withLock {
      recovery?.interrupt(reason, unavailable: unavailable)
      inputUnit = nil
      selectedDeviceID = nil
      stats.inputLevel.invalidate()
    }
    releaseEngine()
    // The current CAF remains open. Recreating it would overwrite prior PCM.
  }

  func maintain() {
    // Formal bounded probes and a private 5-second test retain their existing
    // fail/verify contract; only an active production recording retries inputs.
    guard clock != nil, testWindow == nil,
      lock.withLock({ recovery?.status.state != "stopped" }), let uid = deviceUID
    else { return }
    let report: CaptureInputDevices
    do { report = try InputDevices.live() } catch {
      suspend("metadata_unavailable", unavailable: true)
      return
    }
    guard report.microphonePermission == "authorized" else {
      suspend("permission_unavailable", unavailable: true)
      return
    }
    let matches = report.devices.filter { $0.uid == uid }
    guard matches.count == 1 else {
      suspend(
        matches.isEmpty ? "device_disconnected" : "identity_ambiguous",
        unavailable: !matches.isEmpty)
      return
    }
    let selected = matches[0]
    let healthy = lock.withLock {
      recovery?.status.state == "connected" && !configurationChanged
        && selectedDeviceID == selected.objectID
    }
    if healthy, engine?.isRunning == true, let inputUnit {
      do {
        try InputDevices.verifyDevice(
          inputUnit, expected: selected.objectID, expectedUID: uid)
        return
      } catch {
        // Continue only after invalidating the old engine binding.
      }
    }
    suspend("engine_changed")
    let now = ContinuousClock.now
    if let lastRecoveryAttempt, now < lastRecoveryAttempt.advanced(by: .seconds(1)) { return }
    lastRecoveryAttempt = now
    do { try configure(selected) } catch RebindError.formatChanged {
      suspend("format_changed", unavailable: true)
    } catch { suspend("restart_failed", unavailable: true) }
  }

  func stop() {
    lock.withLock {
      recovery?.stop()
      inputUnit = nil
      selectedDeviceID = nil
      stats.inputLevel.invalidate()
    }
    releaseEngine()
    lock.withLock { file = nil }
  }

  func snapshot() -> TrackStats {
    lock.lock()
    defer { lock.unlock() }
    var result = stats
    result.microphoneRecovery = recovery?.status
    return result
  }
}

@main
private enum Main {
  static func main() async {
    do {
      let arguments = Array(CommandLine.arguments.dropFirst())
      if arguments.first == "--import-audio" {
        try await AudioImporter.command(arguments)
        return
      }
      if arguments == ["--list-apps"] {
        try await listApps()
        return
      }
      if arguments == ["--list-audio-processes"] {
        try listAudioProcesses()
        return
      }
      if arguments == ["--list-input-devices"] {
        let data = try JSONEncoder().encode(InputDevices.live())
        guard data.count <= 256 * 1024 else {
          throw ProbeError.capture("input device report is too large")
        }
        print(String(decoding: data, as: UTF8.self))
        return
      }
      if arguments == ["--list-capture-applications"] {
        let data = try JSONEncoder().encode(CaptureApplications.live())
        guard data.count <= 256 * 1024 else {
          throw ProbeError.capture("application report is too large")
        }
        print(String(decoding: data, as: UTF8.self))
        return
      }
      let options = try Options.parse(arguments)
      if let descriptor = options.captureTestLeaseFD, let owner = options.captureTestOwner {
        // Validate before inspecting inputs or opening any audio. Do not close
        // or unlock the inherited description: the kernel retains it through
        // every asynchronous callback and error path until this process exits.
        try CaptureTestLease.requireInherited(
          descriptor: descriptor, output: options.output, owner: owner)
      }
      if let identity = options.applicationIdentity { try CaptureApplications.current(identity) }
      if FileManager.default.fileExists(atPath: options.output.path) {
        let contents = try FileManager.default.contentsOfDirectory(atPath: options.output.path)
        guard contents.isEmpty else {
          throw ProbeError.usage("output directory already contains files")
        }
      }
      try FileManager.default.createDirectory(at: options.output, withIntermediateDirectories: true)
      defer {
        if options.inputLevelOwner != nil {
          try? FileManager.default.removeItem(
            at: options.output.appendingPathComponent("input-levels.json"))
        }
        if options.publishesInputStatus {
          try? FileManager.default.removeItem(
            at: options.output.appendingPathComponent("input-status.json"))
        }
      }
      let manifestURL = options.output.appendingPathComponent("manifest.json")
      // Production recordings require capture-time proof. Bounded measurement
      // probes retain schema 1 until their formal evaluation harness is updated.
      let clock =
        options.duration == nil || options.captureTestOwner != nil
        ? try CaptureClockDomain.live() : nil
      let testWindow = try options.captureTestOwner.map { try CaptureTestWindow(ownerId: $0) }
      let mic = MicrophoneRecorder(
        outputURL: options.output.appendingPathComponent("microphone.caf"), clock: clock,
        microphoneUID: options.microphoneUID, levelsEnabled: options.inputLevelOwner != nil,
        testWindow: testWindow)
      let systemStopRequest = options.output.appendingPathComponent("stop.request")
      let systemParentPID = options.parentPID
      let systemControlled = options.stopOnFile
      let system = SystemAudioRecorder(
        outputURL: options.output.appendingPathComponent("system.caf"), clock: clock,
        levelsEnabled: options.inputLevelOwner != nil, testWindow: testWindow,
        retries: options.recoversSystemAudio,
        mayContinue: {
          !systemControlled
            || (!FileManager.default.fileExists(atPath: systemStopRequest.path)
              && !parentHasExited(systemParentPID))
        })
      let tap = CoreAudioTapRecorder(
        outputURL: options.output.appendingPathComponent("system.caf"), clock: clock,
        levelsEnabled: options.inputLevelOwner != nil)
      var manifest = Manifest(
        schemaVersion: clock == nil ? 1 : 2,
        scenario: options.scenario,
        captureMode: options.mode,
        backend: options.backend,
        targetPID: options.targetPIDs.first,
        targetPIDs: options.targetPIDs,
        targetApplicationName: nil,
        targetBundleIdentifier: nil,
        startedAtUTC: ISO8601DateFormatter().string(from: Date()),
        requestedDurationSeconds: options.duration,
        osVersion: ProcessInfo.processInfo.operatingSystemVersionString,
        architecture: architecture(),
        state: "starting",
        microphone: TrackStats(),
        system: TrackStats(),
        errors: []
      )
      manifest.captureTest = testWindow?.metadata()
      manifest.recordingOwnerId = options.inputLevelOwner
      try write(manifest, to: manifestURL)
      let power = RecordingPowerAssertion()
      // Also covers a throw while writing the final manifest. Do not depend on
      // a WebView, the main window, or normal application termination to release.
      defer { try? power.release() }
      do {
        try power.acquire()
        manifest.power.acquired = true
        try await mic.start()
        manifest.microphoneDeviceUID = mic.deviceUID
        manifest.microphone = mic.snapshot()
        if options.backend == "coreaudio" {
          try tap.start(targetPIDs: options.targetPIDs)
        } else {
          try await system.start(
            targetPID: options.targetPIDs.first, applicationIdentity: options.applicationIdentity)
          manifest.targetApplicationIdentity = options.applicationIdentity
          manifest.targetApplicationName = system.targetApplicationName
          manifest.targetBundleIdentifier = system.targetBundleIdentifier
        }
        manifest.state = "recording"
        if testWindow != nil,
          FileManager.default.fileExists(
            atPath: options.output.appendingPathComponent("stop.request").path)
            || parentHasExited(options.parentPID)
        {
          throw ProbeError.capture("five-second test was cancelled during initialization")
        }
        try testWindow?.open(at: PCMInputLevel.nowNanoseconds())
        manifest.captureTest = testWindow?.metadata()
        try write(manifest, to: manifestURL)
        if options.stopOnFile {
          let stopRequest = options.output.appendingPathComponent("stop.request")
          let startedAt = ContinuousClock.now
          var levelRevision: Int64 = 0
          while options.shouldContinueRecording(
            startedAt: startedAt, now: .now,
            stopRequested: FileManager.default.fileExists(atPath: stopRequest.path),
            parentExited: parentHasExited(options.parentPID))
          {
            mic.maintain()
            if options.backend == "screencapturekit" { system.maintain() }
            let microphone = mic.snapshot()
            let systemStats = options.backend == "coreaudio" ? tap.snapshot() : system.snapshot()
            if microphone.microphoneRecovery != manifest.microphone.microphoneRecovery
              || systemStats.systemRecovery != manifest.system.systemRecovery
            {
              manifest.microphone = microphone
              manifest.system = systemStats
              try write(manifest, to: manifestURL)
            }
            if let owner = options.inputLevelOwner, levelRevision < 9_007_199_254_740_991 {
              levelRevision += 1
              if options.publishesInputStatus,
                let status = microphone.microphoneRecovery
              {
                try? InputStatusReport(
                  ownerId: owner, revision: levelRevision, microphone: status,
                  system: systemStats.systemRecovery
                )
                .publish(to: options.output)
              }
              let microphone = microphone.inputLevel
              let systemLevel =
                (options.backend == "coreaudio" ? tap.snapshot() : system.snapshot())
                .inputLevel
              let now = PCMInputLevel.nowNanoseconds()
              // Failure to publish telemetry must not stop or declare a saved recording.
              try? InputLevelReport(
                ownerId: owner, revision: levelRevision, microphone: microphone.snapshot(at: now),
                system: systemLevel.snapshot(at: now)
              ).publish(to: options.output)
            }
            try await Task.sleep(for: .milliseconds(options.inputLevelOwner == nil ? 200 : 100))
          }
        } else {
          guard let duration = options.duration else {
            throw ProbeError.usage("recording without a duration requires a stop mechanism")
          }
          try await Task.sleep(for: .seconds(duration))
        }
        testWindow?.finish(at: PCMInputLevel.nowNanoseconds())
        mic.stop()
        if options.backend == "coreaudio" { try tap.stop() } else { try await system.stop() }
        let micStats = mic.snapshot()
        let systemStats = options.backend == "coreaudio" ? tap.snapshot() : system.snapshot()
        if let testWindow, !testWindow.metadata().completed {
          throw ProbeError.capture("five-second test stopped before its window completed")
        }
        guard micStats.frames > 0, systemStats.frames > 0,
          micStats.appendFailures == 0, systemStats.appendFailures == 0
        else {
          throw ProbeError.capture("audio track is empty or contains failed writes")
        }
        try verifyRecordedCAF(
          options.output.appendingPathComponent("microphone.caf"), expected: micStats)
        try verifyRecordedCAF(
          options.output.appendingPathComponent("system.caf"), expected: systemStats)
        manifest.microphone = micStats
        manifest.system = systemStats
        if let clock { try manifest.finalizeClock(clock) }
        manifest.state = "saved"
      } catch {
        testWindow?.finish(at: PCMInputLevel.nowNanoseconds())
        mic.stop()
        do {
          if options.backend == "coreaudio" { try tap.stop() } else { try await system.stop() }
        } catch { manifest.errors.append(String(describing: error)) }
        manifest.state = "failed"
        manifest.errors.append(String(describing: error))
      }
      do {
        try power.release()
        manifest.power.released = manifest.power.acquired
      } catch {
        manifest.state = "failed"
        manifest.errors.append(String(describing: error))
      }
      manifest.endedAtUTC = ISO8601DateFormatter().string(from: Date())
      manifest.captureTest = testWindow?.metadata()
      if manifest.state != "saved" {
        manifest.microphone = mic.snapshot()
        manifest.system = options.backend == "coreaudio" ? tap.snapshot() : system.snapshot()
      } else if let testWindow, let micStart = manifest.microphone.firstCaptureNanoseconds,
        let systemStart = manifest.system.firstCaptureNanoseconds
      {
        manifest.captureTest = testWindow.metadata(
          firstSavedCaptureNanoseconds: min(micStart, systemStart))
      }
      try write(manifest, to: manifestURL)
      if manifest.state != "saved" {
        throw ProbeError.capture(manifest.errors.joined(separator: "; "))
      }
      print(manifestURL.path)
    } catch ProbeError.help {
      print(usage)
    } catch {
      FileHandle.standardError.write(Data("error: \(error)\n".utf8))
      FileHandle.standardError.write(Data(usage.utf8))
      Foundation.exit(2)
    }
  }

  private static let usage = """
    Usage: mon-macos-capture --list-apps
           mon-macos-capture --list-capture-applications
           mon-macos-capture --list-audio-processes
           mon-macos-capture --list-input-devices
           mon-macos-capture --output <directory> [--duration <seconds>]
             [--scenario <id>] [--mode system|application] [--pid <process-id> ...]
             [--backend screencapturekit|coreaudio] [--stop-on-file] [--parent-pid <pid>]
             [--until-stopped] [--microphone-uid <stable-device-uid>] [--application-identity <json>]
             [--input-level-owner <session-uuid>]
             [--five-second-test <test-uuid>]
             [--capture-test-lease-fd <inherited-descriptor>]
    --until-stopped requires --stop-on-file and --parent-pid, without --duration.
    --five-second-test requires --stop-on-file, --parent-pid and --microphone-uid.
    Its shared 5-second capture window opens only after both inputs initialize.
    A supplied capture-test lease must be inherited, private, locked and bound to that test's output.
    Records microphone.caf and system.caf after the required macOS permissions are granted.
    """

  private static func parentHasExited(_ pid: pid_t?) -> Bool {
    guard let pid else { return false }
    return Darwin.kill(pid, 0) != 0 && errno == ESRCH
  }

  private struct ListedApplication: Encodable {
    let processID: pid_t
    let name: String
    let bundleIdentifier: String
  }

  private static func listApps() async throws {
    let content = try await SCShareableContent.excludingDesktopWindows(
      false, onScreenWindowsOnly: false)
    let applications = content.applications.map {
      ListedApplication(
        processID: $0.processID, name: $0.applicationName,
        bundleIdentifier: $0.bundleIdentifier)
    }.sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
    let data = try encoder.encode(applications)
    print(String(decoding: data, as: UTF8.self))
  }

  private struct ListedAudioProcess: Encodable {
    let processID: pid_t
    let audioObjectID: AudioObjectID
    let name: String?
    let bundleIdentifier: String?
  }

  private static func listAudioProcesses() throws {
    let processes = try TapAudioProcesses.list().map { item in
      let application = NSRunningApplication(processIdentifier: item.pid)
      return ListedAudioProcess(
        processID: item.pid, audioObjectID: item.objectID,
        name: application?.localizedName, bundleIdentifier: application?.bundleIdentifier)
    }
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
    print(String(decoding: try encoder.encode(processes), as: UTF8.self))
  }

  private static func write(_ manifest: Manifest, to url: URL) throws {
    try manifest.encodedData().write(to: url, options: .atomic)
  }

  private static func architecture() -> String {
    #if arch(arm64)
      return "arm64"
    #elseif arch(x86_64)
      return "x86_64"
    #else
      return "unknown"
    #endif
  }
}
