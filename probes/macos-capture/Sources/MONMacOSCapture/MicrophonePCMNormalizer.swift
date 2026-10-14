import AVFoundation
import Foundation

private final class OneMicrophoneBuffer: @unchecked Sendable {
  private let lock = NSLock()
  private let input: AVAudioPCMBuffer
  private var provided = false
  init(_ input: AVAudioPCMBuffer) { self.input = input }
  var wasProvided: Bool { lock.withLock { provided } }
  func next(_ status: UnsafeMutablePointer<AVAudioConverterInputStatus>) -> AVAudioBuffer? {
    lock.withLock {
      guard !provided else {
        status.pointee = .noDataNow
        return nil
      }
      provided = true
      status.pointee = .haveData
      return input
    }
  }
}

/// A serial, bounded converter used only after an explicitly retired source
/// binding switches to another physical input. The caller owns its lock and
/// supplies capture timestamps from the original HAL callback.
final class MicrophonePCMNormalizer {
  let inputFormat: AVAudioFormat
  let outputFormat: AVAudioFormat
  private let converter: AVAudioConverter

  init?(from inputFormat: AVAudioFormat, to outputFormat: AVAudioFormat) {
    guard inputFormat.sampleRate.isFinite, outputFormat.sampleRate.isFinite,
      (8000...384000).contains(inputFormat.sampleRate),
      (8000...384000).contains(outputFormat.sampleRate),
      inputFormat.commonFormat == .pcmFormatFloat32,
      outputFormat.commonFormat == .pcmFormatFloat32,
      !inputFormat.isInterleaved, !outputFormat.isInterleaved,
      (1...8).contains(inputFormat.channelCount),
      (1...8).contains(outputFormat.channelCount),
      let converter = AVAudioConverter(from: inputFormat, to: outputFormat)
    else { return nil }
    self.inputFormat = inputFormat
    self.outputFormat = outputFormat
    self.converter = converter
    converter.primeMethod = .none
  }

  func convert(_ input: AVAudioPCMBuffer) throws -> AVAudioPCMBuffer {
    guard MicrophonePCMFormat.matches(inputFormat, input.format),
      input.frameLength > 0, input.frameLength <= 96_000
    else { throw ProbeError.capture("changed microphone PCM does not match its binding") }
    let estimate = ceil(
      Double(input.frameLength) * outputFormat.sampleRate / inputFormat.sampleRate)
    guard estimate.isFinite, estimate > 0,
      estimate + 512 <= Double(UInt32.max),
      estimate + 512 <= outputFormat.sampleRate / 4
    else { throw ProbeError.capture("microphone conversion buffer exceeds its bound") }
    guard
      let output = AVAudioPCMBuffer(
        pcmFormat: outputFormat, frameCapacity: AVAudioFrameCount(estimate) + 512)
    else { throw ProbeError.capture("microphone conversion buffer allocation failed") }
    let provider = OneMicrophoneBuffer(input)
    var error: NSError?
    let result = converter.convert(to: output, error: &error) { _, status in
      provider.next(status)
    }
    guard error == nil, provider.wasProvided, result != .error,
      output.frameLength > 0, output.frameLength <= output.frameCapacity
    else {
      throw ProbeError.capture("microphone PCM conversion produced no verified frames")
    }
    return output
  }
}
