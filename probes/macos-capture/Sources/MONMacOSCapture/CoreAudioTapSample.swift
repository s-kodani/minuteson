import AVFoundation
import CoreMedia
import Foundation

/// Copy borrowed HAL input before its IO callback returns. The resulting
/// sample owns its bytes and explicitly uses hardware host time, not now().
enum CoreAudioTapSample {
  static let maximumFrames = 32_768
  static let maximumBytes = 1_048_576

  static func format(_ asbd: AudioStreamBasicDescription) throws -> AVAudioFormat {
    let planar = asbd.mFormatFlags & kAudioFormatFlagIsNonInterleaved != 0
    guard (8_000...384_000).contains(asbd.mSampleRate),
      asbd.mSampleRate.rounded() == asbd.mSampleRate,
      (1...32).contains(asbd.mChannelsPerFrame), asbd.mFormatID == kAudioFormatLinearPCM,
      asbd.mBitsPerChannel == 32, asbd.mFramesPerPacket == 1,
      asbd.mFormatFlags & kAudioFormatFlagIsFloat != 0,
      asbd.mFormatFlags & kAudioFormatFlagIsPacked != 0,
      asbd.mFormatFlags & kAudioFormatFlagIsBigEndian == 0,
      asbd.mBytesPerFrame == 4 * (planar ? 1 : asbd.mChannelsPerFrame),
      asbd.mBytesPerPacket == asbd.mBytesPerFrame,
      // Never return the temporary pointer from this closure. Release builds
      // may reuse its storage before AVAudioFormat reads the description.
      let format = withUnsafePointer(to: asbd, { AVAudioFormat(streamDescription: $0) }),
      format.commonFormat == .pcmFormatFloat32
    else {
      throw ProbeError.capture(
        "PC音声の形式を読み込めません。権限拒否とは断定できません。詳細: tap Float32 PCM format invalid; rate=\(asbd.mSampleRate), channels=\(asbd.mChannelsPerFrame), format=\(asbd.mFormatID), flags=\(asbd.mFormatFlags), bits=\(asbd.mBitsPerChannel), frameBytes=\(asbd.mBytesPerFrame), packetBytes=\(asbd.mBytesPerPacket), packetFrames=\(asbd.mFramesPerPacket)"
      )
    }
    return format
  }

  static func copy(
    _ input: UnsafePointer<AudioBufferList>, time: AudioTimeStamp,
    format: AVAudioFormat
  ) throws -> CMSampleBuffer {
    let buffers = UnsafeMutableAudioBufferListPointer(UnsafeMutablePointer(mutating: input))
    let asbd = format.streamDescription.pointee
    guard format.commonFormat == .pcmFormatFloat32,
      (8_000...384_000).contains(asbd.mSampleRate), asbd.mSampleRate.rounded() == asbd.mSampleRate,
      (1...32).contains(asbd.mChannelsPerFrame), asbd.mBytesPerFrame > 0,
      asbd.mFramesPerPacket == 1, asbd.mBitsPerChannel == 32,
      asbd.mFormatID == kAudioFormatLinearPCM,
      asbd.mFormatFlags & kAudioFormatFlagIsFloat != 0,
      asbd.mFormatFlags & kAudioFormatFlagIsPacked != 0,
      asbd.mFormatFlags & kAudioFormatFlagIsBigEndian == 0,
      asbd.mBytesPerFrame == 4 * (format.isInterleaved ? format.channelCount : 1),
      asbd.mBytesPerPacket == asbd.mBytesPerFrame,
      !buffers.isEmpty, buffers.count == (format.isInterleaved ? 1 : Int(format.channelCount)),
      time.mFlags.contains(.hostTimeValid), time.mHostTime > 0,
      time.mFlags.contains(.sampleTimeValid), time.mSampleTime >= 0,
      Int64(exactly: time.mSampleTime) != nil
    else {
      throw ProbeError.capture(
        "PC音声のPCM形式または取得時刻を検証できません。詳細: tap PCM format or hardware timestamp is invalid; rate=\(asbd.mSampleRate), channels=\(asbd.mChannelsPerFrame), format=\(asbd.mFormatID), flags=\(asbd.mFormatFlags), frameBytes=\(asbd.mBytesPerFrame), planes=\(buffers.count), timeFlags=\(time.mFlags.rawValue), hostTime=\(time.mHostTime), sampleTime=\(time.mSampleTime)"
      )
    }
    let frameBytes = Int(asbd.mBytesPerFrame)
    let bytes = Int(buffers[0].mDataByteSize)
    guard bytes > 0, bytes % frameBytes == 0,
      bytes / frameBytes <= maximumFrames,
      bytes <= maximumBytes / buffers.count
    else { throw ProbeError.capture("tap PCM exceeds its bounded frame or byte capacity") }
    let frames = bytes / frameBytes
    for buffer in buffers {
      guard buffer.mData != nil, Int(buffer.mDataByteSize) == bytes,
        buffer.mNumberChannels == (format.isInterleaved ? format.channelCount : 1)
      else {
        throw ProbeError.capture(
          "PC音声の入力バッファと音声形式が一致しません。詳細: tap PCM buffer planes do not match their format; interleaved=\(format.isInterleaved), channels=\(format.channelCount), planes=\(buffers.count), planeChannels=\(buffer.mNumberChannels), planeBytes=\(buffer.mDataByteSize), expectedBytes=\(bytes), hasData=\(buffer.mData != nil)"
        )
      }
    }
    var block: CMBlockBuffer?
    let total = bytes * buffers.count
    try require(
      CMBlockBufferCreateWithMemoryBlock(
        allocator: kCFAllocatorDefault, memoryBlock: nil, blockLength: total,
        blockAllocator: kCFAllocatorDefault, customBlockSource: nil, offsetToData: 0,
        dataLength: total, flags: 0, blockBufferOut: &block))
    guard let block else { throw ProbeError.capture("tap PCM owned storage is missing") }
    for (index, buffer) in buffers.enumerated() {
      try require(
        CMBlockBufferReplaceDataBytes(
          with: buffer.mData!, blockBuffer: block,
          offsetIntoDestination: index * bytes, dataLength: bytes))
    }
    var sample: CMSampleBuffer?
    try require(
      CMAudioSampleBufferCreateReadyWithPacketDescriptions(
        allocator: kCFAllocatorDefault, dataBuffer: block,
        formatDescription: format.formatDescription, sampleCount: frames,
        presentationTimeStamp: CMClockMakeHostTimeFromSystemUnits(time.mHostTime),
        packetDescriptions: nil, sampleBufferOut: &sample))
    guard let sample, sample.isValid, sample.numSamples == frames else {
      throw ProbeError.capture("tap PCM sample could not be validated")
    }
    return sample
  }

  private static func require(_ status: OSStatus) throws {
    guard status == noErr else { throw ProbeError.capture("tap PCM conversion failed: \(status)") }
  }
}
