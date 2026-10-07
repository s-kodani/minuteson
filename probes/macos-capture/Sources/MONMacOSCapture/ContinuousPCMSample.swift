import AVFoundation
import CoreMedia

/// Store received PCM contiguously. Capture-time holes belong in clock maps,
/// not in writer-generated silence or raw file frame offsets.
func continuousPCMSample(_ sample: CMSampleBuffer, frameOffset: Int64, sampleRate: Double) throws
  -> CMSampleBuffer
{
  guard sampleRate.isFinite, sampleRate.rounded() == sampleRate,
    (8000...384000).contains(sampleRate), frameOffset >= 0, sample.numSamples > 0,
    let description = sample.formatDescription,
    let format = CMAudioFormatDescriptionGetStreamBasicDescription(description)?.pointee,
    format.mFormatID == kAudioFormatLinearPCM, format.mFramesPerPacket == 1,
    format.mSampleRate == sampleRate, (1...8).contains(format.mChannelsPerFrame)
  else { throw ProbeError.capture("system audio is not supported fixed-rate PCM") }
  var timing = CMSampleTimingInfo(
    duration: CMTime(value: 1, timescale: Int32(sampleRate)),
    presentationTimeStamp: CMTime(value: frameOffset, timescale: Int32(sampleRate)),
    decodeTimeStamp: .invalid)
  var result: CMSampleBuffer?
  let status = CMSampleBufferCreateCopyWithNewTiming(
    allocator: kCFAllocatorDefault, sampleBuffer: sample, sampleTimingEntryCount: 1,
    sampleTimingArray: &timing, sampleBufferOut: &result)
  guard status == noErr, let result else {
    throw ProbeError.capture("cannot preserve PCM with contiguous file timestamps: \(status)")
  }
  return result
}
