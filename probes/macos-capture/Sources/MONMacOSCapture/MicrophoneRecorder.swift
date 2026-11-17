import AVFoundation
import AudioToolbox
import CoreAudio
import CoreMedia
import Darwin
import Foundation
import ScreenCaptureKit

final class MicrophoneRecorder: @unchecked Sendable {
  private(set) var captureClosure: CaptureClosure?
  private enum RebindError: Error { case formatChanged }
  private var engine: (any MicrophoneCaptureEngine)?
  private var configurationChanged = false
  private var configurationRevision: UInt64 = 0
  private var recovery: MicrophoneRecovery?
  private var lastRecoveryAttempt: ContinuousClock.Instant?
  private var inputRetry = CaptureInputRetry()
  private let retryOrigin: ContinuousClock.Instant
  private var preparationReports = 0
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
  private struct PCMFlow {
    let token: MicrophoneRecovery.Binding
    var deadline: ContinuousClock.Instant
  }
  private var pcmFlow: PCMFlow?
  static let pcmTimeout: Duration = .seconds(3)
  private let outputURL: URL
  private let lock = NSLock()
  private var file: AVAudioFile?
  private var captureFormat: AVAudioFormat?
  private var normalizer: MicrophonePCMNormalizer?
  private var conversionAllowedUID: String?
  private var tapInstalled = false
  private var stats = TrackStats()
  private let clock: CaptureClockDomain?
  private let levelsEnabled: Bool
  private let testWindow: CaptureTestWindow?

  private var requestedUID: String?
  private let applicationIdentity: CaptureApplicationIdentity?
  private let processFamily: CaptureProcessFamily?
  private let inputEventObserver: InputEventJournal.Observer?
  private let makeEngine: @Sendable () throws -> any MicrophoneCaptureEngine
  private let devices: @Sendable () throws -> CaptureInputDevices
  private let permission: @Sendable () -> AVAuthorizationStatus
  private let requestPermission: @Sendable () async -> Bool
  private let rawSegments: RawRecordingSegments?
  private var recordingFinalizationAttempted = false
  private let recordingFinalizationLock = NSLock()
  private let mayContinue: @Sendable () -> Bool
  private let controlNow: @Sendable () -> ContinuousClock.Instant
  private var initialPreparationCancelled = false
  private var initialPreparationPending = false
  private var initialNativePending = false
  private var stopped = false
  private var selectedDeviceID: AudioObjectID?
  private var automaticSource: InputDevices.AutomaticSource?
  private(set) var deviceUID: String?

  init(
    outputURL: URL, clock: CaptureClockDomain? = nil, microphoneUID: String? = nil,
    applicationIdentity: CaptureApplicationIdentity? = nil,
    processFamily: CaptureProcessFamily? = nil,
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
    self.applicationIdentity = applicationIdentity
    self.processFamily = processFamily
    self.levelsEnabled = levelsEnabled
    self.testWindow = testWindow
    self.inputEventObserver = inputEventObserver
    self.rawSegments = rawSegments
    self.mayContinue = mayContinue
    self.controlNow = controlNow
    self.retryOrigin = controlNow()
    self.makeEngine = makeEngine
    self.devices = devices
    self.permission = permission
    self.requestPermission = requestPermission
    stats.clockCapacity = rawSegments == nil ? 1024 : CapturedClockMap.maximumTrackSegments
  }

