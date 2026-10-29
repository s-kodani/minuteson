import AVFoundation
import CoreMedia
import Darwin
import Foundation

// Telemetry is deliberately excluded from the saved recording manifest.
// A received PCM window is not evidence of durable CAF storage.
struct InputLevelTrack: Encodable {
  let state: String
  let windowFrames: Int?
  let ageMs: Int64?
  let rms: Double?
  let peak: Double?

  enum CodingKeys: String, CodingKey { case state, windowFrames, ageMs, rms, peak }
  func encode(to encoder: Encoder) throws {
    var c = encoder.container(keyedBy: CodingKeys.self)
    try c.encode(state, forKey: .state)
    try c.encode(windowFrames, forKey: .windowFrames)
    try c.encode(ageMs, forKey: .ageMs)
    try c.encode(rms, forKey: .rms)
    try c.encode(peak, forKey: .peak)
  }
  static func unknown(_ state: String) -> Self {
    Self(state: state, windowFrames: nil, ageMs: nil, rms: nil, peak: nil)
  }
}

struct PCMInputLevel {
  private var sampledAtNs: Int64?
  private var frames: Int?
  private var rms: Double?
  private var peak: Double?
  private var invalid = false

  static func nowNanoseconds() -> Int64? {
    var info = mach_timebase_info_data_t()
    guard mach_timebase_info(&info) == KERN_SUCCESS else { return nil }
    return try? MachTimeUnits(numerator: info.numer, denominator: info.denom)
      .nanoseconds(mach_continuous_time())
  }

  mutating func receive(_ buffer: AVAudioPCMBuffer, at nanoseconds: Int64?) {
    let count = Int(buffer.frameLength)
    let channels = Int(buffer.format.channelCount)
    guard let nanoseconds, nanoseconds >= 0, count > 0, count <= 1_000_000,
      channels > 0, channels <= 8,
      sampledAtNs.map({ nanoseconds >= $0 }) ?? true
    else {
      invalidate()
      return
    }
    var sum = 0.0
    var maximum = 0.0
    let interleaved = buffer.format.isInterleaved
    let stride = interleaved ? channels : 1
    func accumulate<T: BinaryFloatingPoint>(_ data: UnsafePointer<UnsafeMutablePointer<T>>) -> Bool
    {
      for channel in 0..<channels {
        let values = data[interleaved ? 0 : channel]
        for frame in 0..<count {
          let value = Double(values[frame * stride + (interleaved ? channel : 0)])
          guard value.isFinite, abs(value) <= 16 else { return false }
          sum += value * value
          maximum = max(maximum, abs(value))
        }
      }
      return true
    }
    func integers<T: FixedWidthInteger>(
      _ data: UnsafePointer<UnsafeMutablePointer<T>>, fullScale: Double
    ) {
      for channel in 0..<channels {
        let values = data[interleaved ? 0 : channel]
        for frame in 0..<count {
          let value = Double(values[frame * stride + (interleaved ? channel : 0)]) / fullScale
          sum += value * value
          maximum = max(maximum, abs(value))
        }
      }
    }
    switch buffer.format.commonFormat {
    case .pcmFormatFloat32:
      guard let data = buffer.floatChannelData, accumulate(data) else {
        invalidate()
        return
      }
    case .pcmFormatInt16:
      guard let data = buffer.int16ChannelData else {
        invalidate()
        return
      }
      integers(data, fullScale: 32768)
    case .pcmFormatInt32:
      guard let data = buffer.int32ChannelData else {
        invalidate()
        return
      }
      integers(data, fullScale: 2_147_483_648)
    default:
      invalidate()
      return
    }
    sampledAtNs = nanoseconds
    frames = count
    peak = maximum
    // Include every channel independently. Anti-phase stereo must not cancel.
    rms = min(maximum, sqrt(sum / Double(count * channels)))
    invalid = false
  }

  mutating func receive(_ sample: CMSampleBuffer, at nanoseconds: Int64?) {
    guard sample.isValid, sample.numSamples > 0, sample.numSamples <= 1_000_000,
      let description = sample.formatDescription,
      let asbd = CMAudioFormatDescriptionGetStreamBasicDescription(description)?.pointee,
      asbd.mFormatID == kAudioFormatLinearPCM, asbd.mSampleRate.isFinite,
      (8000...384000).contains(asbd.mSampleRate), (1...8).contains(asbd.mChannelsPerFrame)
    else {
      invalidate()
      return
    }
    let format = AVAudioFormat(cmAudioFormatDescription: description)
    guard
      format.channelCount > 0, format.channelCount <= 8,
      let buffer = AVAudioPCMBuffer(
        pcmFormat: format, frameCapacity: AVAudioFrameCount(sample.numSamples))
    else {
      invalidate()
      return
    }
    buffer.frameLength = AVAudioFrameCount(sample.numSamples)
    guard
      CMSampleBufferCopyPCMDataIntoAudioBufferList(
        sample, at: 0, frameCount: Int32(sample.numSamples),
        into: buffer.mutableAudioBufferList) == noErr
    else {
      invalidate()
      return
    }
    receive(buffer, at: nanoseconds)
  }

  mutating func invalidate() { invalid = true }
  func snapshot(at now: Int64?) -> InputLevelTrack {
    if invalid { return .unknown("unavailable") }
    guard let sampledAtNs else { return .unknown("waiting") }
    guard let now, now >= sampledAtNs else { return .unknown("unavailable") }
    let elapsed = now - sampledAtNs
    return InputLevelTrack(
      state: "measured", windowFrames: frames,
      ageMs: elapsed / 1_000_000 + (elapsed % 1_000_000 == 0 ? 0 : 1),
      rms: rms, peak: peak)
  }
}

struct InputLevelReport: Encodable {
  let schemaVersion = 1
  let ownerId: String
  let revision: Int64
  let microphone: InputLevelTrack
  let system: InputLevelTrack

  static func validOwner(_ value: String) -> Bool {
    CaptureCore.validOwner(value)
  }

  func publish(to output: URL) throws {
    guard Self.validOwner(ownerId), revision > 0, revision <= 9_007_199_254_740_991 else {
      throw ProbeError.capture("input level report owner or revision is invalid")
    }
    let data = try JSONEncoder().encode(self)
    guard data.count <= 16 * 1024 else {
      throw ProbeError.capture("input level report exceeds limit")
    }
    try data.write(to: output.appendingPathComponent("input-levels.json"), options: .atomic)
  }
}
