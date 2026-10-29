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
    let processFamily = options.applicationIdentity.map { CaptureProcessFamily(root: $0) }
    let mic = MicrophoneRecorder(
      outputURL: options.output.appendingPathComponent("microphone.caf"), clock: clock,
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
      outputURL: options.output.appendingPathComponent("system.caf"), clock: clock,
      levelsEnabled: options.inputLevelOwner != nil, testWindow: testWindow,
      retries: options.recoversSystemAudio, backend: options.backend,
      mayContinue: {
        !systemControlled
          || (!FileManager.default.fileExists(atPath: systemStopRequest.path)
            && !parentHasExited(systemParentPID))
      }, inputEventObserver: inputEvents?.observer(.system), rawSegments: rawSystem,
      processFamily: processFamily)
    let tap = CoreAudioTapRecorder(
      outputURL: options.output.appendingPathComponent("system.caf"), clock: clock,
      levelsEnabled: options.inputLevelOwner != nil, rawSegments: rawSystem)
    var manifest = Manifest(
      schemaVersion: 4,
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
                try write(manifest, to: manifestURL)
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
                try write(manifest, to: manifestURL)
              }
            }
          }
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
              let status = microphone.microphoneRecovery,
              let history = microphone.inputEventHistory
            {
              try? InputStatusReport(
                ownerId: owner, revision: levelRevision, microphone: status,
                system: systemStats.systemRecovery,
                microphoneHistory: history,
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
            atPath: options.output.appendingPathComponent(lane + ".caf").path)
        else {
          throw ProbeError.capture("unreceived input has an unexpected CAF payload")
        }
      }
      if micStats.frames > 0 {
        try verifyRecordedCAF(
          options.output.appendingPathComponent("microphone.caf"), expected: micStats)
      }
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
    } else if let testWindow {
      manifest.captureTest = testWindow.metadata(
        firstSavedCaptureNanoseconds: [
          manifest.microphone.firstCaptureNanoseconds,
          manifest.system.firstCaptureNanoseconds,
        ].compactMap { $0 }.min())
    }
    try write(manifest, to: manifestURL)
    if manifest.state != "saved" {
      throw ProbeError.capture(manifest.errors.joined(separator: "; "))
    }
    print(manifestURL.path)
  }

  private static func parentHasExited(_ pid: pid_t?) -> Bool {
    guard let pid else { return false }
    return Darwin.kill(pid, 0) != 0 && errno == ESRCH
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
