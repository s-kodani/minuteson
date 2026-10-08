import AVFoundation
import CoreMedia
import Foundation

struct CaptureTestMetadata: Encodable {
  let ownerId: String
  let durationSeconds = 5
  let windowStartNs: Int64?
  let windowEndNs: Int64?
  let completed: Bool
}

struct CaptureTestSelection {
  let frames: Range<Int>
  let observation: CaptureClockObservation
}

/// A single-use window shared by both inputs. Receipt time and a frame budget
/// alone cannot exclude audio queued during permission/stream initialization.
/// Only proven capture timestamps decide which complete PCM frames are saved.
final class CaptureTestWindow: @unchecked Sendable {
  static let durationNs: Int64 = 5_000_000_000
  let ownerId: String
  private let lock = NSLock()
  private var start: Int64?
  private var end: Int64?
  private var closed = false
  private var completed = false

  init(ownerId: String) throws {
    guard InputLevelReport.validOwner(ownerId) else {
      throw ProbeError.usage("five-second test requires one canonical UUID v4")
    }
    self.ownerId = ownerId
  }

  var accepting: Bool { lock.withLock { start != nil && !closed } }

  func open(at nanoseconds: Int64?) throws {
    try lock.withLock {
      guard !closed, start == nil, let nanoseconds, nanoseconds >= 0,
        nanoseconds <= Int64.max - Self.durationNs
      else { throw ProbeError.capture("five-second test window cannot be opened") }
      start = nanoseconds
      end = nanoseconds + Self.durationNs
    }
  }

  func finish(at nanoseconds: Int64?) {
    lock.withLock {
      if closed { return }
      completed = end.map { deadline in nanoseconds.map { $0 >= deadline } ?? false } ?? false
      closed = true
    }
  }

  func metadata() -> CaptureTestMetadata {
    lock.withLock {
      CaptureTestMetadata(
        ownerId: ownerId, windowStartNs: start, windowEndNs: end, completed: completed)
    }
  }

  func selection(
    observation: CaptureClockObservation, frames: Int, sampleRate: Double, savedFrames: Int64
  ) throws -> CaptureTestSelection? {
    try lock.withLock {
      guard let start, let end, !closed else { return nil }
      guard case .timestamp(let capture) = observation else {
        if case .invalid(let message) = observation { throw ProbeError.capture(message) }
        throw ProbeError.capture("five-second test capture timestamp is missing")
      }
      guard capture >= 0, frames > 0, frames <= 1_000_000,
        sampleRate.isFinite, sampleRate.rounded() == sampleRate,
        (8000...384000).contains(sampleRate), savedFrames >= 0
      else { throw ProbeError.capture("five-second test PCM window is invalid") }
      let rate = Int64(sampleRate)
      guard savedFrames <= rate * 5 else {
        throw ProbeError.capture("five-second test exceeds its frame budget")
      }
      // Int128-sized intermediate arithmetic is not needed: clamp to one
      // bounded callback before multiplying, including arbitrarily old audio.
      if capture >= end { return nil }
      let callbackCeilingNs = (Int64(frames) * 1_000_000_000 + rate - 1) / rate
      if capture < start, start - capture >= callbackCeilingNs { return nil }
      let before = max(0, start - capture)
      let after = min(end - capture, callbackCeilingNs)
      let first = (before * rate + 999_999_999) / 1_000_000_000
      let limit = min(Int64(frames), after * rate / 1_000_000_000)
      let count = min(limit - first, rate * 5 - savedFrames)
      guard count > 0 else { return nil }
      let shifted = capture + first * 1_000_000_000 / rate
      return CaptureTestSelection(
        frames: Int(first)..<Int(first + count), observation: .timestamp(shifted))
    }
  }
}

