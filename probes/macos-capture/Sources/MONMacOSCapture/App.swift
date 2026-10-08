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
    return Self(
      output: output.standardizedFileURL, duration: untilStopped ? nil : duration,
      scenario: scenario,
      mode: mode, backend: backend, targetPIDs: targetPIDs, stopOnFile: stopOnFile,
      parentPID: parentPID, microphoneUID: microphoneUID, applicationIdentity: applicationIdentity,
      inputLevelOwner: inputLevelOwner)
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

  enum CodingKeys: String, CodingKey {
    case callbackCount, frames, firstPresentationTimeSeconds, lastPresentationTimeSeconds
    case appendFailures, sampleRate, channels, clockSegments, clockFailure
  }

  var firstCaptureNanoseconds: Int64? { timing?.firstCaptureNanoseconds }

  mutating func recordSavedClock(
    _ observation: CaptureClockObservation, frames: Int64, sourceFrame: Int64?
  ) {
    guard clockFailure == nil else { return }
    do {
      if timing == nil {
        guard let sampleRate else { throw ProbeError.capture("capture sample rate is missing") }
        timing = try CaptureTrackTiming(sampleRate: sampleRate)
      }
      try timing?.recordSaved(frames: frames, observation: observation, sourceFrame: sourceFrame)
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

private final class SystemAudioRecorder: NSObject, SCStreamOutput, @unchecked Sendable {
  private let outputURL: URL
  private let queue = DispatchQueue(label: "app.mon.capture.system")
  private let lock = NSLock()
  private var writer: AVAssetWriter?
  private var input: AVAssetWriterInput?
  private var stream: SCStream?
  private var captureStarted = false
  private var finished = false
  private var stopping = false
  private var lastSampleEnd: CMTime?
  private var stats = TrackStats()
  private let clock: CaptureClockDomain?
  private let levelsEnabled: Bool
  private(set) var targetApplicationName: String?
  private(set) var targetBundleIdentifier: String?

  init(outputURL: URL, clock: CaptureClockDomain? = nil, levelsEnabled: Bool = false) {
    self.outputURL = outputURL
    self.clock = clock
    self.levelsEnabled = levelsEnabled
  }

  private var applicationIdentity: CaptureApplicationIdentity?

  func start(targetPID: pid_t?, applicationIdentity: CaptureApplicationIdentity? = nil) async throws
  {
    if let applicationIdentity { try CaptureApplications.current(applicationIdentity) }
    let content: SCShareableContent
    do {
      content = try await SCShareableContent.excludingDesktopWindows(
        false, onScreenWindowsOnly: false)
    } catch {
      throw ProbeError.permission("system audio permission or shareable content failed: \(error)")
    }
    guard let display = content.displays.first else {
      throw ProbeError.capture("no display available for system audio capture")
    }
    let filter: SCContentFilter
    if let targetPID {
      guard let application = content.applications.first(where: { $0.processID == targetPID })
      else {
        throw ProbeError.capture("selected process is not available in shareable applications")
      }
      targetApplicationName = application.applicationName
      targetBundleIdentifier = application.bundleIdentifier
      if let applicationIdentity {
        guard application.processID == applicationIdentity.pid,
          application.bundleIdentifier == applicationIdentity.bundleIdentifier
        else {
          throw ProbeError.capture("shareable application does not match the confirmed identity")
        }
        try CaptureApplications.current(applicationIdentity)
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
    let stream = SCStream(filter: filter, configuration: configuration, delegate: nil)
    try stream.addStreamOutput(self, type: .audio, sampleHandlerQueue: queue)
    self.stream = stream
    self.applicationIdentity = applicationIdentity
    do {
      try await stream.startCapture()
      captureStarted = true
      if let applicationIdentity { try CaptureApplications.current(applicationIdentity) }
    } catch {
      throw ProbeError.permission("system audio capture could not start: \(error)")
    }
  }

  func stop() async throws {
    if finished { return }
    var streamStopError: String?
    if captureStarted, let stream {
      do {
        try await stream.stopCapture()
      } catch {
        streamStopError = String(describing: error)
        lock.withLock { stopping = true }
      }
      captureStarted = false
      queue.sync {}
    }
    let (input, writer, lastSampleEnd) = lock.withLock {
      () -> (AVAssetWriterInput?, AVAssetWriter?, CMTime?) in
      let result = (self.input, self.writer, self.lastSampleEnd)
      self.input = nil
      self.writer = nil
      self.lastSampleEnd = nil
      return result
    }
    // Without an explicit end time, AVAssetWriter can omit a partial final PCM packet.
    if let writer, let lastSampleEnd { writer.endSession(atSourceTime: lastSampleEnd) }
    input?.markAsFinished()
    if let writer {
      await writer.finishWriting()
      if writer.status != .completed {
        throw ProbeError.capture(
          "system CAF finalization failed: \(writer.error?.localizedDescription ?? String(describing: writer.status))"
        )
      }
      finished = true
      if let streamStopError {
        throw ProbeError.capture(
          "system audio capture stop failed after CAF finalization: \(streamStopError)")
      }
    } else {
      throw ProbeError.capture("no system audio samples were received")
    }
  }

  func snapshot() -> TrackStats {
    lock.lock()
    defer { lock.unlock() }
    return stats
  }

  func stream(
    _ stream: SCStream, didOutputSampleBuffer sampleBuffer: CMSampleBuffer,
    of outputType: SCStreamOutputType
  ) {
    guard outputType == .audio, sampleBuffer.isValid, sampleBuffer.numSamples > 0 else { return }
    let receivedAt = levelsEnabled ? PCMInputLevel.nowNanoseconds() : nil
    lock.lock()
    defer { lock.unlock() }
    if stopping { return }
    do {
      if let identity = applicationIdentity {
        guard try CaptureApplications.processStart(identity.pid) == identity.launchTimeUs else {
          throw ProbeError.capture("application changed during capture")
        }
      }
      if writer == nil {
        let format = sampleBuffer.formatDescription.flatMap {
          CMAudioFormatDescriptionGetStreamBasicDescription($0)?.pointee
        }
        guard let format else { throw ProbeError.capture("system audio format is missing") }
        guard format.mSampleRate > 0, format.mChannelsPerFrame > 0 else {
          throw ProbeError.capture("system audio format is invalid")
        }
        let settings: [String: Any] = [
          AVFormatIDKey: kAudioFormatLinearPCM,
          AVSampleRateKey: format.mSampleRate,
          AVNumberOfChannelsKey: format.mChannelsPerFrame,
          AVLinearPCMBitDepthKey: 32,
          AVLinearPCMIsFloatKey: true,
          AVLinearPCMIsNonInterleaved: false,
          AVLinearPCMIsBigEndianKey: false,
        ]
        let writer = try AVAssetWriter(outputURL: outputURL, fileType: .caf)
        guard writer.canApply(outputSettings: settings, forMediaType: .audio) else {
          throw ProbeError.capture("cannot write system audio format to CAF")
        }
        let input = AVAssetWriterInput(mediaType: .audio, outputSettings: settings)
        input.expectsMediaDataInRealTime = true
        guard writer.canAdd(input) else {
          throw ProbeError.capture("cannot add system audio writer input")
        }
        writer.add(input)
        guard writer.startWriting() else {
          throw ProbeError.capture("cannot start system CAF writer")
        }
        writer.startSession(atSourceTime: .zero)
        self.writer = writer
        self.input = input
        stats.sampleRate = format.mSampleRate
        stats.channels = format.mChannelsPerFrame
        stats.firstPresentationTimeSeconds = sampleBuffer.presentationTimeStamp.seconds
      }
      stats.callbackCount += 1
      guard let sampleRate = stats.sampleRate, let input, input.isReadyForMoreMediaData else {
        throw ProbeError.capture("system audio writer is not ready")
      }
      guard let description = sampleBuffer.formatDescription,
        let format = CMAudioFormatDescriptionGetStreamBasicDescription(description)?.pointee,
        format.mSampleRate == sampleRate, format.mChannelsPerFrame == stats.channels
      else { throw ProbeError.capture("system audio format changed during capture") }
      if levelsEnabled { stats.inputLevel.receive(sampleBuffer, at: receivedAt) }
      let observation = clock?.observe(
        presentation: sampleBuffer.presentationTimeStamp, clock: stream.synchronizationClock)
      let contiguous = try continuousPCMSample(
        sampleBuffer, frameOffset: stats.frames, sampleRate: sampleRate)
      guard input.append(contiguous) else {
        throw ProbeError.capture("system audio append failed")
      }
      let frames = Int64(sampleBuffer.numSamples)
      stats.frames += frames
      stats.lastPresentationTimeSeconds = sampleBuffer.presentationTimeStamp.seconds
      if let observation {
        stats.recordSavedClock(observation, frames: frames, sourceFrame: nil)
      }
      lastSampleEnd = CMTime(value: stats.frames, timescale: Int32(sampleRate))
    } catch {
      stats.appendFailures += 1
    }
  }

}

private final class MicrophoneRecorder: @unchecked Sendable {
  private let engine = AVAudioEngine()
  private let outputURL: URL
  private let lock = NSLock()
  private var file: AVAudioFile?
  private var tapInstalled = false
  private var stats = TrackStats()
  private let clock: CaptureClockDomain?
  private let levelsEnabled: Bool

  private let requestedUID: String?
  private var inputUnit: AudioUnit?
  private var selectedDeviceID: AudioObjectID?
  private(set) var deviceUID: String?

  init(
    outputURL: URL, clock: CaptureClockDomain? = nil, microphoneUID: String? = nil,
    levelsEnabled: Bool = false
  ) {
    self.outputURL = outputURL
    self.clock = clock
    self.requestedUID = microphoneUID
    self.levelsEnabled = levelsEnabled
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
    try InputDevices.verifyDevice(unit, expected: selected.objectID)
    inputUnit = unit
    selectedDeviceID = selected.objectID
    deviceUID = selected.uid
    let format = node.outputFormat(forBus: 0)
    guard format.sampleRate > 0, format.channelCount > 0 else {
      throw ProbeError.capture("microphone has no usable audio format")
    }
    let file = try AVAudioFile(
      forWriting: outputURL, settings: format.settings, commonFormat: .pcmFormatFloat32,
      interleaved: false)
    self.file = file
    lock.withLock {
      stats.sampleRate = format.sampleRate
      stats.channels = format.channelCount
    }
    node.installTap(onBus: 0, bufferSize: 1024, format: format) { [weak self] buffer, when in
      guard let self else { return }
      let receivedAt = self.levelsEnabled ? PCMInputLevel.nowNanoseconds() : nil
      self.lock.lock()
      defer { self.lock.unlock() }
      do {
        guard let unit = self.inputUnit, let device = self.selectedDeviceID else {
          throw ProbeError.capture("microphone selection is missing")
        }
        try InputDevices.verifyDevice(unit, expected: device)
        let observation = self.clock?.observe(hostTicks: when.isHostTimeValid ? when.hostTime : nil)
        guard let file = self.file else {
          throw ProbeError.capture("microphone CAF writer is closed")
        }
        guard buffer.format.sampleRate == self.stats.sampleRate,
          buffer.format.channelCount == self.stats.channels,
          !when.isSampleTimeValid || when.sampleRate == self.stats.sampleRate
        else { throw ProbeError.capture("microphone format changed during capture") }
        if self.levelsEnabled { self.stats.inputLevel.receive(buffer, at: receivedAt) }
        try file.write(from: buffer)
        self.stats.callbackCount += 1
        self.stats.frames += Int64(buffer.frameLength)
        if when.isSampleTimeValid, when.sampleRate > 0 {
          let seconds = Double(when.sampleTime) / when.sampleRate
          if self.stats.firstPresentationTimeSeconds == nil {
            self.stats.firstPresentationTimeSeconds = seconds
          }
          self.stats.lastPresentationTimeSeconds = seconds
        }
        if let observation {
          self.stats.recordSavedClock(
            observation, frames: Int64(buffer.frameLength),
            sourceFrame: when.isSampleTimeValid ? when.sampleTime : nil)
        }
      } catch {
        self.stats.appendFailures += 1
      }
    }
    tapInstalled = true
    engine.prepare()
    try engine.start()
    try InputDevices.verifyDevice(unit, expected: selected.objectID)
  }

  func stop() {
    if tapInstalled {
      engine.inputNode.removeTap(onBus: 0)
      tapInstalled = false
    }
    engine.stop()
    lock.withLock { file = nil }
  }

  func snapshot() -> TrackStats {
    lock.lock()
    defer { lock.unlock() }
    return stats
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
      }
      let manifestURL = options.output.appendingPathComponent("manifest.json")
      // Production recordings require capture-time proof. Bounded measurement
      // probes retain schema 1 until their formal evaluation harness is updated.
      let clock = options.duration == nil ? try CaptureClockDomain.live() : nil
      let mic = MicrophoneRecorder(
        outputURL: options.output.appendingPathComponent("microphone.caf"), clock: clock,
        microphoneUID: options.microphoneUID, levelsEnabled: options.inputLevelOwner != nil)
      let system = SystemAudioRecorder(
        outputURL: options.output.appendingPathComponent("system.caf"), clock: clock,
        levelsEnabled: options.inputLevelOwner != nil)
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
            if let owner = options.inputLevelOwner, levelRevision < 9_007_199_254_740_991 {
              levelRevision += 1
              let microphone = mic.snapshot().inputLevel
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
        mic.stop()
        if options.backend == "coreaudio" { try tap.stop() } else { try await system.stop() }
        let micStats = mic.snapshot()
        let systemStats = options.backend == "coreaudio" ? tap.snapshot() : system.snapshot()
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
      if manifest.state != "saved" {
        manifest.microphone = mic.snapshot()
        manifest.system = options.backend == "coreaudio" ? tap.snapshot() : system.snapshot()
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
    --until-stopped requires --stop-on-file and --parent-pid, without --duration.
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
