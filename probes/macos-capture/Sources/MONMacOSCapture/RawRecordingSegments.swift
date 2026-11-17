import AVFoundation
import CoreMedia
import CryptoKit
import Darwin
import Foundation

struct RawSegmentStatus: Encodable {
  let schemaVersion = 2
  let ownerId: String
  let lane: String
  let sealedSegments: Int
  let sealedFrames: Int64
  let lastEntrySHA256: String?
  let producerClosed: Bool
  let failure: String?
}

/// Immutable, independently readable raw CAFs plus an fsynced prefix pointer.
/// Audio callbacks only enqueue bounded copies. A segment is advertised after
/// its PCM, measured clock boundary and hash have all been durably stored.
/// This is raw recovery evidence, NOT a successful acquisition closure or the
/// final FLAC manifest. Never infer a missing tail or insert silence from it.
final class RawRecordingSegments: @unchecked Sendable {
  // Tests may inject storage faults; native defaults always write and fsync.
  // No command-line or application setting can disable durability.
  struct StorageIO: Sendable {
    var write: @Sendable (Data, Int32) throws -> Void = {
      try RawRecordingSegments.write($0, to: $1)
    }
    var synchronize: @Sendable (Int32) throws -> Void = {
      guard fsync($0) == 0 else { throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO) }
    }
  }
  static let maximumSegments = CaptureCore.maximumSegments
  // Bound both affine maps and nanosecond witnesses within the 65KiB entry.
  static let maximumClockSegments = 128
  static let maximumPendingBytes = CaptureCore.maximumPendingBytes
  static let maximumPendingNanoseconds = CaptureCore.maximumPendingNanoseconds
  private struct Payload: @unchecked Sendable {
    let buffer: AVAudioPCMBuffer
    let observation: CaptureClockObservation
    let sourceFrame: Int64?
    let sourceEpoch: UInt64
    let bytes: Int
    let durationNanoseconds: Int64
  }
  private struct Entry: Encodable {
    let schemaVersion = 3
    let sampleEncoding = "pcm16"
    let ownerId: String
    let lane: String
    let sequence: Int
    let audioName: String
    let sourceFrameStart: Int64
    let frames: Int64
    let sampleRate: Int64
    let channels: Int
    let captureOriginNanoseconds: Int64
    let clockSegments: [CapturedClockSegment]
    let clockNanoseconds: [CapturedClockEndpoints]
    let audioBytes: Int64
    let audioSHA256: String
    let previousEntrySHA256: String?
  }
  private final class Active {
    let origin: Int64
    var pcm: Data
    var frames: Int64 = 0
    var acquiredNanoseconds: Int64 = 0
    var timing: CaptureTrackTiming
    init(origin: Int64, timing: CaptureTrackTiming) throws {
      self.origin = origin
      self.timing = timing
      pcm = try VoicePCM.header(frames: 0)
      pcm.reserveCapacity(128_068)
    }
  }
  private struct SegmentJob: @unchecked Sendable {
    let sequence: Int
    let frameStart: Int64
    let frames: Int64
    let pcm: Data
    let acquiredNanoseconds: Int64
    let priorHash: String?
    let audioSHA256: String
    let entry: Data
    let nextPrefix: CaptureRawPrefix
    let tip: Data
  }
  private struct SourceCredit {
    // A native packet remains charged until the durable segment containing
    // its last normalized frame commits. This includes the converter's halo
    // and avoids rounding an upsampled packet into an early budget release.
    let frameEnd: Int64
    let nanoseconds: Int64
  }

  private let owner: String
  private let lane: String
  private let directory: Int32
  private let pointer: InputEventJournalFile
  private let beforeAppend: @Sendable () -> Void
  private let storageIO: StorageIO
  private let maximumTotalClockSegments: Int
  private let commits: RawSegmentCommitWorker
  private let queue = DispatchQueue(label: "app.mon.capture.raw-segments")
  private let lock = NSLock()
  private let finishLock = NSLock()
  private var pendingBytes = 0
  private var pendingNanoseconds: Int64 = 0
  private var pendingPackets = 0
  private var accepting = true
  // Admission failure retires new callbacks but does not invalidate PCM that
  // was already accepted. Storage/normalization poison remains separate.
  private var admissionFailure: String?
  private var failure: String?
  // A later normalization error cannot revoke already-prevalidated jobs.
  // Only an actual durable-commit failure poisons that ordered job stream.
  private var commitPoison = false
  private var closed = false
  private var prefix = CaptureRawPrefix()
  private var sealedCount: Int { prefix.segments }
  private var sealedFrames: Int64 { prefix.frames }
  private var previousHash: String? { prefix.hash }
  // Only accessed by the normalization queue, including its finish barrier.
  private var active: Active?
  private var converter: VoicePCM?
  private var converterEpoch: UInt64?
  private var sourceEnd: Int64?
  private var sourceCaptureEnd: Int64?
  private var compactCaptureEnd: Int64?
  private let allowsClockRebasing: Bool
  private var alignment = CapturePCMAlignment()
  private var plannedPrefix = CaptureRawPrefix()
  private var converterInputFrames: Int64 = 0
  private var converterOutputBase: Int64 = 0
  private var sourceCredits: [SourceCredit] = []
  private var compactStats = TrackStats()
  private var outputEpoch: UInt64 = 0

  /// Snapshot contains normalized 16kHz frames, never native acquisition frames.
  /// Only rawSegments.sealedFrames describes the durable prefix while recording.
  func applyingSavedAudio(to original: TrackStats) -> TrackStats {
    lock.withLock {
      var result = original
      result.frames = compactStats.frames
      result.sampleRate = compactStats.sampleRate
      result.channels = compactStats.channels
      result.timing = compactStats.timing
      result.clockSegments = nil
      result.clockAlignment = compactStats.clockAlignment
      result.rawSegments = RawSegmentStatus(
        ownerId: owner, lane: lane, sealedSegments: sealedCount,
        sealedFrames: sealedFrames, lastEntrySHA256: previousHash, producerClosed: closed,
        failure: failure ?? admissionFailure)
      return result
    }
  }

  init(
    output: URL, owner: String, lane: String,
    beforeAppend: @escaping @Sendable () -> Void = {}, storageIO: StorageIO = StorageIO(),
    tipStorageIO: InputEventJournalFile.StorageIO = InputEventJournalFile.StorageIO(),
    maximumTotalClockSegments: Int = CapturedClockMap.maximumTrackSegments,
    allowsClockRebasing: Bool = false,
    commits: RawSegmentCommitWorker = RawSegmentCommitWorker()
  ) throws {
    guard InputLevelReport.validOwner(owner), ["microphone", "system"].contains(lane),
      (1...CapturedClockMap.maximumTrackSegments).contains(maximumTotalClockSegments)
    else {
      throw ProbeError.capture("raw segment owner or lane is invalid")
    }
    self.owner = owner
    self.lane = lane
    self.beforeAppend = beforeAppend
    self.storageIO = storageIO
    self.maximumTotalClockSegments = maximumTotalClockSegments
    self.allowsClockRebasing = allowsClockRebasing
    self.commits = commits
    directory = try InputEventJournalFile.pin(output)
    do {
      let data = try CaptureRawPrefix().tip(owner: owner, lane: lane, closed: false)
      pointer = try InputEventJournalFile(
        output: output, initial: data, leaf: lane == "microphone" ? .rawMicrophone : .rawSystem,
        storageIO: tipStorageIO)
    } catch {
      close(directory)
      if let posix = error as? POSIXError, [.ENOSPC, .EDQUOT, .EIO].contains(posix.code) {
        let code = Self.storageFailure(error, fallback: "raw_segment_write_or_sync_failed")
        throw ProbeError.capture(CaptureFailureGuide.message(code))
      }
      throw error
    }
  }
  deinit {
    close(directory)
  }

  func snapshot() -> RawSegmentStatus {
    lock.withLock {
      RawSegmentStatus(
        ownerId: owner, lane: lane, sealedSegments: sealedCount, sealedFrames: sealedFrames,
        lastEntrySHA256: previousHash, producerClosed: closed, failure: failure ?? admissionFailure)
    }
  }

  func enqueue(
    _ sample: CMSampleBuffer, observation: CaptureClockObservation?, sourceEpoch: UInt64
  ) throws {
    guard sample.isValid, sample.numSamples > 0, sample.numSamples <= 96_000,
      let description = sample.formatDescription
    else { throw ProbeError.capture("raw segment sample is invalid") }
    let format = AVAudioFormat(cmAudioFormatDescription: description)
    try Self.requireFormat(format, frames: sample.numSamples)
    guard
      let buffer = AVAudioPCMBuffer(
        pcmFormat: format, frameCapacity: AVAudioFrameCount(sample.numSamples))
    else { throw ProbeError.capture("raw segment sample allocation failed") }
    buffer.frameLength = AVAudioFrameCount(sample.numSamples)
    guard
      CMSampleBufferCopyPCMDataIntoAudioBufferList(
        sample, at: 0, frameCount: Int32(sample.numSamples), into: buffer.mutableAudioBufferList
      ) == noErr
    else { throw ProbeError.capture("raw segment sample copy failed") }
    try enqueue(buffer, observation: observation, sourceFrame: nil, sourceEpoch: sourceEpoch)
  }

  func enqueue(
    _ buffer: AVAudioPCMBuffer, observation: CaptureClockObservation?, sourceFrame: Int64?,
    sourceEpoch: UInt64 = 0
  ) throws {
    let frames = Int(buffer.frameLength)
    try Self.requireFormat(buffer.format, frames: frames)
    guard let observation, case .timestamp = observation else {
      throw ProbeError.capture("raw segment capture timestamp is unavailable")
    }
    let bytes = frames * Int(buffer.format.channelCount) * 4
    let rate = Int64(buffer.format.sampleRate)
    // Round each reservation up: repeated fractional nanoseconds must never
    // admit more PCM than the shared pending limit. requireFormat bounds
    // frames and the integer rate, so this numerator cannot overflow Int64.
    let duration = (Int64(frames) * 1_000_000_000 + rate - 1) / rate
    try lock.withLock {
      guard accepting, failure == nil, admissionFailure == nil else {
        throw ProbeError.capture("raw segment writer is closed or failed")
      }
      guard
        CaptureCore.permitsPending(
          bytes: pendingBytes, nanoseconds: pendingNanoseconds,
          packets: pendingPackets, nextBytes: bytes, nextNanoseconds: duration)
      else {
        accepting = false
        admissionFailure = "raw_segment_callback_rejected"
        throw ProbeError.capture("raw segment writer exceeded its admission capacity")
      }
      guard commits.reserveAcquired(lane: lane, nanoseconds: duration) else {
        accepting = false
        admissionFailure = "raw_segment_callback_rejected"
        throw ProbeError.capture("recording storage is more than 120 seconds behind acquisition")
      }
      pendingBytes += bytes
      pendingNanoseconds += duration
      pendingPackets += 1
      do {
        guard
          let copy = AVAudioPCMBuffer(
            pcmFormat: buffer.format, frameCapacity: buffer.frameLength)
        else { throw ProbeError.capture("raw segment buffer allocation failed") }
        copy.frameLength = buffer.frameLength
        let from = UnsafeMutableAudioBufferListPointer(
          UnsafeMutablePointer(mutating: buffer.audioBufferList))
        let to = UnsafeMutableAudioBufferListPointer(copy.mutableAudioBufferList)
        guard from.count == to.count else { throw ProbeError.capture("raw segment layout changed") }
        for index in from.indices {
          guard let src = from[index].mData, let dst = to[index].mData,
            from[index].mDataByteSize == to[index].mDataByteSize
          else { throw ProbeError.capture("raw segment PCM layout is invalid") }
          memcpy(dst, src, Int(from[index].mDataByteSize))
        }
        let payload = Payload(
          buffer: copy, observation: observation, sourceFrame: sourceFrame,
          sourceEpoch: sourceEpoch, bytes: bytes, durationNanoseconds: duration)
        // Keep admission and queue ordering under one lock. finish() cannot drain
        // before an already accepted callback has actually entered the queue.
        queue.async { [self] in
          defer {
            lock.withLock {
              pendingBytes -= payload.bytes
              pendingNanoseconds -= payload.durationNanoseconds
              pendingPackets -= 1
            }
          }
          beforeAppend()
          guard lock.withLock({ failure == nil }) else { return }
          do { try appendSource(payload) } catch {
            FileHandle.standardError.write(Data("mon_capture_compact_save: \(error)\n".utf8))
            lock.withLock {
              if failure == nil {
                failure = Self.storageFailure(error, fallback: "raw_segment_write_or_sync_failed")
              }
            }
          }
        }
      } catch {
        pendingBytes -= bytes
        pendingNanoseconds -= duration
        pendingPackets -= 1
        commits.releaseAcquired(lane: lane, nanoseconds: duration)
        accepting = false
        admissionFailure = "raw_segment_callback_copy_failed"
        throw error
      }
    }
  }

  /// Non-callback inspection barrier. It does not seal an incomplete raw CAF,
  /// declare successful acquisition or advertise PCM without a synced entry.
  func drainAcceptedPCM() throws {
    queue.sync {}
    commits.drain(lane: lane)
    try requireHealthy()
  }

  /// Also used by fault tests to prove normalization progresses independently
  /// of a blocked durable commit, without publishing buffered audio.
  func drainNormalization() throws {
    queue.sync {}
    try requireHealthy()
  }

  private func requireHealthy(allowAdmissionFailure: Bool = false) throws {
    if let failure = lock.withLock({ failure ?? (allowAdmissionFailure ? nil : admissionFailure) })
    {
      throw ProbeError.capture(CaptureFailureGuide.message(failure))
    }
  }

  /// Called after native callbacks drain. Normalization and durable commits
  /// have distinct barriers; neither a queued job nor an in-memory tail is a receipt.
  func finish() throws {
    finishLock.lock()
    defer { finishLock.unlock() }
    lock.withLock { accepting = false }
    do {
      try queue.sync {
        try requireHealthy(allowAdmissionFailure: true)
        if lock.withLock({ closed }) { return }
        try converter?.finish { [self] pcm, start, end in
          try appendCompact(pcm, start: start, end: end)
        }
        converter = nil
        if let active {
          let maps = try active.timing.finish(
            originNanoseconds: active.origin, expectedFrames: active.frames)
          try seal(active, maps: maps)
        }
        guard sourceCredits.isEmpty else {
          throw ProbeError.capture("compact conversion retained unmatched acquisition credits")
        }
      }
      commits.drain(lane: lane)
      // Accepted audio has been rescued, but refused acquisition must remain a
      // failed, unclosed producer rather than becoming a successful recording.
      try requireHealthy()
      if lock.withLock({ closed }) { return }
      try commits.beginControl()
      commits.submit(lane: lane, bytes: RawSegmentCommitWorker.controlReserveBytes) { [self] in
        guard lock.withLock({ failure == nil }) else { return }
        do {
          try pointer.replace(try tip(producerClosed: true))
          lock.withLock { closed = true }
        } catch {
          lock.withLock {
            commitPoison = true
            if failure == nil {
              failure = Self.storageFailure(error, fallback: "raw_segment_final_sync_failed")
            }
          }
        }
      }
      commits.drain(lane: lane)
      try requireHealthy()
    } catch {
      // Drain admitted commits before returning: the caller may release the
      // pinned directory after this method fails.
      commits.drain(lane: lane)
      let code = Self.storageFailure(error, fallback: "raw_segment_final_sync_failed")
      lock.withLock { if failure == nil && admissionFailure == nil { failure = code } }
      throw error
    }
  }

  /// Each committed segment was read back and verified before its receipt.
  /// Do not require or reconstruct a redundant continuous CAF on normal stop.
  func verifyClosed(expected: TrackStats) throws {
    try requireHealthy()
    let valid = lock.withLock {
      closed && prefix.frames == expected.frames && compactStats.frames == expected.frames
        && (expected.frames == 0 || (expected.sampleRate == 16000 && expected.channels == 1))
    }
    guard valid else { throw ProbeError.capture("recorded raw prefix differs from saved audio") }
    try pointer.verifyCurrent(try tip(producerClosed: true))
  }

  private static func requireFormat(_ format: AVAudioFormat, frames: Int) throws {
    guard format.commonFormat == .pcmFormatFloat32, format.sampleRate.isFinite,
      format.sampleRate.rounded() == format.sampleRate, (8000...384000).contains(format.sampleRate),
      (1...8).contains(format.channelCount), frames > 0,
      frames <= Int(format.sampleRate) / 4
    else { throw ProbeError.capture("raw segment PCM format or callback size is invalid") }
  }

  private func appendSource(_ payload: Payload) throws {
    guard case .timestamp(let observedStart) = payload.observation else {
      throw ProbeError.capture("missing compact capture clock")
    }
    let rate = UInt32(payload.buffer.format.sampleRate)
    let channels = payload.buffer.format.channelCount
    let frames = Int64(payload.buffer.frameLength)
    let duration = frames * 1_000_000_000 / Int64(rate)
    let start =
      try allowsClockRebasing
      ? alignment.projectedStart(observedStart, sourceEpoch: payload.sourceEpoch) : observedStart
    let (nominalEnd, overflow) = start.addingReportingOverflow(duration)
    guard !overflow, converterEpoch.map({ payload.sourceEpoch >= $0 }) ?? true else {
      throw ProbeError.capture("source PCM clock overflow or retired binding")
    }
    let epochChanged = converterEpoch != nil && converterEpoch != payload.sourceEpoch
    if !epochChanged, converter != nil,
      converter?.rate != rate || converter?.channels != channels
        || (payload.sourceFrame != nil && sourceEnd != nil && payload.sourceFrame! < sourceEnd!)
    {
      throw ProbeError.capture("source PCM format or counter changed without a retired binding")
    }
    let gap: Bool
    if let previous = sourceEnd, let frame = payload.sourceFrame, !epochChanged {
      gap = frame != previous
    } else {
      gap =
        sourceCaptureEnd.map { start > $0 + (1_000_000_000 + Int64(rate) - 1) / Int64(rate) }
        ?? false
    }
    if converter != nil
      && (epochChanged || gap || converter?.rate != rate || converter?.channels != channels)
    {
      try converter?.finish { [self] pcm, start, end in
        try appendCompact(pcm, start: start, end: end)
      }
      converter = nil
      outputEpoch += 1
    }
    if converter == nil {
      converter = try VoicePCM(rate: rate, channels: channels)
      converterInputFrames = 0
      converterOutputBase = lock.withLock { compactStats.frames }
    }
    let mapped =
      try allowsClockRebasing
      ? alignment.map(
        start: observedStart, duration: duration, frames: frames, sourceEpoch: payload.sourceEpoch,
        emittedThrough: compactCaptureEnd) : (start: start, end: nominalEnd)
    converterInputFrames += frames
    let target = (converterInputFrames * 16_000 + Int64(rate) - 1) / Int64(rate)
    sourceCredits.append(
      SourceCredit(
        frameEnd: converterOutputBase + target, nanoseconds: payload.durationNanoseconds))
    converterEpoch = payload.sourceEpoch
    sourceEnd = payload.sourceFrame.map { $0 + frames }
    sourceCaptureEnd = mapped.end
    let samples = try VoicePCM.interleaved(payload.buffer)
    guard samples.allSatisfy({ abs($0) <= 16 }) else {
      throw ProbeError.capture("invalid captured PCM amplitude")
    }
    try converter?.push(samples, start: mapped.start, end: mapped.end) { [self] pcm, start, end in
      try appendCompact(pcm, start: start, end: end)
    }
    lock.withLock { compactStats.clockAlignment = alignment.diagnostics }
  }

  private func appendCompact(_ pcm: [Int16], start: Int64, end: Int64) throws {
    guard !pcm.isEmpty else { return }
    let frames = Int64(pcm.count)
    if let active, active.frames + frames > 64_000 || active.timing.needsSegmentRollover {
      let maps = try active.timing.finishBeforeNext(
        frames: frames, observation: .timestamp(start),
        sourceFrame: nil, sourceEpoch: outputEpoch, originNanoseconds: active.origin,
        expectedFrames: active.frames)
      try seal(active, maps: maps)
    }
    if active == nil {
      try commits.beginSegment()
      active = try Active(
        origin: start,
        timing: CaptureTrackTiming(sampleRate: 16000, maximumSegments: Self.maximumClockSegments))
    }
    guard let active else { throw ProbeError.capture("compact raw writer unavailable") }
    var timing = active.timing
    try timing.recordSaved(
      frames: frames, observation: .timestamp(start), sourceFrame: nil,
      sourceEpoch: outputEpoch, captureEndNanoseconds: end)
    try commits.validateSegmentBytes(current: active.pcm.count, next: pcm.count * 2)
    pcm.withUnsafeBytes { active.pcm.append(contentsOf: $0) }
    active.frames += frames
    active.timing = timing
    try lock.withLock {
      compactStats.sampleRate = 16000
      compactStats.channels = 1
      if compactStats.timing == nil {
        compactStats.timing = try CaptureTrackTiming(
          sampleRate: 16000, maximumSegments: maximumTotalClockSegments)
      }
      try compactStats.timing?.recordSaved(
        frames: frames, observation: .timestamp(start), sourceFrame: nil,
        sourceEpoch: outputEpoch, captureEndNanoseconds: end)
      compactStats.frames += frames
    }
    compactCaptureEnd = end
    let savedFrames = lock.withLock { compactStats.frames }
    while let first = sourceCredits.first, first.frameEnd <= savedFrames {
      active.acquiredNanoseconds += first.nanoseconds
      sourceCredits.removeFirst()
    }
  }

  private func seal(_ segment: Active, maps: [CapturedClockSegment]) throws {
    guard plannedPrefix.clockMaps <= maximumTotalClockSegments - maps.count,
      plannedPrefix.segments < Self.maximumSegments
    else { throw ProbeError.capture("raw segment total clock or segment capacity exceeded") }
    segment.pcm.replaceSubrange(0..<68, with: try VoicePCM.header(frames: segment.frames))
    let sequence = plannedPrefix.segments + 1
    let name = String(format: "raw-%@-%08d.caf", lane, sequence)
    let audioSHA256 = Self.hex(SHA256.hash(data: segment.pcm))
    let entry = try Self.encode(
      Entry(
        ownerId: owner, lane: lane, sequence: sequence, audioName: name,
        sourceFrameStart: plannedPrefix.frames, frames: segment.frames, sampleRate: 16000,
        channels: 1, captureOriginNanoseconds: segment.origin, clockSegments: maps,
        clockNanoseconds: segment.timing.finalizedEndpoints(),
        audioBytes: Int64(segment.pcm.count), audioSHA256: audioSHA256,
        previousEntrySHA256: plannedPrefix.hash))
    // Admission is a pure bounded plan. The durable worker must never discover
    // an over-capacity receipt only after writing its audio.
    let plan = try plannedPrefix.prepare(entry)
    let job = SegmentJob(
      sequence: sequence, frameStart: plannedPrefix.frames,
      frames: segment.frames, pcm: segment.pcm, acquiredNanoseconds: segment.acquiredNanoseconds,
      priorHash: plannedPrefix.hash, audioSHA256: audioSHA256, entry: entry,
      nextPrefix: plan.prefix, tip: plan.tip)
    plannedPrefix = plan.prefix
    active = nil
    commits.submit(lane: lane, bytes: RawSegmentCommitWorker.segmentReserveBytes) { [self] in
      guard lock.withLock({ !commitPoison }) else { return }
      do {
        try commit(job)
        commits.releaseAcquired(lane: lane, nanoseconds: job.acquiredNanoseconds)
      } catch {
        lock.withLock {
          commitPoison = true
          if failure == nil {
            failure = Self.storageFailure(error, fallback: "raw_segment_write_or_sync_failed")
          }
        }
      }
    }
  }

  private func commit(_ segment: SegmentJob) throws {
    guard
      lock.withLock({
        sealedCount + 1 == segment.sequence && sealedFrames == segment.frameStart
          && previousHash == segment.priorHash
      })
    else { throw ProbeError.capture("raw segment commit order changed") }
    let name = String(format: "raw-%@-%08d.caf", lane, segment.sequence)
    let audio = try create(name)
    defer { close(audio) }
    var audioIdentity = stat()
    guard fstat(audio, &audioIdentity) == 0 else {
      throw ProbeError.capture("raw segment identity is unavailable")
    }
    let header = try VoicePCM.header(frames: segment.frames)
    try storageIO.write(segment.pcm, audio)
    let size = Int64(header.count) + segment.frames * 2
    try require(audio, name: name, identity: audioIdentity, size: size)
    try storageIO.synchronize(audio)
    guard lseek(audio, 0, SEEK_SET) == 0 else {
      throw ProbeError.capture("raw segment sync failed")
    }
    var frozen = stat()
    guard fstat(audio, &frozen) == 0 else {
      throw ProbeError.capture("raw segment sync identity is unavailable")
    }
    // Stream the actual descriptor back, comparing PCM to what was written.
    // Hashing an arbitrary later pathname is not proof of our captured audio.
    var whole = SHA256()
    var pcm = SHA256()
    var offset: Int64 = 0
    var block = [UInt8](repeating: 0, count: 128 * 1024)
    while offset < size {
      let count = Darwin.read(audio, &block, min(block.count, Int(size - offset)))
      if count < 0 && errno == EINTR { continue }
      guard count > 0 else { throw ProbeError.capture("raw segment readback failed") }
      let bytes = Data(block.prefix(count))
      whole.update(data: bytes)
      let pcmStart = max(0, header.count - Int(offset))
      if pcmStart < count { pcm.update(data: bytes.dropFirst(pcmStart)) }
      if offset == 0, !bytes.starts(with: header) {
        throw ProbeError.capture("raw segment finalized header changed")
      }
      offset += Int64(count)
    }
    guard pcm.finalize() == SHA256.hash(data: segment.pcm.dropFirst(header.count)),
      Self.hex(whole.finalize()) == segment.audioSHA256
    else {
      throw ProbeError.capture("raw segment stored PCM differs from accepted frames")
    }
    try require(audio, name: name, identity: audioIdentity, size: size)
    try requireUnmodified(audio, frozen: frozen)
    let sequence = segment.sequence
    let entry = segment.entry
    let entryName = String(format: "raw-%@-%08d.json", lane, sequence)
    let descriptor = try create(entryName)
    defer { close(descriptor) }
    var identity = stat()
    guard fstat(descriptor, &identity) == 0 else {
      throw ProbeError.capture("raw segment entry identity is unavailable")
    }
    try storageIO.write(entry, descriptor)
    try require(descriptor, name: entryName, identity: identity, size: Int64(entry.count))
    var entryFrozen = stat()
    guard fstat(descriptor, &entryFrozen) == 0 else {
      throw ProbeError.capture("raw segment entry identity is unavailable")
    }
    try storageIO.synchronize(descriptor)
    try storageIO.synchronize(directory)
    guard lseek(descriptor, 0, SEEK_SET) == 0 else {
      throw ProbeError.capture("raw segment entry seek failed")
    }
    var entryRead = [UInt8](repeating: 0, count: entry.count)
    var readOffset = 0
    while readOffset < entryRead.count {
      let remaining = entryRead.count - readOffset
      let count = entryRead.withUnsafeMutableBytes {
        Darwin.read(
          descriptor, $0.baseAddress!.advanced(by: readOffset), remaining)
      }
      if count < 0 && errno == EINTR { continue }
      guard count > 0 else { throw ProbeError.capture("raw segment entry readback failed") }
      readOffset += count
    }
    guard Data(entryRead) == entry else { throw ProbeError.capture("raw segment entry changed") }
    try requireUnmodified(descriptor, frozen: entryFrozen)
    try requireUnmodified(audio, frozen: frozen)
    try pointer.replace(segment.tip)
    try require(descriptor, name: entryName, identity: identity, size: Int64(entry.count))
    try require(audio, name: name, identity: audioIdentity, size: size)
    try requireUnmodified(descriptor, frozen: entryFrozen)
    try requireUnmodified(audio, frozen: frozen)
    lock.withLock {
      prefix = segment.nextPrefix
    }
  }

  private func tip(producerClosed: Bool) throws -> Data {
    try lock.withLock { try prefix.tip(owner: owner, lane: lane, closed: producerClosed) }
  }
  private func create(_ name: String) throws -> Int32 {
    let descriptor = openat(
      directory, name, O_RDWR | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC, 0o600)
    guard descriptor >= 0 else { throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO) }
    return descriptor
  }
  private func requireUnmodified(_ descriptor: Int32, frozen: stat) throws {
    var actual = stat()
    guard fstat(descriptor, &actual) == 0, actual.st_size == frozen.st_size,
      actual.st_dev == frozen.st_dev, actual.st_ino == frozen.st_ino,
      actual.st_mtimespec.tv_sec == frozen.st_mtimespec.tv_sec,
      actual.st_mtimespec.tv_nsec == frozen.st_mtimespec.tv_nsec,
      actual.st_ctimespec.tv_sec == frozen.st_ctimespec.tv_sec,
      actual.st_ctimespec.tv_nsec == frozen.st_ctimespec.tv_nsec
    else { throw ProbeError.capture("raw segment content changed during publication") }
  }
  private func require(_ descriptor: Int32, name: String, identity: stat, size: Int64) throws {
    var actual = stat()
    var named = stat()
    guard fstat(descriptor, &actual) == 0,
      fstatat(directory, name, &named, AT_SYMLINK_NOFOLLOW) == 0,
      actual.st_mode & S_IFMT == S_IFREG, actual.st_uid == geteuid(), actual.st_nlink == 1,
      actual.st_mode & 0o077 == 0, actual.st_size == size,
      actual.st_dev == identity.st_dev, actual.st_ino == identity.st_ino,
      actual.st_birthtimespec.tv_sec == identity.st_birthtimespec.tv_sec,
      actual.st_birthtimespec.tv_nsec == identity.st_birthtimespec.tv_nsec,
      named.st_dev == actual.st_dev, named.st_ino == actual.st_ino,
      named.st_size == actual.st_size, named.st_nlink == 1,
      named.st_mtimespec.tv_sec == actual.st_mtimespec.tv_sec,
      named.st_mtimespec.tv_nsec == actual.st_mtimespec.tv_nsec
    else { throw ProbeError.capture("raw segment leaf identity or content changed") }
  }
  private static func encode<T: Encodable>(_ value: T) throws -> Data {
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.sortedKeys]
    let data = try encoder.encode(value)
    guard data.count <= 65 * 1024 else {
      throw ProbeError.capture("raw segment entry exceeds limit")
    }
    return data
  }
  private static func hex<D: Sequence>(_ digest: D) -> String where D.Element == UInt8 {
    digest.map { String(format: "%02x", $0) }.joined()
  }
  private static func write(_ data: Data, to descriptor: Int32) throws {
    try data.withUnsafeBytes { bytes in
      var offset = 0
      while offset < data.count {
        let count = Darwin.write(
          descriptor, bytes.baseAddress!.advanced(by: offset), data.count - offset)
        if count < 0 && errno == EINTR { continue }
        guard count > 0 else { throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO) }
        offset += count
      }
    }
  }
  private static func storageFailure(_ error: Error, fallback: String) -> String {
    guard let error = error as? POSIXError else { return fallback }
    switch error.code {
    case .ENOSPC, .EDQUOT: return "raw_segment_storage_full"
    case .EIO: return "raw_segment_storage_io_failed"
    default: return fallback
    }
  }
  private static func header(rate: Int64, channels: Int, frames: Int64) throws -> Data {
    try CaptureCore.cafHeader(rate: rate, channels: channels, frames: frames)
  }
}
