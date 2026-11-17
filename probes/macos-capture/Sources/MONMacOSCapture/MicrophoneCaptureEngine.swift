import AVFoundation
import AudioToolbox
import CoreAudio
import Foundation

enum MicrophonePCMFormat {
  static func matches(_ expected: AVAudioFormat, _ actual: AVAudioFormat) -> Bool {
    expected.isEqual(actual) && expected.sampleRate == actual.sampleRate
      && expected.channelCount == actual.channelCount
      && expected.commonFormat == actual.commonFormat
      && expected.isInterleaved == actual.isInterleaved
  }
}

protocol MicrophoneCaptureEngine: AnyObject, Sendable {
  var inputFormat: AVAudioFormat { get }
  var isRunning: Bool { get }
  func selectDevice(_ binding: MicrophoneRecovery.Binding) throws
  func verifyDevice(_ binding: MicrophoneRecovery.Binding) throws
  func verifyCallbackDevice(_ binding: MicrophoneRecovery.Binding) throws
  func observeConfiguration(_ handler: (@Sendable () -> Void)?)
  func installTap(
    format: AVAudioFormat,
    handler: @escaping @Sendable (AVAudioPCMBuffer, AVAudioTime) -> Void) throws
  func removeTap()
  func prepare()
  func start() throws
  func stop() throws
}
extension MicrophoneCaptureEngine {
  func verifyCallbackDevice(_ binding: MicrophoneRecovery.Binding) throws {
    try verifyDevice(binding)
  }
}

/// Own only an input AUHAL. AVAudioEngine automatically creates a default
/// input/output aggregate for split Bluetooth endpoints and overwrites an
/// explicitly selected input device; it must not own production microphone IO.
final class NativeMicrophoneCaptureEngine: MicrophoneCaptureEngine, @unchecked Sendable {
  private let owner: DispatchQueue
  private let state = NSLock()
  private let api: any MicrophoneAudioUnitAPI
  private var context: MicrophoneAudioUnitContext?
  private var preparationFailure: Error?
  init(
    api: any MicrophoneAudioUnitAPI = NativeMicrophoneAudioUnitAPI(),
    owner: DispatchQueue = DispatchQueue(label: "app.mon.capture.microphone-unit")
  ) {
    self.api = api
    self.owner = owner
  }
  deinit {
    // Context owns a native retain until actual closure. A failed stop cannot
    // free RefCon or PCM storage while the operating system may still use it.
    if let context { owner.async { try? context.close() } }
  }
  var inputFormat: AVAudioFormat {
    state.withLock { context }?.currentFormat()
      ?? AVAudioFormat(standardFormatWithSampleRate: 48000, channels: 1)!
  }
  var isRunning: Bool { state.withLock { context }?.running() ?? false }
  func selectDevice(_ binding: MicrophoneRecovery.Binding) throws {
    try owner.sync {
      guard state.withLock({ context == nil }) else {
        throw ProbeError.capture("microphone AUHAL is already owned")
      }
      let unit = try api.create()
      let context = MicrophoneAudioUnitContext(api: api, unit: unit, binding: binding)
      state.withLock { self.context = context }
      try context.select()
    }
  }
  func verifyDevice(_ binding: MicrophoneRecovery.Binding) throws {
    guard let context = state.withLock({ context }) else {
      throw ProbeError.capture("microphone AUHAL is missing")
    }
    try context.verify(binding)
  }
  func verifyCallbackDevice(_ binding: MicrophoneRecovery.Binding) throws {
    guard let context = state.withLock({ context }) else {
      throw ProbeError.capture("microphone AUHAL is missing")
    }
    try context.verifyCallback(binding)
  }
  func observeConfiguration(_ handler: (@Sendable () -> Void)?) {
    state.withLock { context }?.observe(handler)
  }
  func installTap(
    format: AVAudioFormat,
    handler: @escaping @Sendable (AVAudioPCMBuffer, AVAudioTime) -> Void
  ) throws {
    try owner.sync {
      guard let context = state.withLock({ context }) else {
        throw ProbeError.capture("microphone AUHAL is missing")
      }
      try context.install(format: format, handler: handler)
    }
  }
  func removeTap() { state.withLock { context }?.removeTap() }
  func prepare() {
    owner.sync {
      do {
        guard let context = state.withLock({ context }) else {
          throw ProbeError.capture("microphone AUHAL is missing")
        }
        try context.prepare()
      } catch { preparationFailure = error }
    }
  }
  func start() throws {
    try owner.sync {
      if let preparationFailure { throw preparationFailure }
      guard let context = state.withLock({ context }) else {
        throw ProbeError.capture("microphone AUHAL is missing")
      }
      try context.start()
    }
  }
  func stop() throws {
    try owner.sync {
      guard let context = state.withLock({ context }) else { return }
      try context.close()
      state.withLock { self.context = nil }
    }
  }
}
