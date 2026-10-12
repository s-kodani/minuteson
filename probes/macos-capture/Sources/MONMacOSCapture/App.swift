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
  // Bounded multi-PID tap probes retain their independent measurement path.
  // The application and its five-second test use the shared owned writer.
  var usesStandaloneTapProbe: Bool {
    backend == "coreaudio" && !targetPIDs.isEmpty && applicationIdentity == nil
  }
  var recoversSystemAudio: Bool {
    duration == nil && captureTestOwner == nil && !usesStandaloneTapProbe
      && (targetPIDs.isEmpty || applicationIdentity != nil)
  }
  var recoversCaptureClock: Bool { publishesInputStatus && recoversSystemAudio }
  // Creating the tap aggregate may notify existing microphone engines. A
  // private test opens no save window until both explicitly requested inputs
  // are ready, so prepare its tap before activating the microphone.
  var startsSystemBeforeMicrophone: Bool { captureTestOwner != nil && backend == "coreaudio" }
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
    if backend == "screencapturekit" && targetPIDs.count > 1 {
      throw ProbeError.usage("multiple --pid values require the coreaudio backend")
    }
    if Set(targetPIDs).count != targetPIDs.count {
      throw ProbeError.usage("duplicate --pid values are not allowed")
    }
    if let identity = applicationIdentity {
      guard mode == "application", targetPIDs == [identity.pid]
      else {
        throw ProbeError.usage(
          "application identity requires the exact target PID")
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
        !durationSpecified, !untilStopped, inputLevelOwner == nil,
        mode == "system" || applicationIdentity != nil
      else {
        throw ProbeError.usage(
          "five-second test requires stop control, parent PID, microphone UID and exact application identity when selected; duration, until-stopped and input-level-owner are not accepted"
        )
      }
      duration = 5
      inputLevelOwner = captureTestOwner
    }
    if captureTestLeaseFD != nil && captureTestOwner == nil {
      throw ProbeError.usage("an inherited test lease is only valid for a five-second test")
    }
    return Self(
      // Foundation rewrites /private/tmp and /private/var into symlink aliases
      // even when the native owner supplied a canonical path. Ordinary owned
      // recordings retain that path for descriptor-relative journal pinning.
      // Bounded probes/private tests keep their existing path contract.
      output: untilStopped && captureTestOwner == nil && inputLevelOwner != nil
        ? output : output.standardizedFileURL, duration: untilStopped ? nil : duration,
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
  // Diagnostic only; keep the manifest track schema unchanged. The initiating
  // failure is carried by requireWritable into manifest.errors instead.
  var captureFailureDetail: String?
  var sampleRate: Double?
  var channels: UInt32?
  var clockSegments: [CapturedClockSegment]?
  var clockFailure: String?
  var timing: CaptureTrackTiming?
  var clockCapacity = 1024
  var inputLevel = PCMInputLevel()
  var microphoneRecovery: MicrophoneRecovery.Status?
  var systemRecovery: SystemAudioRecovery.Status?
  // Snapshot under the same policy lock. Never mix a newer event suffix with
  // an older current state, and do not add telemetry to the audio manifest.
  var inputEventHistory: InputEventHistory?
  var rawSegments: RawSegmentStatus?

  enum CodingKeys: String, CodingKey {
    case callbackCount, frames, firstPresentationTimeSeconds, lastPresentationTimeSeconds
    case appendFailures, sampleRate, channels, clockSegments, clockFailure
    case microphoneRecovery, systemRecovery
    case rawSegments
  }

  var firstCaptureNanoseconds: Int64? { timing?.firstCaptureNanoseconds }

  // Input loss may be recoverable, but a broken storage/clock contract is not.
  // This is derived from existing evidence, not a successful capture closure.
  var captureFailure: String? {
    if let failure = rawSegments?.failure { return failure }
    if clockFailure != nil { return "capture_clock_failed" }
    if appendFailures > 0 { return "capture_audio_write_failed" }
    return nil
  }

  func requireWritable(raw: RawSegmentStatus? = nil) throws {
    if let failure = raw?.failure ?? captureFailure {
      let detail = captureFailureDetail.map { " 原因: \($0)" } ?? ""
      throw ProbeError.capture(CaptureFailureGuide.message(failure) + detail)
    }
  }

  mutating func retainCaptureFailure(_ error: Error) {
    guard captureFailureDetail == nil else { return }
    let detail = String(describing: error).trimmingCharacters(in: .whitespacesAndNewlines)
    if !detail.isEmpty { captureFailureDetail = String(detail.prefix(1200)) }
  }

  // The callback must use the same full processing format as a rebind. Equal
  // rates/channels alone do not prove sample representation or buffer layout.
  func acceptsMicrophonePCM(
    _ buffer: AVAudioPCMBuffer, at when: AVAudioTime, processingFormat: AVAudioFormat
  ) -> Bool {
    MicrophonePCMFormat.matches(processingFormat, buffer.format)
      && buffer.format.sampleRate == sampleRate
      && buffer.format.channelCount == channels
      && (!when.isSampleTimeValid || when.sampleRate == sampleRate)
  }

  // Check clock bounds before writing the corresponding PCM. If writing later
  // fails, the lane becomes terminal and cannot publish this timing as saved.
  mutating func prepareSavedClock(
    _ observation: CaptureClockObservation?, frames: Int64, sourceFrame: Int64?,
    sourceEpoch: UInt64 = 0
  ) throws {
    try requireWritable()
    if let observation {
      recordSavedClock(
        observation, frames: frames, sourceFrame: sourceFrame, sourceEpoch: sourceEpoch)
      try requireWritable()
    }
  }

  mutating func recordSavedClock(
    _ observation: CaptureClockObservation, frames: Int64, sourceFrame: Int64?,
    sourceEpoch: UInt64 = 0
  ) {
    guard clockFailure == nil else { return }
    do {
      if timing == nil {
        guard let sampleRate else { throw ProbeError.capture("capture sample rate is missing") }
        timing = try CaptureTrackTiming(sampleRate: sampleRate, maximumSegments: clockCapacity)
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
  var systemAudioUnreceived: Bool? = nil
  var recordingOwnerId: String? = nil
  var captureEnd: CaptureEndMetadata? = nil
  // Notification retention failure is separate from saved PCM/clock failure.
  // Keep audio finalization possible and retain a bounded diagnostic for the
  // native recovery UI; it must not claim all notifications were committed.
  var inputEventJournalError: String? = nil
  var inputEventJournalPersistence: InputJournalPersistence? = nil

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

  mutating func finalizeTestClock(_ clock: CaptureClockDomain, window: CaptureTestWindow) throws {
    // The manifest still contains the snapshot published when the window
    // opened. Finalize against this same owned window's completion snapshot,
    // not its deliberately incomplete starting snapshot.
    let completed = window.metadata()
    guard let previous = captureTest, previous.ownerId == completed.ownerId,
      previous.windowStartNs == completed.windowStartNs,
      previous.windowEndNs == completed.windowEndNs
    else { throw ProbeError.capture("capture test completion does not match its starting window") }
    guard schemaVersion == 2, completed.completed == true,
      let micStart = microphone.firstCaptureNanoseconds
    else { throw ProbeError.capture("completed test and microphone capture clock are required") }
    captureTest = completed
    if system.frames > 0 {
      try finalizeClock(clock)
      return
    }
    try system.requireWritable()
    guard system.callbackCount == 0, system.sampleRate == nil, system.channels == nil,
      system.firstCaptureNanoseconds == nil, system.captureFailureDetail == nil
    else { throw ProbeError.capture("empty system test track has inconsistent capture evidence") }
    try microphone.finishClock(originNanoseconds: micStart)
    system.clockSegments = []
    captureClock = try clock.metadata(originNanoseconds: micStart)
    systemAudioUnreceived = true
  }

  mutating func finalizeStoppedClock(
    _ clock: CaptureClockDomain, microphoneClosure: CaptureClosure?, systemClosure: CaptureClosure?
  ) throws {
    guard schemaVersion == 2, captureTest == nil,
      let owner = recordingOwnerId, InputLevelReport.validOwner(owner),
      let micStart = microphone.firstCaptureNanoseconds,
      let microphoneClosure, let systemClosure
    else { throw ProbeError.capture("recording owner and both acquisition closures are required") }
    let unreceived = system.frames == 0
    if unreceived {
      try system.requireWritable()
      guard system.callbackCount == 0, system.sampleRate == nil, system.channels == nil,
        system.firstCaptureNanoseconds == nil, system.captureFailureDetail == nil,
        let raw = system.rawSegments, raw.ownerId == owner, raw.lane == "system",
        raw.producerClosed, raw.sealedSegments == 0, raw.sealedFrames == 0,
        raw.lastEntrySHA256 == nil, raw.failure == nil,
        system.systemRecovery?.state == "stopped"
      else { throw ProbeError.capture("unreceived system track has inconsistent closure evidence") }
      system.clockSegments = []
      systemAudioUnreceived = true
    }
    guard unreceived || system.firstCaptureNanoseconds != nil else {
      throw ProbeError.capture("system capture clock is missing")
    }
    let origin = min(micStart, system.firstCaptureNanoseconds ?? micStart)
    try microphone.finishClock(originNanoseconds: origin)
    if !unreceived { try system.finishClock(originNanoseconds: origin) }
    let (metadata, ending) = try clock.ending(
      originNanoseconds: origin, microphone: microphoneClosure, system: systemClosure)
    guard let micEnd = microphone.clockSegments?.last?.timelineEndUs,
      let pcEnd = unreceived ? 0 : system.clockSegments?.last?.timelineEndUs,
      micEnd <= ending.microphoneClosedUs, pcEnd <= ending.systemClosedUs
    else { throw ProbeError.capture("saved PCM extends beyond its acquisition closure") }
    captureClock = metadata
    captureEnd = ending
    schemaVersion = 3
  }

  func encodedData() throws -> Data {
    let segmented =
      recordingOwnerId.map { owner in
        InputLevelReport.validOwner(owner)
          && microphone.rawSegments?.ownerId == owner
          && microphone.rawSegments?.lane == "microphone"
          && system.rawSegments?.ownerId == owner && system.rawSegments?.lane == "system"
      } ?? false
    let extended = segmented && (2...3).contains(schemaVersion)
    let encoder = JSONEncoder()
    encoder.outputFormatting = extended ? [.sortedKeys] : [.prettyPrinted, .sortedKeys]
    let data = try encoder.encode(self)
    let limit = extended ? 32 * 1024 * 1024 : 256 * 1024
    guard data.count <= limit else {
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
  private var stopFailure: String?
  private var captureClosure: CaptureClosure?
  private var clockRecoveryRequest: CaptureClockRecoveryRequest?
  private var clockStopReceipt: StopReceipt?
  private let retries: Bool
  private let backend: String
  private let clock: CaptureClockDomain?
  private let levelsEnabled: Bool
  private let testWindow: CaptureTestWindow?
  private let factory: Factory?
  private let now: @Sendable () -> Int64?
  private let requireIdentity: @Sendable (CaptureApplicationIdentity) throws -> Void
  private let mayContinue: @Sendable () -> Bool
  private let inputEventObserver: InputEventJournal.Observer?
  private(set) var targetApplicationName: String?
  private(set) var targetBundleIdentifier: String?

  init(
    outputURL: URL, clock: CaptureClockDomain? = nil, levelsEnabled: Bool = false,
    testWindow: CaptureTestWindow? = nil, retries: Bool = false,
    backend: String = "screencapturekit",
    factory: Factory? = nil, now: @escaping @Sendable () -> Int64? = PCMInputLevel.nowNanoseconds,
    requireIdentity: @escaping @Sendable (CaptureApplicationIdentity) throws -> Void =
      CaptureApplications.current,
    mayContinue: @escaping @Sendable () -> Bool = { true },
    inputEventObserver: InputEventJournal.Observer? = nil,
    rawSegments: RawRecordingSegments? = nil
  ) {
    self.outputURL = outputURL
    self.pcm = SystemPCMWriter(outputURL: outputURL, rawSegments: rawSegments)
    self.clock = clock
    self.levelsEnabled = levelsEnabled
    self.testWindow = testWindow
    self.retries = retries && clock != nil && testWindow == nil
    self.backend = backend
    self.factory = factory
    self.now = now
    self.requireIdentity = requireIdentity
    self.mayContinue = mayContinue
    self.inputEventObserver = inputEventObserver
  }

  private var applicationIdentity: CaptureApplicationIdentity?

  func start(targetPID: pid_t?, applicationIdentity: CaptureApplicationIdentity? = nil) async throws
  {
    let policy = try SystemAudioRecovery(
      targetPID: targetPID, applicationIdentity: applicationIdentity, retries: retries,
      observe: inputEventObserver)
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
        // A callback may fail before start() returns. Keep its recorded reason
        // rather than replacing it with the activation-state summary.
        try pcm.stats.requireWritable()
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
    if backend == "coreaudio" {
      guard targetPID == nil || applicationIdentity != nil else {
        throw ProbeError.capture("audio-only application capture requires its exact identity")
      }
      let scope = applicationIdentity.map(CoreAudioTapScope.application) ?? .system
      let allowed: @Sendable () -> Bool = { [weak self] in
        guard let self else { return false }
        return self.mayContinue()
          && self.lock.withLock { !self.stopping && self.recovery?.owns(token) == true }
      }
      return CoreAudioCaptureStream(
        driver: NativeCoreAudioCaptureDriver(scope: scope, mayContinue: allowed),
        bundleIdentifier: applicationIdentity?.bundleIdentifier,
        mayContinue: allowed,
        deliver: { [weak self] source, sample in
          self?.receive(from: source, sampleBuffer: sample)
        }, ended: { [weak self] source, error in self?.sourceEnded(source, error: error) })
    }
    let content: SCShareableContent
    do {
      content = try await SCShareableContent.excludingDesktopWindows(
        false, onScreenWindowsOnly: false)
    } catch {
      throw CaptureFailureGuide.systemCaptureFailure(error)
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

  /// Never stop acquisition on its delegate/sample queue. Awaiting OS
  /// work happens in this owned task, leaving the microphone/control loop free.
  private func retireStream() async throws {
    let old = lock.withLock { (stream, captureStarted && !streamEnded) }
    if let stream = old.0 {
      if old.1 { try await stream.stop() }
      // Successful stop and successful output removal are separate facts.
      // Keep an undetached source owned, but never stop it a second time just
      // because removal failed. A replacement cannot be created until detach
      // succeeds and its admitted callback queue has drained.
      lock.withLock {
        if self.stream === stream {
          captureStarted = false
          streamEnded = true
        }
      }
      try stream.detach()
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
    guard retries, mayContinue(),
      lock.withLock({
        !stopping && clockRecoveryRequest == nil && pcm.stats.captureFailure == nil
      })
    else { return }
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
          if !retirementOnly, lock.withLock({ streamToken == token }) {
            try? await retireStream()
          }
        }
      }
    }
  }

  func stream(_ stream: SCStream, didStopWithError error: Error) {
    if let source = lock.withLock({ self.stream?.native === stream ? self.stream : nil }) {
      sourceEnded(source, error: error)
    }
  }

  func sourceEnded(_ stream: any SystemCaptureStream, error: Error? = nil) {
    lock.withLock {
      guard self.stream === stream, let token = streamToken else { return }
      streamEnded = true
      guard !stopping, recovery?.owns(token) == true else { return }
      recovery?.interrupt("stream_stopped")
      pcm.invalidateLevels()
      if !retries { pcm.recordFailure(error) }
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

  /// Called only by the recording control loop, never on an SCStream queue.
  /// Keep the writer open, but do not release the clock barrier until all owned
  /// acquisition work and the actual OS stop have returned successfully.
  func suspendForClockRecovery(_ request: CaptureClockRecoveryRequest) async throws
    -> CaptureClockQuiescence
  {
    guard let clock, try clock.recoveryRequest() == request else {
      throw ProbeError.capture("system clock recovery request is obsolete")
    }
    try lock.withLock {
      try pcm.stats.requireWritable()
      guard retries, !stopping, clockRecoveryRequest == nil
      else { throw ProbeError.capture("system clock recovery is not available") }
      clockRecoveryRequest = request
      recovery?.interrupt("clock_changed")
      pcm.invalidateLevels()
      operationTask?.cancel()
    }
    let deadline = ContinuousClock.now.advanced(by: .seconds(5))
    while lock.withLock({ recovery?.operation != nil }), ContinuousClock.now < deadline {
      try await Task.sleep(for: .milliseconds(10))
    }
    guard !lock.withLock({ recovery?.operation != nil }) else {
      throw ProbeError.capture("system acquisition is still pending during clock recovery")
    }
    let retirement = try lock.withLock {
      guard !stopping, let token = try recovery?.beginRetirement(at: now() ?? -1) else {
        throw ProbeError.capture("system clock retirement cannot be owned")
      }
      return token
    }
    defer { lock.withLock { recovery?.operationFinished(retirement) } }
    if let active = lock.withLock({ captureStarted && !streamEnded ? stream : nil }) {
      let receipt = StopReceipt()
      lock.withLock { clockStopReceipt = receipt }
      active.requestStop { receipt.complete($0) }
      let deadline = ContinuousClock.now.advanced(by: .seconds(5))
      while !receipt.read().0, ContinuousClock.now < deadline {
        try await Task.sleep(for: .milliseconds(10))
      }
      let result = receipt.read()
      guard result.0, result.1 == nil else {
        throw ProbeError.capture("system stream did not stop for clock recovery")
      }
    }
    if let source = lock.withLock({ stream }) { try source.detach() }
    queue.sync {}
    try lock.withLock {
      guard !stopping else { throw ProbeError.capture("system clock recovery was stopped") }
      stream = nil
      streamToken = nil
      captureStarted = false
      streamEnded = true
      clockStopReceipt = nil
    }
    return try clock.quiesced(.system, for: request)
  }

  func resumeAfterClockRecovery(_ request: CaptureClockRecoveryRequest) throws {
    guard let clock, try clock.recoveryRequest() == nil else {
      throw ProbeError.capture("system clock has not resumed")
    }
    try lock.withLock {
      guard !stopping, clockRecoveryRequest == request else {
        throw ProbeError.capture("system clock recovery result is obsolete")
      }
      clockRecoveryRequest = nil
    }
    // The normal single-flight path verifies the same frozen application scope.
    maintain()
  }

  func stop() async throws { _ = try await finishStop(proveEnding: false) }

  func stopRecording() async throws -> CaptureClosure {
    guard let receipt = try await finishStop(proveEnding: true) else {
      throw ProbeError.capture("system acquisition has no proven closure")
    }
    return receipt
  }

  private func finishStop(proveEnding: Bool) async throws -> CaptureClosure? {
    let (ownsStop, previous) = lock.withLock { () -> (Bool, SystemAudioRecovery.Status?) in
      guard !stopping else { return (false, nil) }
      stopping = true
      let status = recovery?.status
      if let token = streamToken, recovery?.accepts(token) == true { drainingToken = token }
      recovery?.stop()
      operationTask?.cancel()
      return (true, status)
    }
    if !ownsStop {
      while !lock.withLock({ finished }) { try await Task.sleep(for: .milliseconds(10)) }
      if let failure = lock.withLock({ stopFailure }) { throw ProbeError.capture(failure) }
      return lock.withLock { captureClosure }
    }
    defer { lock.withLock { finished = true } }
    var streamStopError: String?
    let deadline = ContinuousClock.now.advanced(by: .seconds(5))
    while lock.withLock({ recovery?.operation != nil }), ContinuousClock.now < deadline {
      try? await Task.sleep(for: .milliseconds(10))
    }
    if lock.withLock({ recovery?.operation != nil }) {
      streamStopError =
        "system acquisition is still pending; retained audio is not published as saved"
    } else if let active = lock.withLock({ captureStarted && !streamEnded ? stream : nil }) {
      let pending = lock.withLock { clockStopReceipt }
      let receipt = pending ?? StopReceipt()
      if pending == nil { active.requestStop { receipt.complete($0) } }
      let deadline = ContinuousClock.now.advanced(by: .seconds(5))
      while !receipt.read().0, ContinuousClock.now < deadline {
        try? await Task.sleep(for: .milliseconds(10))
      }
      let result = receipt.read()
      streamStopError = result.0 ? result.1 : "system stream stop timed out"
    }
    queue.sync {}
    if streamStopError == nil, let source = lock.withLock({ stream }),
      lock.withLock({ recovery?.operation == nil })
    {
      do { try source.detach() } catch {
        streamStopError = streamStopError ?? "system source detach failed: \(error)"
      }
    }
    if streamStopError == nil, proveEnding {
      do {
        guard let clock else { throw ProbeError.capture("system capture clock is missing") }
        let receipt = try clock.close(.system)
        lock.withLock { captureClosure = receipt }
      } catch { streamStopError = String(describing: error) }
    }
    lock.withLock {
      drainingToken = nil
      captureStarted = false
      pcm.invalidateLevels()
    }
    let allowUnreceivedAudio =
      (proveEnding && lock.withLock { captureClosure != nil }
        || !proveEnding && testWindow?.metadata().completed == true)
      && streamStopError == nil && previous?.state == "connected"
    do { try await pcm.finish(allowUnreceivedAudio: allowUnreceivedAudio) } catch {
      let failure =
        streamStopError.map { "system stop failed: \($0); writer finalization: \(error)" }
        ?? String(describing: error)
      lock.withLock { stopFailure = failure }
      throw ProbeError.capture(failure)
    }
    if let streamStopError {
      lock.withLock { stopFailure = streamStopError }
      throw ProbeError.capture("system stop failed after CAF finalization: \(streamStopError)")
    }
    if !proveEnding, retries, previous?.state != "connected" {
      let failure = "system input did not recover before stop; trailing missing time is unresolved"
      lock.withLock { stopFailure = failure }
      throw ProbeError.capture(failure)
    }
    return lock.withLock { captureClosure }
  }

  func snapshot() -> TrackStats {
    lock.lock()
    defer { lock.unlock() }
    var result = pcm.stats
    result.systemRecovery = recovery?.status
    result.inputEventHistory = recovery?.history
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
          if !retries { pcm.recordFailure(error) }
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
          clock: stream.synchronizationClock)
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
          clock: stream.synchronizationClock)
      if case .suspended = observation {
        recovery?.interrupt("clock_changed")
        pcm.invalidateLevels()
        return
      }
      try pcm.append(
        sampleBuffer, observation: observation, sourceEpoch: token.epoch,
        levelsEnabled: levelsEnabled, receivedAt: receivedAt)
    } catch SystemPCMWriter.AppendError.formatChanged {
      recovery?.interrupt("format_changed", unavailable: true)
      pcm.invalidateLevels()
      if !retries { pcm.recordFailure(ProbeError.capture("PC音声の保存中に音声形式が変わりました")) }
    } catch {
      pcm.retainFailure(error)
      if pcm.stats.captureFailure == nil { pcm.recordFailure() }
    }
  }

}

final class MicrophoneRecorder: @unchecked Sendable {
  private(set) var captureClosure: CaptureClosure?
  private enum RebindError: Error { case formatChanged }
  private var engine: (any MicrophoneCaptureEngine)?
  private var configurationChanged = false
  private var configurationRevision: UInt64 = 0
  private var recovery: MicrophoneRecovery?
  private var lastRecoveryAttempt: ContinuousClock.Instant?
  private struct NormalPreparation {
    let token: MicrophoneRecovery.Binding
    let format: AVAudioFormat
    var deadline: ContinuousClock.Instant?
    var nextProof: ContinuousClock.Instant
    var revision: UInt64?
    var stableProofs = 0
    var restarts = 0
  }
  private var normalPreparation: NormalPreparation?
  // A stable engine may admit its first real PCM while still reconnecting.
  // Connection/recovery is published only after that PCM has been saved.
  private var normalAdmission: MicrophoneRecovery.Binding?
  private let outputURL: URL
  private let lock = NSLock()
  private var file: AVAudioFile?
  private var tapInstalled = false
  private var stats = TrackStats()
  private let clock: CaptureClockDomain?
  private let levelsEnabled: Bool
  private let testWindow: CaptureTestWindow?

  private let requestedUID: String?
  private let inputEventObserver: InputEventJournal.Observer?
  private let makeEngine: @Sendable () throws -> any MicrophoneCaptureEngine
  private let devices: @Sendable () throws -> CaptureInputDevices
  private let permission: @Sendable () -> AVAuthorizationStatus
  private let requestPermission: @Sendable () async -> Bool
  private let rawSegments: RawRecordingSegments?
  private let mayContinue: @Sendable () -> Bool
  private let controlNow: @Sendable () -> ContinuousClock.Instant
  private var selectedDeviceID: AudioObjectID?
  private(set) var deviceUID: String?

  init(
    outputURL: URL, clock: CaptureClockDomain? = nil, microphoneUID: String? = nil,
    levelsEnabled: Bool = false, testWindow: CaptureTestWindow? = nil,
    inputEventObserver: InputEventJournal.Observer? = nil,
    rawSegments: RawRecordingSegments? = nil,
    mayContinue: @escaping @Sendable () -> Bool = { true },
    controlNow: @escaping @Sendable () -> ContinuousClock.Instant = { .now },
    makeEngine: @escaping @Sendable () throws -> any MicrophoneCaptureEngine = {
      NativeMicrophoneCaptureEngine()
    },
    devices: @escaping @Sendable () throws -> CaptureInputDevices = { try InputDevices.live() },
    permission: @escaping @Sendable () -> AVAuthorizationStatus = {
      AVCaptureDevice.authorizationStatus(for: .audio)
    },
    requestPermission: @escaping @Sendable () async -> Bool = {
      await AVCaptureDevice.requestAccess(for: .audio)
    }
  ) {
    self.outputURL = outputURL
    self.clock = clock
    self.requestedUID = microphoneUID
    self.levelsEnabled = levelsEnabled
    self.testWindow = testWindow
    self.inputEventObserver = inputEventObserver
    self.rawSegments = rawSegments
    self.mayContinue = mayContinue
    self.controlNow = controlNow
    self.makeEngine = makeEngine
    self.devices = devices
    self.permission = permission
    self.requestPermission = requestPermission
    stats.clockCapacity = rawSegments == nil ? 1024 : CapturedClockMap.maximumTrackSegments
  }

  func start() async throws {
    let selected = try InputDevices.choose(requestedUID, from: devices().devices)
    var authorization = permission()
    if authorization == .notDetermined {
      _ = await requestPermission()
      authorization = permission()
    }
    guard authorization == .authorized else {
      throw ProbeError.permission(
        CaptureFailureGuide.microphonePermission(authorization)
      )
    }
    try lock.withLock {
      guard recovery == nil, mayContinue() else {
        throw ProbeError.capture("microphone start is already owned or stopped")
      }
      recovery = try MicrophoneRecovery(uid: selected.uid, observe: inputEventObserver)
      deviceUID = selected.uid
    }
    do {
      try configure(selected)
    } catch {
      stop()
      throw ProbeError.capture(CaptureFailureGuide.microphoneInitialization(error))
    }
  }

  private func configure(_ selected: CaptureInputDevice) throws {
    guard mayContinue() else { throw ProbeError.capture("microphone stop was requested") }
    let token = try lock.withLock {
      guard let token = try recovery?.begin(selected) else {
        throw ProbeError.capture("microphone recovery owner is missing")
      }
      return token
    }
    let engine = try makeEngine()
    self.engine = engine
    try engine.selectDevice(token)
    let format = engine.inputFormat
    guard format.sampleRate > 0, format.channelCount > 0 else {
      throw ProbeError.capture("microphone has no usable audio format")
    }
    try lock.withLock {
      if let file {
        guard MicrophonePCMFormat.matches(file.processingFormat, format),
          stats.sampleRate == format.sampleRate,
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
      guard let file, MicrophonePCMFormat.matches(file.processingFormat, format) else {
        throw RebindError.formatChanged
      }
      selectedDeviceID = selected.objectID
      configurationChanged = false
      configurationRevision = 0
    }
    engine.observeConfiguration { [weak self] in
      // Apple's callback runs on an internal queue. Never stop, rebuild or
      // deallocate an engine here; the serialized control loop does that.
      self?.lock.withLock {
        guard self?.recovery?.owns(token) == true else { return }
        if self?.configurationChanged == false,
          self?.recovery?.accepts(token) == true,
          self?.clock == nil || self?.testWindow?.preparing == false
        {
          self?.stats.appendFailures += 1
          self?.stats.retainCaptureFailure(
            ProbeError.capture("マイクの初期化完了後に音声エンジンの構成が変更されました（microphone_configuration_changed）"))
        }
        self?.configurationChanged = true
        if let revision = self?.configurationRevision, revision < UInt64.max {
          self?.configurationRevision = revision + 1
        }
        self?.stats.inputLevel.invalidate()
      }
    }
    engine.installTap(format: format) { [weak self, weak engine] buffer, when in
      guard let self, let engine else { return }
      let receivedAt = self.levelsEnabled ? PCMInputLevel.nowNanoseconds() : nil
      let revision: UInt64? = self.lock.withLock {
        guard self.mayContinue(), self.admitsPCM(token),
          !self.configurationChanged, self.selectedDeviceID == token.objectID
        else { return nil }
        return self.configurationRevision
      }
      guard let revision else { return }
      // Core Audio property reads may notify or wait for another queue. Keep
      // them outside the callback/state lock, then fence the proof against
      // notification changes and binding retirement before writing any PCM.
      let identityError: Error?
      do {
        try engine.verifyDevice(token)
        identityError = nil
      } catch { identityError = error }
      self.lock.lock()
      defer { self.lock.unlock() }
      guard self.mayContinue(), self.admitsPCM(token), !self.configurationChanged,
        self.configurationRevision == revision, self.selectedDeviceID == token.objectID
      else {
        return
      }
      guard (try? self.stats.requireWritable(raw: self.rawSegments?.snapshot())) != nil else {
        self.stats.inputLevel.invalidate()
        return
      }
      if let identityError {
        if self.clock == nil || self.testWindow != nil {
          self.stats.appendFailures += 1
          self.stats.retainCaptureFailure(identityError)
        }
        self.recovery?.interrupt("identity_changed")
        self.configurationChanged = true
        self.stats.inputLevel.invalidate()
        return
      }
      do {
        if let testWindow = self.testWindow, !testWindow.accepting { return }
        var observation = self.clock?.observe(hostTicks: when.isHostTimeValid ? when.hostTime : nil)
        if case .suspended = observation {
          self.recovery?.interrupt("clock_changed")
          self.configurationChanged = true
          self.stats.inputLevel.invalidate()
          return
        }
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
        guard
          self.stats.acceptsMicrophonePCM(
            buffer, at: when, processingFormat: file.processingFormat)
        else {
          self.recovery?.interrupt("format_changed", unavailable: true)
          self.configurationChanged = true
          self.stats.inputLevel.invalidate()
          if self.clock == nil || self.testWindow != nil {
            self.stats.appendFailures += 1
            self.stats.retainCaptureFailure(
              ProbeError.capture(
                "マイクの入力PCMと選択済みの保存形式が一致しません（microphone_pcm_format_changed）; inputRate=\(buffer.format.sampleRate), channels=\(buffer.format.channelCount), inputFormat=\(buffer.format.commonFormat.rawValue), interleaved=\(buffer.format.isInterleaved), savedRate=\(file.processingFormat.sampleRate), savedChannels=\(file.processingFormat.channelCount), timestampRate=\(when.sampleRate)"
              ))
          }
          return
        }
        var savedSourceFrame: Int64?
        if when.isSampleTimeValid {
          let (frame, overflow) = when.sampleTime.addingReportingOverflow(sourceFrameOffset)
          guard !overflow else { throw ProbeError.capture("microphone frame offset overflows") }
          savedSourceFrame = frame
        }
        try self.stats.prepareSavedClock(
          observation, frames: Int64(buffer.frameLength), sourceFrame: savedSourceFrame,
          sourceEpoch: token.epoch)
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
        try self.rawSegments?.enqueue(
          buffer, observation: observation, sourceFrame: savedSourceFrame, sourceEpoch: token.epoch)
        if self.normalAdmission == token {
          try self.recovery?.activate(token)
          self.normalAdmission = nil
        }
      } catch {
        self.stats.appendFailures += 1
        self.stats.retainCaptureFailure(error)
        self.stats.inputLevel.invalidate()
      }
    }
    tapInstalled = true
    engine.prepare()
    guard mayContinue() else { throw ProbeError.capture("microphone stop was requested") }
    try engine.start()
    // Preparing/starting can issue a configuration notification before any PCM
    // is admitted. Do not treat that alone as an input failure, or ignore it:
    // prove the same binding, complete PCM format and a running engine against
    // one unchanged notification revision before enabling callbacks. Never
    // call AVAudioEngine control methods while holding its callback lock.
    for attempt in 0..<3 {
      let revision = try lock.withLock {
        guard mayContinue(), recovery?.owns(token) == true,
          configurationRevision < UInt64.max
        else { throw ProbeError.capture("microphone activation is obsolete or stopped") }
        return configurationRevision
      }
      try engine.verifyDevice(token)
      guard MicrophonePCMFormat.matches(format, engine.inputFormat) else {
        throw RebindError.formatChanged
      }
      if !engine.isRunning {
        guard attempt < 2, lock.withLock({ configurationChanged }), mayContinue() else {
          throw ProbeError.capture("microphone engine is not running after initialization")
        }
        // Only the already-selected engine and unchanged format may retry.
        // A changed UID/object/format never falls back to the default device.
        engine.prepare()
        guard mayContinue() else { throw ProbeError.capture("microphone stop was requested") }
        try engine.start()
        continue
      }
      let activated = try lock.withLock {
        guard mayContinue(), recovery?.owns(token) == true else {
          throw ProbeError.capture("microphone activation is obsolete or stopped")
        }
        guard configurationRevision == revision else { return false }
        configurationChanged = false
        if clock != nil, testWindow == nil {
          let now = controlNow()
          normalPreparation = NormalPreparation(
            token: token, format: format,
            nextProof: now.advanced(by: .milliseconds(100)))
        } else {
          try recovery?.activate(token)
        }
        return true
      }
      if activated { return }
    }
    throw ProbeError.capture("microphone configuration did not stabilize during initialization")
  }

  private func releaseEngine() {
    engine?.observeConfiguration(nil)
    if tapInstalled, let engine {
      engine.removeTap()
      tapInstalled = false
    }
    engine?.stop()
    engine = nil
  }

  // Called only with the callback/state lock held.
  private func admitsPCM(_ token: MicrophoneRecovery.Binding) -> Bool {
    recovery?.accepts(token) == true
      || (normalAdmission == token && recovery?.owns(token) == true)
  }

  /// Serialized control-loop proofs, not sleeps or engine control on Apple's
  /// notification/audio queue. Delayed startup notifications keep this same
  /// engine preparing instead of repeatedly reporting a false recovery.
  private func maintainNormalPreparation() throws {
    guard let engine, var pending = lock.withLock({ normalPreparation }) else { return }
    let now = controlNow()
    // PC-audio initialization may be waiting for the user's OS permission.
    // Start the bounded proof budget on the first control-loop check, not
    // before that unrelated prompt, and still re-prove the complete binding.
    let deadline = pending.deadline ?? now.advanced(by: .seconds(2))
    pending.deadline = deadline
    guard now < deadline else {
      throw ProbeError.capture("microphone configuration did not stabilize during recovery")
    }
    guard now >= pending.nextProof else { return }
    let revision = try lock.withLock {
      guard mayContinue(), recovery?.owns(pending.token) == true,
        configurationRevision < UInt64.max
      else { throw ProbeError.capture("microphone preparation is obsolete or stopped") }
      try stats.requireWritable(raw: rawSegments?.snapshot())
      return configurationRevision
    }
    try engine.verifyDevice(pending.token)
    guard MicrophonePCMFormat.matches(pending.format, engine.inputFormat) else {
      throw RebindError.formatChanged
    }
    if !engine.isRunning {
      guard pending.restarts < 2,
        lock.withLock({
          mayContinue() && recovery?.owns(pending.token) == true && configurationChanged
            && configurationRevision == revision
        })
      else { throw ProbeError.capture("microphone engine stopped during recovery") }
      engine.prepare()
      guard mayContinue() else { throw ProbeError.capture("microphone stop was requested") }
      try engine.start()
      pending.restarts += 1
      pending.revision = nil
      pending.stableProofs = 0
    } else {
      pending.stableProofs = pending.revision == revision ? pending.stableProofs + 1 : 1
      pending.revision = revision
    }
    pending.nextProof = now.advanced(by: .milliseconds(100))
    try lock.withLock {
      guard mayContinue(), recovery?.owns(pending.token) == true else {
        throw ProbeError.capture("microphone preparation is obsolete or stopped")
      }
      guard configurationRevision == revision else {
        pending.revision = nil
        pending.stableProofs = 0
        normalPreparation = pending
        return
      }
      configurationChanged = false
      if pending.stableProofs >= 3 {
        normalPreparation = nil
        normalAdmission = pending.token
      } else {
        normalPreparation = pending
      }
    }
  }

  /// A synchronous start proof cannot fence delayed HAL notifications. Keep
  /// both tracks' shared window closed until three spaced proofs agree. Only
  /// this pre-window phase may restart the same engine; never repair a measured
  /// five-second window by discarding an interruption or inventing PCM.
  func openTestWindow(
    now: @Sendable () -> Int64? = { PCMInputLevel.nowNanoseconds() },
    pause: @Sendable () async throws -> Void = {
      try await Task.sleep(for: .milliseconds(100))
    }
  ) async throws {
    guard let testWindow, let engine else {
      throw ProbeError.capture("five-second test microphone is not initialized")
    }
    let (token, format) = try lock.withLock {
      guard testWindow.preparing, stats.frames == 0, let recovery, let file else {
        throw ProbeError.capture("five-second test microphone preparation is obsolete")
      }
      return (try recovery.currentBinding(), file.processingFormat)
    }
    var previousRevision: UInt64?
    var stableProofs = 0
    var restarts = 0
    for _ in 0..<20 {
      try Task.checkCancellation()
      guard mayContinue() else { throw ProbeError.capture("microphone stop was requested") }
      try await pause()
      try Task.checkCancellation()
      let revision = try lock.withLock {
        guard mayContinue(), recovery?.accepts(token) == true, testWindow.preparing,
          stats.frames == 0, configurationRevision < UInt64.max
        else { throw ProbeError.capture("five-second test microphone preparation is stopped") }
        try stats.requireWritable(raw: rawSegments?.snapshot())
        return configurationRevision
      }
      let authorization = permission()
      guard authorization == .authorized else {
        throw ProbeError.permission(CaptureFailureGuide.microphonePermission(authorization))
      }
      let selected = try InputDevices.choose(token.uid, from: devices().devices)
      guard selected.objectID == token.objectID else {
        throw ProbeError.capture("microphone identity changed during test preparation")
      }
      try engine.verifyDevice(token)
      guard MicrophonePCMFormat.matches(format, engine.inputFormat) else {
        throw RebindError.formatChanged
      }
      if !engine.isRunning {
        guard restarts < 2,
          lock.withLock({
            mayContinue() && recovery?.accepts(token) == true && configurationChanged
              && testWindow.preparing && configurationRevision == revision
          })
        else {
          throw ProbeError.capture("microphone engine is not running during test preparation")
        }
        // Controls run on this serialized owner, never Apple's notify queue.
        engine.prepare()
        guard mayContinue() else { throw ProbeError.capture("microphone stop was requested") }
        try engine.start()
        restarts += 1
        previousRevision = nil
        stableProofs = 0
        continue
      }
      let opened = try lock.withLock {
        guard mayContinue(), recovery?.accepts(token) == true, testWindow.preparing else {
          throw ProbeError.capture("five-second test microphone preparation is stopped")
        }
        try stats.requireWritable(raw: rawSegments?.snapshot())
        guard configurationRevision == revision else {
          previousRevision = nil
          stableProofs = 0
          return false
        }
        stableProofs = previousRevision == revision ? stableProofs + 1 : 1
        previousRevision = revision
        configurationChanged = false
        guard stableProofs >= 3 else { return false }
        // The same callback lock fences the final revision and window opening.
        // A later notification is an in-window failure, not another retry.
        try testWindow.open(at: now())
        return true
      }
      if opened { return }
    }
    throw ProbeError.capture("microphone configuration did not stabilize before five-second test")
  }

  private func suspend(_ reason: String? = nil, unavailable: Bool = false) {
    lock.withLock {
      if let reason {
        recovery?.interrupt(reason, unavailable: unavailable)
      } else {
        recovery?.interruptForRetry()
      }
      selectedDeviceID = nil
      normalPreparation = nil
      normalAdmission = nil
      stats.inputLevel.invalidate()
    }
    releaseEngine()
    // The current CAF remains open. Recreating it would overwrite prior PCM.
  }

  func suspendForClockRecovery(_ request: CaptureClockRecoveryRequest) throws
    -> CaptureClockQuiescence
  {
    guard let clock, testWindow == nil, try clock.recoveryRequest() == request else {
      throw ProbeError.capture("microphone clock recovery is not available")
    }
    try lock.withLock {
      guard let recovery, recovery.status.state != "stopped" else {
        throw ProbeError.capture("microphone acquisition has not started or is stopped")
      }
      try stats.requireWritable(raw: rawSegments?.snapshot())
    }
    suspend("clock_changed")
    // Invalidation takes the callback lock, draining already-admitted writes.
    // After removeTap/stop, late callbacks still carry the retired binding and
    // are rejected before timestamps, CAF or raw writes are considered.
    return try clock.quiesced(.microphone, for: request)
  }

  func maintain() {
    // Formal bounded probes and a private 5-second test retain their existing
    // fail/verify contract; only an active production recording retries inputs.
    guard mayContinue(), clock != nil, testWindow == nil,
      lock.withLock({
        recovery?.status.state != "stopped"
          && (try? stats.requireWritable(raw: rawSegments?.snapshot())) != nil
      }), let uid = deviceUID
    else { return }
    let report: CaptureInputDevices
    do { report = try devices() } catch {
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
    if lock.withLock({ normalPreparation != nil && selectedDeviceID == selected.objectID }) {
      do { try maintainNormalPreparation() } catch RebindError.formatChanged {
        suspend("format_changed", unavailable: true)
      } catch { suspend("restart_failed", unavailable: true) }
      return
    }
    let healthy = lock.withLock {
      (recovery?.status.state == "connected" || normalAdmission != nil) && !configurationChanged
        && selectedDeviceID == selected.objectID
    }
    if healthy, let engine, engine.isRunning {
      do {
        let binding = try lock.withLock {
          if let token = normalAdmission, recovery?.owns(token) == true { return token }
          guard let binding = try recovery?.currentBinding() else {
            throw ProbeError.capture("microphone binding is missing")
          }
          return binding
        }
        try engine.verifyDevice(binding)
        return
      } catch {
        // Continue only after invalidating the old engine binding.
      }
    }
    // A retired engine is already interrupted. Preserve its diagnosis during
    // backoff instead of changing unavailable/restart_failed to engine_changed
    // on every heartbeat without any new acquisition evidence.
    if engine != nil { suspend() }
    let now = controlNow()
    if let lastRecoveryAttempt, now < lastRecoveryAttempt.advanced(by: .seconds(1)) { return }
    lastRecoveryAttempt = now
    do { try configure(selected) } catch RebindError.formatChanged {
      suspend("format_changed", unavailable: true)
    } catch { suspend("restart_failed", unavailable: true) }
  }

  func stop() {
    lock.withLock {
      recovery?.stop()
      selectedDeviceID = nil
      normalPreparation = nil
      normalAdmission = nil
      stats.inputLevel.invalidate()
    }
    releaseEngine()
    lock.withLock {
      if captureClosure == nil {
        do { captureClosure = try clock?.close(.microphone) } catch {
          stats.clockFailure = stats.clockFailure ?? String(describing: error)
        }
      }
      file = nil
    }
    do { try rawSegments?.finish() } catch {
      lock.withLock {
        stats.appendFailures += 1
        stats.retainCaptureFailure(error)
      }
    }
  }

  func snapshot() -> TrackStats {
    lock.lock()
    defer { lock.unlock() }
    var result = stats
    result.microphoneRecovery = recovery?.status
    result.inputEventHistory = recovery?.history
    result.rawSegments = rawSegments?.snapshot()
    if result.captureFailure != nil { result.inputLevel.invalidate() }
    return result
  }
}

/// The executable owns @main; importing the core must not import a C entry
/// point into an optimized test runner. Preserve async-main actor isolation.
public enum CaptureCommand {
  @MainActor
  public static func run() async {
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
        ? try CaptureClockDomain.live(
          allowsRecovery: options.recoversCaptureClock)
        : nil
      let testWindow = try options.captureTestOwner.map { try CaptureTestWindow(ownerId: $0) }
      let inputEvents =
        options.publishesInputStatus
        ? try InputEventJournal(
          output: options.output, owner: options.inputLevelOwner!,
          supportsSystem: !options.usesStandaloneTapProbe) : nil
      let journalStatus = try inputEvents.map {
        try InputJournalStatusPublisher(
          output: options.output, owner: options.inputLevelOwner!, initial: $0.persistence())
      }
      defer { journalStatus?.removeIfOwned() }
      let rawMic =
        options.publishesInputStatus
        ? try RawRecordingSegments(
          output: options.output, owner: options.inputLevelOwner!, lane: "microphone") : nil
      let rawSystem =
        options.publishesInputStatus
        ? try RawRecordingSegments(
          output: options.output, owner: options.inputLevelOwner!, lane: "system") : nil
      let systemStopRequest = options.output.appendingPathComponent("stop.request")
      let systemParentPID = options.parentPID
      let systemControlled = options.stopOnFile
      let mic = MicrophoneRecorder(
        outputURL: options.output.appendingPathComponent("microphone.caf"), clock: clock,
        microphoneUID: options.microphoneUID, levelsEnabled: options.inputLevelOwner != nil,
        testWindow: testWindow, inputEventObserver: inputEvents?.observer(.microphone),
        rawSegments: rawMic,
        mayContinue: {
          !systemControlled
            || (!FileManager.default.fileExists(atPath: systemStopRequest.path)
              && !parentHasExited(systemParentPID))
        })
      let system = SystemAudioRecorder(
        outputURL: options.output.appendingPathComponent("system.caf"), clock: clock,
        levelsEnabled: options.inputLevelOwner != nil, testWindow: testWindow,
        retries: options.recoversSystemAudio, backend: options.backend,
        mayContinue: {
          !systemControlled
            || (!FileManager.default.fileExists(atPath: systemStopRequest.path)
              && !parentHasExited(systemParentPID))
        }, inputEventObserver: inputEvents?.observer(.system), rawSegments: rawSystem)
      let tap = CoreAudioTapRecorder(
        outputURL: options.output.appendingPathComponent("system.caf"), clock: clock,
        levelsEnabled: options.inputLevelOwner != nil, rawSegments: rawSystem)
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
        if options.startsSystemBeforeMicrophone {
          try await system.start(
            targetPID: options.targetPIDs.first, applicationIdentity: options.applicationIdentity)
        }
        try await mic.start()
        manifest.microphoneDeviceUID = mic.deviceUID
        manifest.microphone = mic.snapshot()
        if options.usesStandaloneTapProbe {
          try tap.start(targetPIDs: options.targetPIDs)
        } else {
          if !options.startsSystemBeforeMicrophone {
            try await system.start(
              targetPID: options.targetPIDs.first, applicationIdentity: options.applicationIdentity)
          }
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
        if testWindow != nil { try await mic.openTestWindow() }
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
            if let clock, let request = try clock.recoveryRequest() {
              let micQuiescence = try mic.suspendForClockRecovery(request)
              let systemQuiescence = try await system.suspendForClockRecovery(request)
              try clock.resume(
                request, microphone: micQuiescence, system: systemQuiescence)
              // A stop/parent exit during quiescence may close the proven clock,
              // but must never launch another microphone/SCStream acquisition.
              guard
                options.shouldContinueRecording(
                  startedAt: startedAt, now: .now,
                  stopRequested: FileManager.default.fileExists(atPath: stopRequest.path),
                  parentExited: parentHasExited(options.parentPID))
              else { break }
              try system.resumeAfterClockRecovery(request)
            }
            mic.maintain()
            if !options.usesStandaloneTapProbe { system.maintain() }
            let microphone = mic.snapshot()
            let systemStats = options.usesStandaloneTapProbe ? tap.snapshot() : system.snapshot()
            try microphone.requireWritable()
            try systemStats.requireWritable()
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
                  system: systemStats.systemRecovery,
                  microphoneHistory: microphone.inputEventHistory,
                  systemHistory: systemStats.inputEventHistory
                )
                .publish(to: options.output)
              }
              if let inputEvents {
                try? journalStatus?.publish(inputEvents.persistence())
              }
              let microphone = microphone.inputLevel
              let systemLevel =
                (options.usesStandaloneTapProbe ? tap.snapshot() : system.snapshot())
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
          let deadline = ContinuousClock.now.advanced(by: .seconds(duration))
          while ContinuousClock.now < deadline {
            try mic.snapshot().requireWritable()
            try (options.usesStandaloneTapProbe ? tap.snapshot() : system.snapshot())
              .requireWritable()
            try await Task.sleep(for: .milliseconds(100))
          }
        }
        testWindow?.finish(at: PCMInputLevel.nowNanoseconds())
        mic.stop()
        var systemClosure: CaptureClosure?
        if options.publishesInputStatus {
          systemClosure =
            options.usesStandaloneTapProbe
            ? try tap.stopRecording() : try await system.stopRecording()
        } else {
          if options.usesStandaloneTapProbe { try tap.stop() } else { try await system.stop() }
        }
        let micStats = mic.snapshot()
        let systemStats = options.usesStandaloneTapProbe ? tap.snapshot() : system.snapshot()
        try micStats.requireWritable()
        try systemStats.requireWritable()
        if let testWindow, !testWindow.metadata().completed {
          throw ProbeError.capture("five-second test stopped before its window completed")
        }
        guard micStats.frames > 0,
          systemStats.frames > 0 || testWindow != nil || systemClosure != nil,
          micStats.appendFailures == 0, systemStats.appendFailures == 0
        else {
          throw ProbeError.capture("audio track is empty or contains failed writes")
        }
        try verifyRecordedCAF(
          options.output.appendingPathComponent("microphone.caf"), expected: micStats)
        if systemStats.frames > 0 {
          try verifyRecordedCAF(
            options.output.appendingPathComponent("system.caf"), expected: systemStats)
        }
        manifest.microphone = micStats
        manifest.system = systemStats
        if let clock {
          if options.publishesInputStatus {
            try manifest.finalizeStoppedClock(
              clock, microphoneClosure: mic.captureClosure, systemClosure: systemClosure)
          } else if let testWindow {
            try manifest.finalizeTestClock(clock, window: testWindow)
          } else {
            try manifest.finalizeClock(clock)
          }
        }
        manifest.state = "saved"
      } catch {
        // Keep the initiating failure first. Cleanup can also fail when an
        // input never started, but must not replace the permission/device error.
        manifest.errors.append(String(describing: error))
        testWindow?.finish(at: PCMInputLevel.nowNanoseconds())
        mic.stop()
        do {
          if options.usesStandaloneTapProbe { try tap.stop() } else { try await system.stop() }
        } catch { manifest.errors.append(String(describing: error)) }
        manifest.state = "failed"
      }
      do {
        try power.release()
        manifest.power.released = manifest.power.acquired
      } catch {
        manifest.state = "failed"
        manifest.errors.append(String(describing: error))
      }
      manifest.endedAtUTC = ISO8601DateFormatter().string(from: Date())
      do { try inputEvents?.finish() } catch {
        manifest.inputEventJournalError = String(describing: error)
      }
      manifest.inputEventJournalPersistence = inputEvents?.persistence()
      manifest.captureTest = testWindow?.metadata()
      if manifest.state != "saved" {
        manifest.microphone = mic.snapshot()
        manifest.system = options.usesStandaloneTapProbe ? tap.snapshot() : system.snapshot()
      } else if let testWindow, let micStart = manifest.microphone.firstCaptureNanoseconds {
        let systemStart = manifest.system.firstCaptureNanoseconds ?? micStart
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
