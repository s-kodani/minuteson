import AVFoundation
import CoreMedia
import Foundation

/// One writer survives all source epochs;
/// its timestamps describe physical file frames, not the capture timeline.
final class SystemPCMWriter: @unchecked Sendable {
  enum AppendError: Error { case formatChanged }
  private let outputURL: URL
  private let lock = NSLock()
  private var writer: AVAssetWriter?
  private var input: AVAssetWriterInput?
  private var sourceFormat: AVAudioFormat?
  private var closed = false
  private var finishFailure: String?
  private var savedStats = TrackStats()
  private let rawSegments: RawRecordingSegments?
  var stats: TrackStats {
    lock.withLock {
      var result = savedStats
      result.rawSegments = rawSegments?.snapshot()
      return result
    }
  }

  init(outputURL: URL, rawSegments: RawRecordingSegments? = nil) {
    self.outputURL = outputURL
    self.rawSegments = rawSegments
  }
  var ready: Bool { lock.withLock { input?.isReadyForMoreMediaData ?? true } }

  func invalidateLevels() { lock.withLock { savedStats.inputLevel.invalidate() } }
  func recordFailure() { lock.withLock { savedStats.appendFailures += 1 } }

  func append(
    _ sample: CMSampleBuffer, observation: CaptureClockObservation?, sourceEpoch: UInt64,
    levelsEnabled: Bool, receivedAt: Int64?
  ) throws {
    lock.lock()
    defer { lock.unlock() }
    guard !closed, sample.isValid, sample.numSamples > 0,
      let description = sample.formatDescription,
      let asbd = CMAudioFormatDescriptionGetStreamBasicDescription(description)?.pointee
    else { throw ProbeError.capture("system audio format or writer is unavailable") }
    let format = AVAudioFormat(cmAudioFormatDescription: description)
    if let sourceFormat, !sourceFormat.isEqual(format) { throw AppendError.formatChanged }
    let contiguous = try continuousPCMSample(
      sample, frameOffset: savedStats.frames, sampleRate: asbd.mSampleRate)
    if writer == nil {
      let settings: [String: Any] = [
        AVFormatIDKey: kAudioFormatLinearPCM, AVSampleRateKey: asbd.mSampleRate,
        AVNumberOfChannelsKey: asbd.mChannelsPerFrame,
        AVLinearPCMBitDepthKey: 32, AVLinearPCMIsFloatKey: true,
        AVLinearPCMIsNonInterleaved: false, AVLinearPCMIsBigEndianKey: false,
      ]
      let writer = try AVAssetWriter(outputURL: outputURL, fileType: .caf)
      guard writer.canApply(outputSettings: settings, forMediaType: .audio) else {
        throw ProbeError.capture("cannot write system audio format to CAF")
      }
      let input = AVAssetWriterInput(mediaType: .audio, outputSettings: settings)
      input.expectsMediaDataInRealTime = true
      guard writer.canAdd(input) else { throw ProbeError.capture("cannot add system audio input") }
      writer.add(input)
      guard writer.startWriting() else {
        throw ProbeError.capture("cannot start system CAF writer")
      }
      writer.startSession(atSourceTime: .zero)
      self.writer = writer
      self.input = input
      sourceFormat = format
      savedStats.sampleRate = asbd.mSampleRate
      savedStats.channels = asbd.mChannelsPerFrame
      savedStats.firstPresentationTimeSeconds = sample.presentationTimeStamp.seconds
    }
    savedStats.callbackCount += 1
    guard let input, input.isReadyForMoreMediaData, input.append(contiguous) else {
      throw ProbeError.capture("system audio writer is not ready or append failed")
    }
    let frames = Int64(sample.numSamples)
    savedStats.frames += frames
    savedStats.lastPresentationTimeSeconds = sample.presentationTimeStamp.seconds
    if levelsEnabled { savedStats.inputLevel.receive(sample, at: receivedAt) }
    if let observation {
      savedStats.recordSavedClock(
        observation, frames: frames, sourceFrame: nil, sourceEpoch: sourceEpoch)
    }
    try rawSegments?.enqueue(sample, observation: observation, sourceEpoch: sourceEpoch)
  }

  func finish() async throws {
    let pending = try lock.withLock { () -> AVAssetWriter? in
      if closed {
        if let finishFailure { throw ProbeError.capture(finishFailure) }
        return nil
      }
      closed = true
      guard let writer, let input, let rate = savedStats.sampleRate else {
        finishFailure = "no system audio samples were received"
        throw ProbeError.capture(finishFailure!)
      }
      self.writer = nil
      self.input = nil
      // Explicit end time also preserves a partial final PCM packet.
      writer.endSession(atSourceTime: CMTime(value: savedStats.frames, timescale: Int32(rate)))
      input.markAsFinished()
      return writer
    }
    guard let writer = pending else { return }
    await writer.finishWriting()
    guard writer.status == .completed else {
      let failure =
        "system CAF finalization failed: \(writer.error?.localizedDescription ?? String(describing: writer.status))"
      lock.withLock { finishFailure = failure }
      throw ProbeError.capture(failure)
    }
    do { try rawSegments?.finish() } catch {
      lock.withLock {
        finishFailure = "raw system audio finalization failed"
        savedStats.appendFailures += 1
      }
      throw error
    }
  }
}