  func start() async throws {
    try Task.checkCancellation()
    try lock.withLock {
      guard recovery == nil, !stopped, !initialPreparationCancelled, mayContinue() else {
        throw ProbeError.capture("microphone start is already owned or stopped")
      }
      recovery = try MicrophoneRecovery(uid: requestedUID, observe: inputEventObserver)
      deviceUID = requestedUID
      initialPreparationPending = true
    }
    defer {
      lock.withLock {
        initialPreparationPending = false
        initialNativePending = false
        if testWindow == nil, !stopped { initialPreparationCancelled = false }
      }
    }
    let selected: CaptureInputDevice
    do {
      let available = try devices().devices
      let automatic =
        try requestedUID == nil
        ? InputDevices.automaticChoice(
          from: available, application: applicationIdentity,
          processFamily: processFamily) : nil
      selected = try automatic?.device ?? InputDevices.choose(requestedUID, from: available)
      try lock.withLock {
        guard !stopped, !initialPreparationCancelled, mayContinue() else {
          throw ProbeError.capture("microphone device discovery is obsolete")
        }
        try recovery?.retarget(selected.uid)
        deviceUID = selected.uid
        automaticSource = automatic?.source
      }
    } catch {
      suspend("metadata_unavailable", unavailable: true)
      throw error
    }
    var authorization = permission()
    if authorization == .notDetermined {
      _ = await requestPermission()
      authorization = permission()
    }
    guard authorization == .authorized else {
      suspend("permission_unavailable", unavailable: true)
      throw ProbeError.permission(CaptureFailureGuide.microphonePermission(authorization))
    }
    do {
      try lock.withLock {
        guard !stopped, !initialPreparationCancelled, mayContinue() else {
          throw ProbeError.capture("microphone initialization is obsolete")
        }
        initialNativePending = true
      }
      try configure(selected)
    } catch {
      reportPreparationFailure(error, stage: "initial")
      suspend("restart_failed", unavailable: true)
      if clock == nil || testWindow != nil || !mayContinue() {
        lock.withLock { recovery?.stop() }
      }
      throw ProbeError.capture(CaptureFailureGuide.microphoneInitialization(error))
    }
  }

  func cancelInitialPreparation() {
    let mayRetire = lock.withLock {
      initialPreparationCancelled = initialPreparationPending || testWindow != nil
      recovery?.interrupt("restart_failed", unavailable: true)
      selectedDeviceID = nil
      normalPreparation = nil
      normalAdmission = nil
      pcmFlow = nil
      stats.inputLevel.invalidate()
      return !initialNativePending
    }
    // Native start may still be on its owner thread. Its catch path retires
    // the engine after returning; timeout cannot stop/deallocate it in parallel.
    if mayRetire { releaseEngine() }
  }

  var initialAcquisitionPending: Bool { lock.withLock { initialPreparationPending } }

  func selectInput(_ uid: String?) throws {
    guard uid.map(InputDevices.validUID) ?? true else {
      throw ProbeError.capture("requested microphone UID is invalid")
    }
    guard lock.withLock({ recovery?.status.state != "stopped" && recovery != nil }),
      mayContinue()
    else { throw ProbeError.capture("microphone selection is not active") }
    requestedUID = uid
    if uid != nil { lock.withLock { automaticSource = nil } }
  }

