import AVFoundation
import AudioToolbox
import CoreAudio
import CoreMedia
import Darwin
import Foundation
import ScreenCaptureKit

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
  private let processFamily: CaptureProcessFamily?
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
    rawSegments: RawRecordingSegments? = nil,
    processFamily: CaptureProcessFamily? = nil
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
    self.processFamily = processFamily
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

  func cancelInitialPreparation() {
    lock.withLock {
      recovery?.interrupt("acquisition_timeout", unavailable: true)
      pcm.invalidateLevels()
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
        driver: NativeCoreAudioCaptureDriver(
          scope: scope, processFamily: processFamily, mayContinue: allowed),
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
      if !retries, testWindow == nil || testWindow?.accepting == true { pcm.recordFailure(error) }
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
      && streamStopError == nil && lock.withLock { recovery?.operation == nil }
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
          if !retries, testWindow == nil || testWindow?.accepting == true {
            pcm.recordFailure(error)
          }
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
