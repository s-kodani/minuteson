import AVFoundation
import AudioToolbox
import CoreAudio
import CoreMedia
import Darwin
import Foundation
import ScreenCaptureKit

/// Product session orchestration. CLI dispatch and native acquisition drivers
/// are separate; lifecycle and raw-prefix decisions belong to the Rust core.
enum CaptureSession {
  /// Both acquisitions must stop and freeze their clocks before a microphone
  /// disk backlog can delay the system input's stop/closure. Always finalize
  /// the microphone's accepted prefix even when the other native stop fails.
  @MainActor
  static func stopInputs(
    microphone: MicrophoneRecorder,
    system: @MainActor () async throws -> CaptureClosure?
  ) async throws -> CaptureClosure? {
    microphone.stopAcquisition()
    defer { microphone.finishRecording() }
    return try await system()
  }

  /// Final metadata is optional evidence. Its failure cannot prevent the
  /// stopped writer from committing accepted PCM and its final header.
  static func finishSharedWav(
    _ writer: LiveWavSession, prepareEnd: () throws -> Int64?
  ) throws -> String? {
    var end: Int64?
    var diagnostic: String?
    do { end = try prepareEnd() } catch { diagnostic = String(describing: error) }
    try writer.finish(endNanoseconds: end)
    return diagnostic
  }