/// Copy complete frames without changing the callback's input. This works for
/// both interleaved and planar PCM, unlike CoreMedia's range-copy API.
func testPCMFrames(_ source: AVAudioPCMBuffer, range: Range<Int>) throws -> AVAudioPCMBuffer {
  guard range.lowerBound >= 0, range.upperBound <= Int(source.frameLength), !range.isEmpty,
    source.frameLength <= 1_000_000, (1...8).contains(source.format.channelCount),
    let destination = AVAudioPCMBuffer(
      pcmFormat: source.format, frameCapacity: AVAudioFrameCount(range.count))
  else { throw ProbeError.capture("five-second test PCM slice is invalid") }
  destination.frameLength = destination.frameCapacity
  let input = UnsafeMutableAudioBufferListPointer(
    UnsafeMutablePointer(mutating: source.audioBufferList))
  let output = UnsafeMutableAudioBufferListPointer(destination.mutableAudioBufferList)
  let width: Int
  switch source.format.commonFormat {
  case .pcmFormatFloat32, .pcmFormatInt32: width = 4
  case .pcmFormatFloat64: width = 8
  case .pcmFormatInt16: width = 2
  default: throw ProbeError.capture("five-second test PCM format is unsupported")
  }
  guard input.count == output.count, !input.isEmpty else {
    throw ProbeError.capture("five-second test PCM layout is invalid")
  }
  for index in input.indices {
    let stride = width * Int(input[index].mNumberChannels)
    let offset = range.lowerBound * stride
    let length = range.count * stride
    guard stride > 0, input[index].mNumberChannels == output[index].mNumberChannels,
      offset + length <= Int(input[index].mDataByteSize),
      length <= Int(output[index].mDataByteSize),
      let from = input[index].mData, let to = output[index].mData
    else { throw ProbeError.capture("five-second test PCM storage is invalid") }
    memcpy(to, from.advanced(by: offset), length)
  }
  return destination
}

func testPCMFrames(_ source: CMSampleBuffer, range: Range<Int>) throws -> CMSampleBuffer {
  guard source.isValid, CMSampleBufferDataIsReady(source), source.numSamples > 0,
    source.numSamples <= 1_000_000, let description = source.formatDescription,
    let asbd = CMAudioFormatDescriptionGetStreamBasicDescription(description)?.pointee,
    asbd.mFormatID == kAudioFormatLinearPCM, asbd.mFramesPerPacket == 1,
    asbd.mSampleRate.isFinite, asbd.mSampleRate.rounded() == asbd.mSampleRate,
    (8000...384000).contains(asbd.mSampleRate), (1...8).contains(asbd.mChannelsPerFrame)
  else { throw ProbeError.capture("five-second test system PCM is unsupported") }
  let format = AVAudioFormat(cmAudioFormatDescription: description)
  guard
    let pcm = AVAudioPCMBuffer(
      pcmFormat: format, frameCapacity: AVAudioFrameCount(source.numSamples))
  else { throw ProbeError.capture("cannot allocate five-second test PCM") }
  pcm.frameLength = pcm.frameCapacity
  let copy = CMSampleBufferCopyPCMDataIntoAudioBufferList(
    source, at: 0, frameCount: Int32(source.numSamples), into: pcm.mutableAudioBufferList)
  guard copy == noErr else { throw ProbeError.capture("cannot copy five-second test PCM") }
  let sliced = try testPCMFrames(pcm, range: range)
  var timing = CMSampleTimingInfo(
    duration: CMTime(value: 1, timescale: Int32(asbd.mSampleRate)),
    presentationTimeStamp: source.presentationTimeStamp
      + CMTime(value: Int64(range.lowerBound), timescale: Int32(asbd.mSampleRate)),
    decodeTimeStamp: .invalid)
  var result: CMSampleBuffer?
  let create = CMSampleBufferCreate(
    allocator: kCFAllocatorDefault, dataBuffer: nil, dataReady: false,
    makeDataReadyCallback: nil, refcon: nil, formatDescription: description,
    sampleCount: range.count, sampleTimingEntryCount: 1, sampleTimingArray: &timing,
    sampleSizeEntryCount: 0, sampleSizeArray: nil, sampleBufferOut: &result)
  guard create == noErr, let result,
    CMSampleBufferSetDataBufferFromAudioBufferList(
      result, blockBufferAllocator: kCFAllocatorDefault,
      blockBufferMemoryAllocator: kCFAllocatorDefault,
      flags: 0, bufferList: sliced.audioBufferList) == noErr,
    CMSampleBufferSetDataReady(result) == noErr
  else { throw ProbeError.capture("cannot create five-second test PCM sample") }
  return result
}