  func selectionStatus(owner: String, revision: Int64, requestRevision: Int64) throws
    -> MicrophoneSelectionStatus
  {
    try lock.withLock {
      guard let deviceUID else {
        throw ProbeError.capture("microphone selection has no device")
      }
      let source: String
      let fallbackReason: String?
      if requestedUID != nil {
        source = requestedUID == deviceUID ? "fixed" : "pending"
        fallbackReason = nil
      } else {
        switch automaticSource {
        case .application:
          source = "application"
          fallbackReason = nil
        case .systemDefault(let reason):
          source = "systemDefault"
          fallbackReason = reason
        case nil:
          source = "pending"
          fallbackReason = nil
        }
      }
      return MicrophoneSelectionStatus(
        ownerId: owner, revision: revision, requestRevision: requestRevision,
        mode: requestedUID == nil ? "automatic" : "fixed", deviceUID: deviceUID,
        source: source, fallbackReason: fallbackReason)
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
      captureFormat = format
      if rawSegments != nil {
        stats.sampleRate = format.sampleRate
        stats.channels = format.channelCount
        normalizer = nil
      } else if let file {
        if MicrophonePCMFormat.matches(file.processingFormat, format),
          stats.sampleRate == format.sampleRate,
          stats.channels == format.channelCount
        {
          normalizer = nil
        } else {
          guard selected.uid == conversionAllowedUID,
            let converter = MicrophonePCMNormalizer(
              from: format, to: file.processingFormat)
          else { throw RebindError.formatChanged }
          normalizer = converter
        }
      } else {
        guard stats.frames == 0 else { throw ProbeError.capture("microphone writer was lost") }
        file = try AVAudioFile(
          forWriting: outputURL, settings: format.settings, commonFormat: .pcmFormatFloat32,
          interleaved: false)
        stats.sampleRate = format.sampleRate
        stats.channels = format.channelCount
        normalizer = nil
      }
      guard
        rawSegments != nil
          || (file.map {
            normalizer != nil || MicrophonePCMFormat.matches($0.processingFormat, format)
          } ?? false)
      else {
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
    try engine.installTap(format: format) { [weak self, weak engine] buffer, when in
      guard let self, let engine else { return }
      let receivedAt = self.levelsEnabled ? PCMInputLevel.nowNanoseconds() : nil
      let revision: UInt64? = self.lock.withLock {
        guard self.mayContinue(), self.admitsPCM(token),
          !self.configurationChanged, self.selectedDeviceID == token.objectID
        else { return nil }
        return self.configurationRevision
      }
      guard let revision else { return }
      // The native input uses an ownership proof revoked by metadata listeners;
      // HAL reads belong to the control owner, never this audio callback.
      // Fence that proof against notification and binding retirement again
      // under the writer lock before admitting any PCM.
      let identityError: Error?
      do {
        try engine.verifyCallbackDevice(token)
        identityError = nil
      } catch is MicrophoneDeviceProofPending {
        return
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
        if let rawSegments = self.rawSegments {
          guard let captureFormat = self.captureFormat,
            self.stats.acceptsMicrophonePCM(buffer, at: when, processingFormat: captureFormat)
          else {
            throw CaptureInputFault.invalidFormat
          }
          var sourceFrame: Int64?
          if when.isSampleTimeValid {
            let (frame, overflow) = when.sampleTime.addingReportingOverflow(sourceFrameOffset)
            guard !overflow else { throw CaptureInputFault.sourceCounter }
            sourceFrame = frame
          }
          try rawSegments.enqueue(
            buffer, observation: observation, sourceFrame: sourceFrame, sourceEpoch: token.epoch)
          if self.levelsEnabled { self.stats.inputLevel.receive(buffer, at: receivedAt) }
          self.stats.callbackCount += 1
          self.stats.frames += Int64(buffer.frameLength)
          if self.normalAdmission == token {
            try self.recovery?.activate(token)
            self.normalAdmission = nil
          }
          self.receivedPCM(token)
          return
        }
        guard let file = self.file else {
          throw ProbeError.capture("microphone CAF writer is closed")
        }
        var savedWhen = when
        let converted = self.normalizer != nil
        if let normalizer = self.normalizer {
          guard when.isHostTimeValid,
            !when.isSampleTimeValid || when.sampleRate == normalizer.inputFormat.sampleRate
          else { throw ProbeError.capture("changed microphone clock format is invalid") }
          buffer = try normalizer.convert(buffer)
          savedWhen = AVAudioTime(hostTime: when.hostTime)
        }
        guard
          self.stats.acceptsMicrophonePCM(
            buffer, at: savedWhen, processingFormat: file.processingFormat)
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
        if !converted, when.isSampleTimeValid {
          let (frame, overflow) = when.sampleTime.addingReportingOverflow(sourceFrameOffset)
          guard !overflow else { throw ProbeError.capture("microphone frame offset overflows") }
          savedSourceFrame = frame
        }
        try CaptureInputFault.validate(buffer)
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
        self.receivedPCM(token)
      } catch let fault as CaptureInputFault {
        if self.clock != nil, self.testWindow == nil, self.rawSegments != nil {
          self.inputRetry.fault(fault, at: self.retryNow(), input: "microphone")
          self.retireBinding("invalid_audio_data", unavailable: true)
        } else {
          self.stats.appendFailures += 1
          self.stats.retainCaptureFailure(fault)
          self.stats.inputLevel.invalidate()
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
      do { try engine.verifyDevice(token) } catch is MicrophoneDeviceProofPending {
        continue
      }
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

  @discardableResult
  private func releaseEngine() -> Bool {
    engine?.observeConfiguration(nil)
    if tapInstalled, let engine {
      engine.removeTap()
      tapInstalled = false
    }
    do { try engine?.stop() } catch {
      // Keep the native owner alive until an actual stop succeeds. A source
      // failure must never fabricate acquisition closure or permit overlap.
      lock.withLock {
        stats.appendFailures += 1
        stats.retainCaptureFailure(error)
      }
      return false
    }
    engine = nil
    return true
  }

  // Called only with the callback/state lock held.
  private func admitsPCM(_ token: MicrophoneRecovery.Binding) -> Bool {
    recovery?.accepts(token) == true
      || (normalAdmission == token && recovery?.owns(token) == true)
  }

  // Called under the callback lock, after valid PCM has been accepted. Silent
  // samples count; engine flags, notifications and retired callbacks do not.
  private func receivedPCM(_ token: MicrophoneRecovery.Binding) {
    inputRetry.valid(at: retryNow())
    if pcmFlow?.token == token {
      pcmFlow?.deadline = controlNow().advanced(by: Self.pcmTimeout)
    }
  }

  private func retryNow() -> Int64 {
    let elapsed = retryOrigin.duration(to: controlNow()).components
    return max(0, elapsed.seconds * 1_000_000_000 + elapsed.attoseconds / 1_000_000_000)
  }

  private func reportPreparationFailure(_ error: Error, stage: String) {
    let status = lock.withLock { () -> MicrophoneRecovery.Status? in
      guard preparationReports < 16 else { return nil }
      preparationReports += 1
      return recovery?.status
    }
    guard let status else { return }
    let detail = String(String(describing: error).prefix(1200)).replacingOccurrences(
      of: "\n", with: " ")
    CaptureInputRetry.diagnostic(
      "mon_capture_microphone_preparation: stage=\(stage) epoch=\(status.sourceEpoch) error=\(detail)"
    )
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
      pending.nextProof = now.advanced(by: .milliseconds(100))
      normalPreparation = pending
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
        pcmFlow = PCMFlow(
          token: pending.token, deadline: now.advanced(by: Self.pcmTimeout))
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
    guard let testWindow else {
      throw ProbeError.capture("five-second test microphone is not initialized")
    }
    let engine = try lock.withLock {
      guard !initialPreparationPending, !initialNativePending, let engine = self.engine else {
        throw ProbeError.capture("five-second test microphone preparation is still pending")
      }
      return engine
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
      do { try engine.verifyDevice(token) } catch is MicrophoneDeviceProofPending {
        previousRevision = nil
        stableProofs = 0
        continue
      }
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
      retireBinding(reason, unavailable: unavailable)
    }
    releaseEngine()
    flushRetiredSource()
    // The current CAF remains open. Recreating it would overwrite prior PCM.
  }

  private func flushRetiredSource() {
    do {
      if lock.withLock({ inputRetry.recovering }) { try rawSegments?.retireSource() }
    } catch {
      lock.withLock {
        stats.appendFailures += 1
        stats.retainCaptureFailure(error)
      }
    }
  }

  // Caller holds the callback lock. Invalidation and the last-PCM timeout
  // decision must be atomic, so a newly admitted packet cannot lose this race.
  private func retireBinding(_ reason: String?, unavailable: Bool) {
    if let reason {
      recovery?.interrupt(reason, unavailable: unavailable)
    } else {
      recovery?.interruptForRetry()
    }
    selectedDeviceID = nil
    normalPreparation = nil
    normalAdmission = nil
    pcmFlow = nil
    stats.inputLevel.invalidate()
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
    try lock.withLock { try stats.requireWritable(raw: rawSegments?.snapshot()) }
    // Invalidation takes the callback lock, draining already-admitted writes.
    // After removeTap/stop, late callbacks still carry the retired binding and
    // are rejected before timestamps, CAF or raw writes are considered.
    return try clock.quiesced(.microphone, for: request)
  }

  func maintain() {
    // Formal bounded probes and a private 5-second test retain their existing
    // fail/verify contract; only an active production recording retries inputs.
    if lock.withLock({ recovery?.uid == nil }), mayContinue(), clock != nil, testWindow == nil,
      lock.withLock({ !stopped && !initialPreparationCancelled && !initialPreparationPending }),
      permission() == .authorized
    {
      // No device metadata was available at explicit start. Discovery may
      // recover, but this heartbeat must never request OS permission.
      do {
        let available = try devices().devices
        let automatic =
          try requestedUID == nil
          ? InputDevices.automaticChoice(
            from: available, application: applicationIdentity,
            processFamily: processFamily) : nil
        let selected = try automatic?.device ?? InputDevices.choose(requestedUID, from: available)
        try lock.withLock {
          guard !stopped, !initialPreparationCancelled, recovery != nil else {
            throw ProbeError.capture("microphone preparation is obsolete")
          }
          try recovery?.retarget(selected.uid)
          deviceUID = selected.uid
          automaticSource = automatic?.source
        }
      } catch { return }
    }
    guard mayContinue(), clock != nil, testWindow == nil,
      lock.withLock({
        !stopped && !initialPreparationCancelled && !initialPreparationPending
          && recovery?.status.state != "stopped"
          && (try? stats.requireWritable(raw: rawSegments?.snapshot())) != nil
      }), let uid = deviceUID
    else { return }
    let report: CaptureInputDevices
    do { report = try devices() } catch {
      suspend("metadata_unavailable", unavailable: true)
      return
    }
    guard report.microphonePermission == "authorized", permission() == .authorized else {
      suspend("permission_unavailable", unavailable: true)
      return
    }
    let desired: CaptureInputDevice
    do {
      let automatic =
        try requestedUID == nil
        ? InputDevices.automaticChoice(
          from: report.devices, application: applicationIdentity, processFamily: processFamily)
        : nil
      desired = try automatic?.device ?? InputDevices.choose(requestedUID, from: report.devices)
      lock.withLock { automaticSource = automatic?.source }
    } catch {
      let known = report.devices.filter { $0.uid == uid }
      suspend(
        requestedUID == nil
          ? "device_disconnected"
          : known.isEmpty ? "device_disconnected" : "identity_ambiguous",
        unavailable: requestedUID == nil || !known.isEmpty)
      return
    }
    if desired.uid != uid {
      // The previous callback lock drains before a new engine is created.
      // Recovery keeps its epoch and rejects any late callbacks from the old
      // UID even when HAL later recycles a transient device object ID.
      suspend("device_changed")
      do {
        try lock.withLock {
          guard recovery != nil else { throw ProbeError.capture("microphone owner is missing") }
          try recovery?.retarget(desired.uid)
          deviceUID = desired.uid
          conversionAllowedUID = desired.uid
        }
      } catch {
        suspend("identity_ambiguous", unavailable: true)
        return
      }
      lastRecoveryAttempt = nil
    }
    let matches = report.devices.filter { $0.uid == desired.uid }
    guard matches.count == 1 else {
      suspend(
        matches.isEmpty ? "device_disconnected" : "identity_ambiguous",
        unavailable: !matches.isEmpty)
      return
    }
    let selected = matches[0]
    let flowExpired = lock.withLock {
      guard let flow = pcmFlow, recovery?.owns(flow.token) == true,
        controlNow() >= flow.deadline
      else { return false }
      stats.microphoneFlowTimeouts = (stats.microphoneFlowTimeouts ?? 0) + 1
      inputRetry.failedAttempt(at: retryNow())
      retireBinding("engine_changed", unavailable: true)
      return true
    }
    if flowExpired {
      releaseEngine()
      flushRetiredSource()
      lastRecoveryAttempt = nil
    }
    if lock.withLock({ normalPreparation != nil && selectedDeviceID == selected.objectID }) {
      do { try maintainNormalPreparation() } catch is MicrophoneDeviceProofPending {
        // Keep the same input fenced until the next spaced proof. The original
        // preparation deadline still bounds a stream of racing notifications.
        return
      } catch RebindError.formatChanged {
        reportPreparationFailure(RebindError.formatChanged, stage: "stabilization")
        lock.withLock { inputRetry.failedAttempt(at: retryNow()) }
        suspend("format_changed", unavailable: true)
      } catch {
        reportPreparationFailure(error, stage: "stabilization")
        lock.withLock { inputRetry.failedAttempt(at: retryNow()) }
        suspend("restart_failed", unavailable: true)
      }
      return
    }
    let healthy = lock.withLock {
      (recovery?.status.state == "connected" || normalAdmission != nil)
        && selectedDeviceID == selected.objectID
    }
    if healthy, let engine, engine.isRunning {
      do {
        let proof = try lock.withLock {
          let binding: MicrophoneRecovery.Binding
          if let token = normalAdmission, recovery?.owns(token) == true {
            binding = token
          } else if let current = try recovery?.currentBinding() {
            binding = current
          } else {
            throw ProbeError.capture("microphone binding is missing")
          }
          guard let captureFormat else {
            throw ProbeError.capture("microphone input format is missing")
          }
          return (binding, captureFormat, configurationRevision)
        }
        try engine.verifyDevice(proof.0)
        guard MicrophonePCMFormat.matches(proof.1, engine.inputFormat) else {
          throw RebindError.formatChanged
        }
        lock.withLock {
          // A notification is a request to inspect the input, not proof that
          // it was lost. Keep a running, unchanged physical input open rather
          // than renegotiating a Bluetooth profile for an output-only event.
          if mayContinue(), recovery?.owns(proof.0) == true,
            configurationRevision == proof.2
          {
            configurationChanged = false
          }
        }
        return
      } catch is MicrophoneDeviceProofPending {
        // Listener and metadata read raced. Recheck on the next heartbeat;
        // opening the device again would turn notification noise into IO loss.
        return
      } catch {
        // Continue only after invalidating the old engine binding.
      }
    }
    // A retired engine is already interrupted. Preserve its diagnosis during
    // backoff instead of changing unavailable/restart_failed to engine_changed
    // on every heartbeat without any new acquisition evidence.
    if engine != nil { suspend() }
    guard engine == nil,
      lock.withLock({ (try? stats.requireWritable(raw: rawSegments?.snapshot())) != nil })
    else { return }
    let now = controlNow()
    guard lock.withLock({ inputRetry.permits(retryNow()) }) else { return }
    if let lastRecoveryAttempt, now < lastRecoveryAttempt.advanced(by: .seconds(1)) { return }
    lastRecoveryAttempt = now
    do { try configure(selected) } catch RebindError.formatChanged {
      reportPreparationFailure(RebindError.formatChanged, stage: "recovery")
      lock.withLock { inputRetry.failedAttempt(at: retryNow()) }
      suspend("format_changed", unavailable: true)
    } catch {
      reportPreparationFailure(error, stage: "recovery")
      lock.withLock { inputRetry.failedAttempt(at: retryNow()) }
      suspend("restart_failed", unavailable: true)
    }
  }

  func stop() {
    stopAcquisition()
    finishRecording()
  }

  /// Close native input and freeze its clock before any lane waits for disk.
  /// Session orchestration stops the other input before finishRecording().
  func stopAcquisition() {
    let pending = lock.withLock {
      stopped = true
      recovery?.stop()
      selectedDeviceID = nil
      normalPreparation = nil
      normalAdmission = nil
      pcmFlow = nil
      stats.inputLevel.invalidate()
      if initialNativePending {
        stats.appendFailures += 1
        stats.retainCaptureFailure(
          ProbeError.capture("microphone acquisition is still pending; closure is unproven"))
      }
      return initialNativePending
    }
    guard !pending else { return }
    guard releaseEngine() else { return }
    lock.withLock {
      if captureClosure == nil {
        do { captureClosure = try clock?.close(.microphone) } catch {
          stats.clockFailure = stats.clockFailure ?? String(describing: error)
        }
      }
      let ownedEmptyFile = file != nil && stats.frames == 0 && stats.callbackCount == 0
      file = nil
      if ownedEmptyFile, stats.captureFailure == nil {
        do {
          try FileManager.default.removeItem(at: outputURL)
          stats.sampleRate = nil
          stats.channels = nil
        } catch {
          stats.appendFailures += 1
          stats.retainCaptureFailure(error)
        }
      }
    }
  }

  func finishRecording() {
    recordingFinalizationLock.lock()
    defer { recordingFinalizationLock.unlock() }
    let shouldFinish = lock.withLock {
      guard stopped, !initialNativePending, !recordingFinalizationAttempted else { return false }
      recordingFinalizationAttempted = true
      return true
    }
    guard shouldFinish else { return }
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
    result.microphoneRecovery =
      recovery?.status
      ?? (stopped
        ? MicrophoneRecovery.Status(state: "stopped", sourceEpoch: 0, interruptions: 0, reason: nil)
        : nil)
    result.inputEventHistory = recovery?.history
    result.rawSegments = rawSegments?.snapshot()
    if result.captureFailure != nil { result.inputLevel.invalidate() }
    return rawSegments?.applyingSavedAudio(to: result) ?? result
  }
}
