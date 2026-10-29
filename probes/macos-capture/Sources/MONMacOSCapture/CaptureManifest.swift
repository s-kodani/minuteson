import AVFoundation
import AudioToolbox
import CoreAudio
import CoreMedia
import Darwin
import Foundation
import ScreenCaptureKit

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
  var schemaVersion = 4
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
  var microphoneAudioUnreceived: Bool? = nil
  var recordingOwnerId: String? = nil
  var captureEnd: CaptureEndMetadata? = nil
  // Notification retention failure is separate from saved PCM/clock failure.
  // Keep audio finalization possible and retain a bounded diagnostic for the
  // native recovery UI; it must not claim all notifications were committed.
  var inputEventJournalError: String? = nil
  var inputEventJournalPersistence: InputJournalPersistence? = nil
  var microphoneSelectionError: String? = nil

  mutating func finalizeClock(_ clock: CaptureClockDomain) throws {
    guard schemaVersion == 4,
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
    guard schemaVersion == 4, completed.completed == true else {
      throw ProbeError.capture("completed capture test is required")
    }
    captureTest = completed
    let origin = try finishAvailableClocks()
    captureClock = try clock.metadata(originNanoseconds: origin)
  }

  // A missing lane is explicit evidence, never a silent placeholder. The
  // normal recording additionally requires its closed, empty raw producer.
  private mutating func finishAvailableClocks(owner: String? = nil) throws -> Int64 {
    guard microphone.frames > 0 || system.frames > 0 else {
      throw ProbeError.capture("both audio inputs are empty")
    }
    func empty(_ track: TrackStats, lane: String) throws {
      try track.requireWritable()
      guard track.frames == 0, track.callbackCount == 0, track.sampleRate == nil,
        track.channels == nil, track.firstCaptureNanoseconds == nil,
        track.captureFailureDetail == nil
      else { throw ProbeError.capture("empty \(lane) track has inconsistent capture evidence") }
      if let owner {
        guard let raw = track.rawSegments, raw.ownerId == owner, raw.lane == lane,
          raw.producerClosed, raw.sealedSegments == 0, raw.sealedFrames == 0,
          raw.lastEntrySHA256 == nil, raw.failure == nil,
          (lane == "system" ? track.systemRecovery?.state : track.microphoneRecovery?.state)
            == "stopped"
        else {
          throw ProbeError.capture("unreceived \(lane) track has inconsistent closure evidence")
        }
      }
    }
    let starts = [microphone.firstCaptureNanoseconds, system.firstCaptureNanoseconds].compactMap {
      $0
    }
    guard let origin = starts.min() else { throw ProbeError.capture("capture clock is missing") }
    if microphone.frames == 0 {
      try empty(microphone, lane: "microphone")
      microphone.clockSegments = []
      microphoneAudioUnreceived = true
    } else {
      try microphone.finishClock(originNanoseconds: origin)
    }
    if system.frames == 0 {
      try empty(system, lane: "system")
      system.clockSegments = []
      systemAudioUnreceived = true
    } else {
      try system.finishClock(originNanoseconds: origin)
    }
    return origin
  }

  mutating func finalizeStoppedClock(
    _ clock: CaptureClockDomain, microphoneClosure: CaptureClosure?, systemClosure: CaptureClosure?
  ) throws {
    guard schemaVersion == 4, captureTest == nil,
      let owner = recordingOwnerId, InputLevelReport.validOwner(owner),
      let microphoneClosure, let systemClosure
    else { throw ProbeError.capture("recording owner and both acquisition closures are required") }
    let origin = try finishAvailableClocks(owner: owner)
    let (metadata, ending) = try clock.ending(
      originNanoseconds: origin, microphone: microphoneClosure, system: systemClosure)
    guard let micEnd = microphone.frames == 0 ? 0 : microphone.clockSegments?.last?.timelineEndUs,
      let pcEnd = system.frames == 0 ? 0 : system.clockSegments?.last?.timelineEndUs,
      micEnd <= ending.microphoneClosedUs, pcEnd <= ending.systemClosedUs
    else { throw ProbeError.capture("saved PCM extends beyond its acquisition closure") }
    captureClock = metadata
    captureEnd = ending
    schemaVersion = 4
  }

  func encodedData() throws -> Data {
    let segmented =
      recordingOwnerId.map { owner in
        InputLevelReport.validOwner(owner)
          && microphone.rawSegments?.ownerId == owner
          && microphone.rawSegments?.lane == "microphone"
          && system.rawSegments?.ownerId == owner && system.rawSegments?.lane == "system"
      } ?? false
    let extended = segmented && schemaVersion == 4
    let encoder = JSONEncoder()
    encoder.outputFormatting = extended ? [.sortedKeys] : [.prettyPrinted, .sortedKeys]
    let original = try encoder.encode(self)
    var object = try JSONSerialization.jsonObject(with: original) as! [String: Any]
    for key in [
      "captureClock", "captureEnd", "recordingOwnerId", "microphoneDeviceUID",
      "targetApplicationIdentity", "inputEventJournalError", "inputEventJournalPersistence",
      "microphoneSelectionError",
    ] {
      if object[key] == nil { object[key] = NSNull() }
    }
    object["systemAudioUnreceived"] = systemAudioUnreceived ?? false
    object["microphoneAudioUnreceived"] = microphoneAudioUnreceived ?? false
    object["inputDiagnosticsState"] =
      inputEventJournalError != nil
      ? "unavailable" : (inputEventJournalPersistence == nil ? "unconfirmed" : "reported")
    let data = try JSONSerialization.data(withJSONObject: object, options: [.sortedKeys])
    let limit = extended ? 32 * 1024 * 1024 : 256 * 1024
    guard data.count <= limit else {
      throw ProbeError.capture("capture manifest exceeds the bounded reader capacity")
    }
    return data
  }
}
