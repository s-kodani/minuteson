import AVFoundation
import Foundation
import MONCaptureCore

/// One serial writer owns a converter. Its C callback never escapes a call.
final class VoicePCM {
  let rate: UInt32
  let channels: UInt32
  private let value: OpaquePointer
  private final class Sink {
    let write: ([Int16], Int64, Int64) throws -> Void
    var error: Error?
    init(_ write: @escaping ([Int16], Int64, Int64) throws -> Void) { self.write = write }
  }
  init(rate: UInt32, channels: UInt32) throws {
    guard let value = mon_voice_new(rate, channels) else {
      throw ProbeError.capture("voice PCM converter could not be created")
    }
    self.rate = rate
    self.channels = channels
    self.value = value
  }
  deinit { mon_voice_free(value) }
  private func perform(
    _ write: @escaping ([Int16], Int64, Int64) throws -> Void,
    _ operation: (UnsafeMutableRawPointer, MonVoiceWrite) -> Bool
  ) throws {
    let sink = Sink(write)
    let success = operation(Unmanaged.passUnretained(sink).toOpaque()) {
      context, samples, count, start, end in
      guard let context, let samples else { return false }
      let sink = Unmanaged<Sink>.fromOpaque(context).takeUnretainedValue()
      do {
        try sink.write(Array(UnsafeBufferPointer(start: samples, count: count)), start, end)
        return true
      } catch {
        sink.error = error
        return false
      }
    }
    if let error = sink.error { throw error }
    guard success else {
      throw ProbeError.capture("voice PCM conversion or timestamp mapping failed")
    }
  }
  func push(
    _ samples: [Float], start: Int64, end: Int64,
    write: @escaping ([Int16], Int64, Int64) throws -> Void
  ) throws {
    try perform(write) { context, callback in
      samples.withUnsafeBufferPointer {
        mon_voice_push(value, $0.baseAddress, $0.count, start, end, context, callback)
      }
    }
  }
  func finish(write: @escaping ([Int16], Int64, Int64) throws -> Void) throws {
    try perform(write) { mon_voice_finish(value, $0, $1) }
  }
  static func header(frames: Int64) throws -> Data {
    guard frames >= 0 else { throw ProbeError.capture("invalid compact CAF length") }
    var bytes = [UInt8](repeating: 0, count: 68)
    let count = bytes.withUnsafeMutableBufferPointer {
      mon_voice_caf_header(UInt64(frames), $0.baseAddress, $0.count)
    }
    guard count == 68 else { throw ProbeError.capture("invalid compact CAF geometry") }
    return Data(bytes)
  }
  static func interleaved(_ buffer: AVAudioPCMBuffer) throws -> [Float] {
    guard buffer.format.commonFormat == .pcmFormatFloat32, let data = buffer.floatChannelData else {
      throw ProbeError.capture("voice PCM input must be Float32")
    }
    let channels = Int(buffer.format.channelCount)
    let interleaved = buffer.format.isInterleaved
    var samples = [Float]()
    samples.reserveCapacity(Int(buffer.frameLength) * channels)
    for frame in 0..<Int(buffer.frameLength) {
      for channel in 0..<channels {
        let value = data[interleaved ? 0 : channel][
          interleaved ? frame * channels + channel : frame]
        guard value.isFinite else { throw ProbeError.capture("non-finite voice PCM input") }
        samples.append(value)
      }
    }
    return samples
  }
}
