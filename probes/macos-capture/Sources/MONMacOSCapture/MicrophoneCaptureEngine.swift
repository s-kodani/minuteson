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

/// Production and silent test engines share the same selection, observation,
/// activation and teardown path. Tests never initialize a hardware input unit.
protocol MicrophoneCaptureEngine: AnyObject, Sendable {
  var inputFormat: AVAudioFormat { get }
  var isRunning: Bool { get }
  func selectDevice(_ binding: MicrophoneRecovery.Binding) throws
  func verifyDevice(_ binding: MicrophoneRecovery.Binding) throws
  func observeConfiguration(_ handler: (@Sendable () -> Void)?)
  func installTap(
    format: AVAudioFormat,
    handler: @escaping @Sendable (AVAudioPCMBuffer, AVAudioTime) -> Void)
  func removeTap()
  func prepare()
  func start() throws
  func stop()
}

final class NativeMicrophoneCaptureEngine: MicrophoneCaptureEngine, @unchecked Sendable {
  private let engine = AVAudioEngine()
  private var observer: NSObjectProtocol?

  var inputFormat: AVAudioFormat { engine.inputNode.outputFormat(forBus: 0) }
  var isRunning: Bool { engine.isRunning }

  func selectDevice(_ binding: MicrophoneRecovery.Binding) throws {
    guard let unit = engine.inputNode.audioUnit else {
      throw ProbeError.capture("microphone input unit is unavailable")
    }
    var device = binding.objectID
    let status = AudioUnitSetProperty(
      unit, kAudioOutputUnitProperty_CurrentDevice, kAudioUnitScope_Global, 0, &device,
      UInt32(MemoryLayout<AudioObjectID>.stride))
    guard status == noErr else {
      throw ProbeError.capture("could not select microphone: \(status)")
    }
    try verifyDevice(binding)
  }

  func verifyDevice(_ binding: MicrophoneRecovery.Binding) throws {
    guard let unit = engine.inputNode.audioUnit else {
      throw ProbeError.capture("microphone input unit is unavailable")
    }
    try InputDevices.verifyDevice(
      unit, expected: binding.objectID, expectedUID: binding.uid)
  }

  func observeConfiguration(_ handler: (@Sendable () -> Void)?) {
    if let observer { NotificationCenter.default.removeObserver(observer) }
    observer = nil
    if let handler {
      observer = NotificationCenter.default.addObserver(
        forName: .AVAudioEngineConfigurationChange, object: engine, queue: nil
      ) { _ in handler() }
    }
  }

  func installTap(
    format: AVAudioFormat,
    handler: @escaping @Sendable (AVAudioPCMBuffer, AVAudioTime) -> Void
  ) {
    engine.inputNode.installTap(onBus: 0, bufferSize: 1024, format: format, block: handler)
  }

  func removeTap() { engine.inputNode.removeTap(onBus: 0) }
  func prepare() { engine.prepare() }
  func start() throws { try engine.start() }
  func stop() { engine.stop() }
}
