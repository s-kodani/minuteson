import AVFoundation
import CoreMedia
import Foundation
import MONCaptureCore

/// Only rejected source data is recoverable. Writer, allocator and converter
/// errors retain their original fatal path, including errors from sink callbacks.
enum CaptureInputFault: String, Error, Sendable {
  case invalidPCM, invalidFormat, invalidClock, sourceCounter

  static func requireFormat(_ format: AVAudioFormat, frames: Int) throws {
    guard format.commonFormat == .pcmFormatFloat32, format.sampleRate.isFinite,
      format.sampleRate.rounded() == format.sampleRate,
      (8000...384000).contains(format.sampleRate),
      frames > 0,
      mon_capture_pcm_geometry(UInt32(format.sampleRate), format.channelCount, UInt64(frames)),
      frames <= Int(format.sampleRate) / 4
    else { throw Self.invalidFormat }
  }
  static func copy(_ sample: CMSampleBuffer) throws -> AVAudioPCMBuffer {
    guard sample.isValid, let description = sample.formatDescription else {
      throw Self.invalidFormat
    }
    let format = AVAudioFormat(cmAudioFormatDescription: description)
    try requireFormat(format, frames: sample.numSamples)
    guard
      let buffer = AVAudioPCMBuffer(
        pcmFormat: format, frameCapacity: AVAudioFrameCount(sample.numSamples))
    else {
      throw ProbeError.capture("capture PCM copy allocation failed")
    }
    buffer.frameLength = AVAudioFrameCount(sample.numSamples)
    guard
      CMSampleBufferCopyPCMDataIntoAudioBufferList(
        sample, at: 0, frameCount: Int32(sample.numSamples), into: buffer.mutableAudioBufferList)
        == noErr
    else {
      throw Self.invalidPCM
    }
    return buffer
  }
  static func validate(_ buffer: AVAudioPCMBuffer) throws {
    let format = buffer.format
    try requireFormat(format, frames: Int(buffer.frameLength))
    let buffers = UnsafeMutableAudioBufferListPointer(
      UnsafeMutablePointer(mutating: buffer.audioBufferList))
    let channels = Int(format.channelCount)
    guard buffers.count == (format.isInterleaved ? 1 : channels) else { throw Self.invalidFormat }
    for part in buffers {
      let count = Int(buffer.frameLength) * (format.isInterleaved ? channels : 1)
      guard part.mNumberChannels == (format.isInterleaved ? format.channelCount : 1),
        Int(part.mDataByteSize) == count * MemoryLayout<Float>.size,
        let data = part.mData
      else { throw Self.invalidPCM }
      guard mon_capture_pcm_samples(data.assumingMemoryBound(to: Float.self), count) else {
        throw Self.invalidPCM
      }
    }
  }
}

struct CaptureInputRetry {
  private static let diagnostics = DispatchQueue(label: "app.mon.capture.input-fault-log")
  private var value = MonCaptureInputRetry()
  private var reported = 0
  var recovering: Bool { value.failures > 0 }
  func permits(_ now: Int64) -> Bool { now >= 0 && UInt64(now) >= value.retry_at }
  mutating func fault(_ fault: CaptureInputFault, at now: Int64, input: String) {
    mon_capture_input_fault(&value, UInt64(max(0, now)))
    // Repeated bad callbacks never create unbounded log traffic. State/history
    // still reports every real interruption and recovery through its bounded journal.
    if reported < 16 {
      let attempt = value.failures
      Self.diagnostics.async {
        FileHandle.standardError.write(
          Data(
            "mon_capture_input_fault: input=\(input) kind=\(fault.rawValue) atNs=\(now) attempt=\(attempt)\n"
              .utf8))
      }
      reported += 1
    }
  }
  mutating func failedAttempt(at now: Int64) {
    if recovering { mon_capture_input_fault(&value, UInt64(max(0, now))) }
  }
  mutating func valid(at now: Int64) {
    mon_capture_input_valid(&value, UInt64(max(0, now)))
  }
}
