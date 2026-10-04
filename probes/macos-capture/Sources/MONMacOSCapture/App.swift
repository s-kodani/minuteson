import AVFoundation
import CoreMedia
import Darwin
import Foundation
import ScreenCaptureKit

private struct Options {
  let output: URL
  let duration: TimeInterval
  let scenario: String
  let mode: String
  let backend: String
  let targetPIDs: [pid_t]
  let stopOnFile: Bool
  let parentPID: pid_t?

  static func parse(_ arguments: [String]) throws -> Self {
    var output: URL?
    var duration: TimeInterval = 30
    var scenario = "manual"
    var mode = "system"
    var backend = "screencapturekit"
    var targetPIDs: [pid_t] = []
    var stopOnFile = false
    var parentPID: pid_t?
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
      case "--stop-on-file":
        stopOnFile = true
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
    if parentPID != nil && !stopOnFile {
      throw ProbeError.usage("--parent-pid requires --stop-on-file")
    }
    return Self(
      output: output.standardizedFileURL, duration: duration, scenario: scenario,
      mode: mode, backend: backend, targetPIDs: targetPIDs, stopOnFile: stopOnFile,
      parentPID: parentPID)
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

struct TrackStats: Codable {
  var callbackCount = 0
  var frames: Int64 = 0
  var firstPresentationTimeSeconds: Double?
  var lastPresentationTimeSeconds: Double?
  var appendFailures = 0
  var sampleRate: Double?
  var channels: UInt32?
}

private struct Manifest: Encodable {
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
  let requestedDurationSeconds: Double
  let osVersion: String
  let architecture: String
  var state: String
  var microphone: TrackStats
  var system: TrackStats
  var power = RecordingPowerStatus()
  var errors: [String]
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
  private(set) var targetApplicationName: String?
  private(set) var targetBundleIdentifier: String?

  init(outputURL: URL) { self.outputURL = outputURL }

  func start(targetPID: pid_t?) async throws {
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
    do {
      try await stream.startCapture()
      captureStarted = true
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
    lock.lock()
    defer { lock.unlock() }
    if stopping { return }
    do {
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
        writer.startSession(atSourceTime: sampleBuffer.presentationTimeStamp)
        self.writer = writer
        self.input = input
        stats.sampleRate = format.mSampleRate
        stats.channels = format.mChannelsPerFrame
        stats.firstPresentationTimeSeconds = sampleBuffer.presentationTimeStamp.seconds
      }
      stats.callbackCount += 1
      stats.frames += Int64(sampleBuffer.numSamples)
      stats.lastPresentationTimeSeconds = sampleBuffer.presentationTimeStamp.seconds
      if input?.isReadyForMoreMediaData == true {
        if input?.append(sampleBuffer) == true {
          guard let sampleRate = stats.sampleRate else {
            throw ProbeError.capture("system audio sample rate is missing")
          }
          let frameDuration = CMTime(
            seconds: Double(sampleBuffer.numSamples) / sampleRate,
            preferredTimescale: 1_000_000_000)
          lastSampleEnd = CMTimeAdd(sampleBuffer.presentationTimeStamp, frameDuration)
        } else {
          stats.appendFailures += 1
        }
      } else {
        stats.appendFailures += 1
      }
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

  init(outputURL: URL) { self.outputURL = outputURL }

  func start() async throws {
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
      self.lock.lock()
      defer { self.lock.unlock() }
      do {
        try self.file?.write(from: buffer)
        self.stats.callbackCount += 1
        self.stats.frames += Int64(buffer.frameLength)
        let seconds = Double(when.sampleTime) / when.sampleRate
        if self.stats.firstPresentationTimeSeconds == nil {
          self.stats.firstPresentationTimeSeconds = seconds
        }
        self.stats.lastPresentationTimeSeconds = seconds
      } catch {
        self.stats.appendFailures += 1
      }
    }
    tapInstalled = true
    engine.prepare()
    try engine.start()
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
      let options = try Options.parse(arguments)
      if FileManager.default.fileExists(atPath: options.output.path) {
        let contents = try FileManager.default.contentsOfDirectory(atPath: options.output.path)
        guard contents.isEmpty else {
          throw ProbeError.usage("output directory already contains files")
        }
      }
      try FileManager.default.createDirectory(at: options.output, withIntermediateDirectories: true)
      let manifestURL = options.output.appendingPathComponent("manifest.json")
      let mic = MicrophoneRecorder(
        outputURL: options.output.appendingPathComponent("microphone.caf"))
      let system = SystemAudioRecorder(
        outputURL: options.output.appendingPathComponent("system.caf"))
      let tap = CoreAudioTapRecorder(
        outputURL: options.output.appendingPathComponent("system.caf"))
      var manifest = Manifest(
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
        if options.backend == "coreaudio" {
          try tap.start(targetPIDs: options.targetPIDs)
        } else {
          try await system.start(targetPID: options.targetPIDs.first)
          manifest.targetApplicationName = system.targetApplicationName
          manifest.targetBundleIdentifier = system.targetBundleIdentifier
        }
        manifest.state = "recording"
        try write(manifest, to: manifestURL)
        if options.stopOnFile {
          let stopRequest = options.output.appendingPathComponent("stop.request")
          let deadline = ContinuousClock.now.advanced(by: .seconds(options.duration))
          while ContinuousClock.now < deadline
            && !FileManager.default.fileExists(atPath: stopRequest.path)
            && !parentHasExited(options.parentPID)
          {
            try await Task.sleep(for: .milliseconds(200))
          }
        } else {
          try await Task.sleep(for: .seconds(options.duration))
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
        try verifyCAF(options.output.appendingPathComponent("microphone.caf"))
        try verifyCAF(options.output.appendingPathComponent("system.caf"))
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
      manifest.microphone = mic.snapshot()
      manifest.system = options.backend == "coreaudio" ? tap.snapshot() : system.snapshot()
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
           mon-macos-capture --list-audio-processes
           mon-macos-capture --output <directory> [--duration <seconds>]
             [--scenario <id>] [--mode system|application] [--pid <process-id> ...]
             [--backend screencapturekit|coreaudio] [--stop-on-file] [--parent-pid <pid>]
    Records microphone.caf and system.caf after the required macOS permissions are granted.
    """

  private static func verifyCAF(_ url: URL) throws {
    let file = try AVAudioFile(forReading: url)
    guard file.length > 0, file.processingFormat.sampleRate > 0,
      file.processingFormat.channelCount > 0
    else {
      throw ProbeError.capture(
        "recorded CAF cannot be read or contains no frames: \(url.lastPathComponent)")
    }
  }

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
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
    try encoder.encode(manifest).write(to: url, options: .atomic)
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
