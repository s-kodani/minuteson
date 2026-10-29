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
    var seen = Set<String>()
    var index = 0
    while index < arguments.count {
      let argument = arguments[index]
      if argument != "--pid", !seen.insert(argument).inserted {
        throw ProbeError.usage("duplicate argument: \(argument)")
      }
      switch argument {
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