  @MainActor
  static func run(_ options: Options) async throws {
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
        try? FileManager.default.removeItem(
          at: options.output.appendingPathComponent("microphone-selection.request"))
        try? FileManager.default.removeItem(
          at: options.output.appendingPathComponent("microphone-selection.status.json"))
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
    let microphoneSelection =
      try options.publishesInputStatus
      ? MicrophoneSelectionReader(output: options.output, owner: options.inputLevelOwner!) : nil
    var publishedMicrophoneSelection: MicrophoneSelectionStatus?
    defer { journalStatus?.removeIfOwned() }
    let segmentCommits = RawSegmentCommitWorker()
    let liveWav =
      try options.writesWav && options.publishesInputStatus
      ? LiveWavSession(
        output: options.output, owner: options.inputLevelOwner!, commits: segmentCommits)
      : nil
    let testMicWav =
      try options.writesWav && testWindow != nil
      ? LiveWavSession(
        output: options.output, owner: options.inputLevelOwner!, commits: segmentCommits,
        fileName: "microphone.wav", onlyLane: 0) : nil
    let testSystemWav =
      try options.writesWav && testWindow != nil
      ? LiveWavSession(
        output: options.output, owner: options.inputLevelOwner!, commits: segmentCommits,
        fileName: "system.wav", onlyLane: 1) : nil
    let rawMic =
      options.publishesInputStatus || testMicWav != nil
      ? try RawRecordingSegments(
        output: options.output, owner: options.inputLevelOwner!, lane: "microphone",
        allowsClockRebasing: options.recoversCaptureClock,
        commits: segmentCommits, wavSession: testMicWav ?? liveWav) : nil
    let rawSystem =
      options.publishesInputStatus || testSystemWav != nil
      ? try RawRecordingSegments(
        output: options.output, owner: options.inputLevelOwner!, lane: "system",
        allowsClockRebasing: options.recoversCaptureClock,
        commits: segmentCommits, wavSession: testSystemWav ?? liveWav) : nil
    let trackExtension = options.writesWav ? "wav" : "caf"
    let systemStopRequest = options.output.appendingPathComponent("stop.request")
    let systemParentPID = options.parentPID
    let systemControlled = options.stopOnFile
    let processFamily = options.applicationIdentity.map { CaptureProcessFamily(root: $0) }
    let mic = MicrophoneRecorder(
      outputURL: options.output.appendingPathComponent("microphone." + trackExtension),
      clock: clock,
      microphoneUID: options.microphoneUID,
      applicationIdentity: options.applicationIdentity,
      processFamily: processFamily,
      levelsEnabled: options.inputLevelOwner != nil,
      testWindow: testWindow, inputEventObserver: inputEvents?.observer(.microphone),
      rawSegments: rawMic,
      mayContinue: {
        !systemControlled
          || (!FileManager.default.fileExists(atPath: systemStopRequest.path)
            && !parentHasExited(systemParentPID))
      })
    let system = SystemAudioRecorder(
      outputURL: options.output.appendingPathComponent("system." + trackExtension), clock: clock,
      levelsEnabled: options.inputLevelOwner != nil, testWindow: testWindow,
      retries: options.recoversSystemAudio, backend: options.backend,
      mayContinue: {
        !systemControlled
          || (!FileManager.default.fileExists(atPath: systemStopRequest.path)
            && !parentHasExited(systemParentPID))
      }, inputEventObserver: inputEvents?.observer(.system), rawSegments: rawSystem,
      processFamily: processFamily)
    let tap = CoreAudioTapRecorder(
      outputURL: options.output.appendingPathComponent("system." + trackExtension), clock: clock,
      levelsEnabled: options.inputLevelOwner != nil, rawSegments: rawSystem)
    var telemetry: CaptureTelemetryPublisher?
    defer { telemetry?.stop() }
    var manifest = Manifest(
      schemaVersion: options.writesWav ? 5 : 4,
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
    manifest.singleWav = liveWav?.snapshot()
    // Managed captures already require their canonical owned output. Keep
    // standalone measurement probes' existing /tmp path contract as well.
    let manifestOutput =
      options.publishesInputStatus || options.writesWav
      ? options.output : options.output.resolvingSymlinksInPath()
    let manifestFile = try createManifest(manifest, output: manifestOutput)
    let power = RecordingPowerAssertion()
    // Also covers a throw while writing the final manifest. Do not depend on
    // a WebView, the main window, or normal application termination to release.
    defer { try? power.release() }
    do {
      try power.acquire()
      manifest.power.acquired = true
      let keepsAvailableInput = options.publishesInputStatus || testWindow != nil
      func prepareSystem() async throws {
        if keepsAvailableInput {
          _ = await InitialCapturePreparation.run(
            operation: {
              try await system.start(
                targetPID: options.targetPIDs.first,
                applicationIdentity: options.applicationIdentity)
            }, cancel: { system.cancelInitialPreparation() })
        } else {
          try await system.start(
            targetPID: options.targetPIDs.first, applicationIdentity: options.applicationIdentity)
        }
      }
      if options.startsSystemBeforeMicrophone { try await prepareSystem() }
      if keepsAvailableInput {
        _ = await InitialCapturePreparation.run(
          operation: { try await mic.start() }, cancel: { mic.cancelInitialPreparation() })
      } else {
        try await mic.start()
      }
      manifest.microphoneDeviceUID = mic.deviceUID
      manifest.microphone = mic.snapshot()
      if options.usesStandaloneTapProbe {
        try tap.start(targetPIDs: options.targetPIDs)
      } else {
        if !options.startsSystemBeforeMicrophone { try await prepareSystem() }
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
      if let testWindow {
        do { try await mic.openTestWindow() } catch {
          // Retire an unprepared microphone before measuring the healthy PC
          // lane. The shared timestamp window still rejects queued pre-roll.
          mic.cancelInitialPreparation()
          try mic.snapshot().requireWritable()
          try testWindow.open(at: PCMInputLevel.nowNanoseconds())
        }
      }
      manifest.captureTest = testWindow?.metadata()
      try write(manifest, using: manifestFile)
      if let owner = options.inputLevelOwner {
        let standalone = options.usesStandaloneTapProbe
        telemetry = CaptureTelemetryPublisher(
          output: options.output, owner: owner, publishesStatus: options.publishesInputStatus,
          snapshots: { (mic.snapshot(), standalone ? tap.snapshot() : system.snapshot()) },
          journal: {
            if let inputEvents { try? journalStatus?.publish(inputEvents.persistence()) }
          })
        telemetry?.start()
      }
      if options.stopOnFile {
        let stopRequest = options.output.appendingPathComponent("stop.request")
        let startedAt = ContinuousClock.now
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
          if let microphoneSelection {
            do {
              if let request = try microphoneSelection.next() {
                try mic.selectInput(request.uid)
                try microphoneSelection.acknowledge(request)
                manifest.microphoneSelectionError = nil
              }
            } catch {
              let detail = "microphone selection request was rejected"
              if manifest.microphoneSelectionError != detail {
                manifest.microphoneSelectionError = detail
                try write(manifest, using: manifestFile)
              }
            }
          }
          mic.maintain()
          if let owner = options.inputLevelOwner, let microphoneSelection {
            do {
              let revision = (publishedMicrophoneSelection?.revision ?? 0) + 1
              let current = try mic.selectionStatus(
                owner: owner, revision: revision,
                requestRevision: microphoneSelection.revision)
              let previous = publishedMicrophoneSelection
              if previous?.requestRevision != current.requestRevision
                || previous?.mode != current.mode
                || previous?.deviceUID != current.deviceUID
                || previous?.source != current.source
                || previous?.fallbackReason != current.fallbackReason
              {
                try current.publish(to: options.output)
                publishedMicrophoneSelection = current
              }
            } catch {
              let detail = "microphone selection status could not be confirmed"
              if manifest.microphoneSelectionError != detail {
                manifest.microphoneSelectionError = detail
                try write(manifest, using: manifestFile)
              }
            }
          }
          if !options.usesStandaloneTapProbe { system.maintain() }
          if liveWav != nil, let now = PCMInputLevel.nowNanoseconds() {
            mic.observeUnreceived(at: now)
            system.observeUnreceived(at: now)
            manifest.singleWav = liveWav?.snapshot()
          }
          let microphone = mic.snapshot()
          let systemStats = options.usesStandaloneTapProbe ? tap.snapshot() : system.snapshot()
          try microphone.requireWritable()
          try systemStats.requireWritable()
          if microphone.microphoneRecovery != manifest.microphone.microphoneRecovery
            || systemStats.systemRecovery != manifest.system.systemRecovery
            || (microphone.clockAlignment != nil) != (manifest.microphone.clockAlignment != nil)
            || (systemStats.clockAlignment != nil) != (manifest.system.clockAlignment != nil)
          {
            manifest.microphone = microphone
            manifest.system = systemStats
            manifest.timingEstimated =
              microphone.clockAlignment != nil || systemStats.clockAlignment != nil ? true : nil
            try write(manifest, using: manifestFile)
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
      telemetry?.stop()
      testWindow?.finish(at: PCMInputLevel.nowNanoseconds())
      let systemClosure = try await stopInputs(microphone: mic) {
        if options.publishesInputStatus {
          return options.usesStandaloneTapProbe
            ? try tap.stopRecording() : try await system.stopRecording()
        }
        if options.usesStandaloneTapProbe { try tap.stop() } else { try await system.stop() }
        return nil
      }
      let micStats = mic.snapshot()
      let systemStats = options.usesStandaloneTapProbe ? tap.snapshot() : system.snapshot()
      manifest.microphone = micStats
      manifest.system = systemStats
      if let liveWav {
        let diagnostic = try finishSharedWav(liveWav) {
          let geometry = liveWav.captureGeometry()
          guard let clock, let microphoneClosure = mic.captureClosure, let systemClosure,
            let origin = geometry.origin
          else { throw ProbeError.capture("single WAV acquisition ending is unavailable") }
          let (metadata, observed) = try clock.ending(
            originNanoseconds: origin, microphone: microphoneClosure, system: systemClosure)
          let microphoneEnd = max(
            observed.microphoneClosedUs, (geometry.microphoneEnd - origin) / 1000)
          let systemEnd = max(
            observed.systemClosedUs, (geometry.systemEnd - origin) / 1000)
          let end = max(microphoneEnd, systemEnd)
          guard end <= (9_007_199_254_740_991 - 1_000_000) / 1000 else {
            throw ProbeError.capture("single WAV ending is outside precision bounds")
          }
          let estimated =
            microphoneEnd != observed.microphoneClosedUs
            || systemEnd != observed.systemClosedUs || micStats.clockAlignment != nil
            || systemStats.clockAlignment != nil
          manifest.captureClock = CaptureClockMetadata(
            originUtcMs: metadata.originUtcMs,
            uncertaintyNs: estimated
              ? max(metadata.uncertaintyNs, end * 1000 + 1_000_000) : metadata.uncertaintyNs)
          manifest.captureEnd = CaptureEndMetadata(
            microphoneClosedUs: microphoneEnd, systemClosedUs: systemEnd, endUs: end)
          manifest.timingEstimated = estimated ? true : nil
          manifest.observedCaptureEnd = estimated ? observed : nil
          return origin + end * 1000
        }
        if let diagnostic { manifest.errors.append(diagnostic) }
        manifest.singleWav = liveWav.snapshot()
      } else {
        try micStats.requireWritable()
        try systemStats.requireWritable()
        if let testWindow, !testWindow.metadata().completed {
          throw ProbeError.capture("five-second test stopped before its window completed")
        }
        guard micStats.frames > 0 || systemStats.frames > 0,
          (micStats.frames > 0 && systemStats.frames > 0) || testWindow != nil
            || systemClosure != nil,
          micStats.appendFailures == 0, systemStats.appendFailures == 0
        else {
          throw ProbeError.capture("audio track is empty or contains failed writes")
        }
        for (lane, stats) in [("microphone", micStats), ("system", systemStats)]
        where stats.frames == 0 {
          guard
            !FileManager.default.fileExists(
              atPath: options.output.appendingPathComponent(lane + "." + trackExtension).path)
          else {
            throw ProbeError.capture("unreceived input has an unexpected audio payload")
          }
        }
        if let rawMic {
          try rawMic.verifyClosed(expected: micStats)
        } else if micStats.frames > 0 {
          try verifyRecordedCAF(
            options.output.appendingPathComponent("microphone.caf"), expected: micStats)
        }
        if let rawSystem {
          try rawSystem.verifyClosed(expected: systemStats)
        } else if systemStats.frames > 0 {
          try verifyRecordedCAF(
            options.output.appendingPathComponent("system.caf"), expected: systemStats)
        }
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
      }
      try testMicWav?.finish()
      try testSystemWav?.finish()
      if testMicWav != nil, micStats.frames > 0 {
        try verifyRecordedWAV(
          options.output.appendingPathComponent("microphone.wav"), expected: micStats)
      }
      if testSystemWav != nil, systemStats.frames > 0 {
        try verifyRecordedWAV(
          options.output.appendingPathComponent("system.wav"), expected: systemStats)
      }
      manifest.state = "saved"
    } catch {
      // Keep the initiating failure first. Cleanup can also fail when an
      // input never started, but must not replace the permission/device error.
      manifest.errors.append(String(describing: error))
      telemetry?.stop()
      testWindow?.finish(at: PCMInputLevel.nowNanoseconds())
      do {
        _ = try await stopInputs(microphone: mic) {
          if options.usesStandaloneTapProbe { try tap.stop() } else { try await system.stop() }
          return nil
        }
      } catch { manifest.errors.append(String(describing: error)) }
      do { try liveWav?.preserveAcceptedPrefix() } catch {
        manifest.errors.append(String(describing: error))
      }
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
    manifest.singleWav = liveWav?.snapshot()
    manifest.captureTest = testWindow?.metadata()
    if manifest.state != "saved" {
      manifest.microphone = mic.snapshot()
      manifest.system = options.usesStandaloneTapProbe ? tap.snapshot() : system.snapshot()
    } else if let testWindow {
      manifest.captureTest = testWindow.metadata(
        firstSavedCaptureNanoseconds: [
          manifest.microphone.firstCaptureNanoseconds,
          manifest.system.firstCaptureNanoseconds,
        ].compactMap { $0 }.min())
    }
    try write(manifest, using: manifestFile)
    if manifest.state != "saved" {
      throw ProbeError.capture(manifest.errors.joined(separator: "; "))
    }
    print(manifestURL.path)
  }

  private static func parentHasExited(_ pid: pid_t?) -> Bool {
    guard let pid else { return false }
    return Darwin.kill(pid, 0) != 0 && errno == ESRCH
  }

  static func createManifest(_ manifest: Manifest, output: URL) throws -> InputEventJournalFile {
    try InputEventJournalFile(
      output: output, initial: manifest.encodedData(), leaf: .captureManifest)
  }

  static func write(_ manifest: Manifest, using file: InputEventJournalFile) throws {
    try file.replace(manifest.encodedData())
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
