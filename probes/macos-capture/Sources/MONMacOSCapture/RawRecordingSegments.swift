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
  static let maximumSegments = 32_768
  // Bound both affine maps and nanosecond witnesses within the 65KiB entry.
  static let maximumClockSegments = 128
  static let maximumPendingBytes = 16 * 1024 * 1024
  static let maximumPendingNanoseconds: Int64 = 500_000_000
  private struct Payload: @unchecked Sendable {
    let buffer: AVAudioPCMBuffer
    let observation: CaptureClockObservation
    let sourceFrame: Int64?
    let sourceEpoch: UInt64
    let bytes: Int
    let durationNanoseconds: Int64
  }
  private struct Tip: Encodable {
    let schemaVersion = 2
    let ownerId: String
    let lane: String
    let sealedSegments: Int
    let sealedFrames: Int64
    let lastEntrySHA256: String?
    let producerClosed: Bool
  }
  private struct Entry: Encodable {
    let schemaVersion = 2
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
    let descriptor: Int32
    let identity: stat
    let name: String
    let rate: Int64
    let channels: Int
    let origin: Int64
    var frames: Int64 = 0
    var timing: CaptureTrackTiming
    var pcmHash = SHA256()
    init(
      descriptor: Int32, identity: stat, name: String, rate: Int64, channels: Int,
      origin: Int64, timing: CaptureTrackTiming
    ) {
      self.descriptor = descriptor
      self.identity = identity
      self.name = name
      self.rate = rate
      self.channels = channels
      self.origin = origin
      self.timing = timing
    }
    deinit { close(descriptor) }
  }

  private let owner: String
  private let lane: String
  private let directory: Int32
  private let pointer: InputEventJournalFile
  private let beforeAppend: @Sendable () -> Void
  private let storageIO: StorageIO
  private let maximumTotalClockSegments: Int
  private let queue = DispatchQueue(label: "app.mon.capture.raw-segments")
  private let lock = NSLock()
  private var pendingBytes = 0
  private var pendingNanoseconds: Int64 = 0
  private var accepting = true
  private var failure: String?
  private var closed = false
  private var sealedCount = 0
  private var sealedFrames: Int64 = 0
  private var sealedClockSegments = 0
  private var sealedMetadataBytes = 0
  private var previousHash: String?
  // Only accessed by the serial writer queue, including finish().
  private var active: Active?

  init(
    output: URL, owner: String, lane: String,
    beforeAppend: @escaping @Sendable () -> Void = {}, storageIO: StorageIO = StorageIO(),
    tipStorageIO: InputEventJournalFile.StorageIO = InputEventJournalFile.StorageIO(),
    maximumTotalClockSegments: Int = CapturedClockMap.maximumTrackSegments
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
    directory = try InputEventJournalFile.pin(output)
    do {
      let data = try Self.encode(
        Tip(
          ownerId: owner, lane: lane, sealedSegments: 0, sealedFrames: 0,
          lastEntrySHA256: nil, producerClosed: false))
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
  deinit { close(directory) }

  func snapshot() -> RawSegmentStatus {
    lock.withLock {
      RawSegmentStatus(
        ownerId: owner, lane: lane, sealedSegments: sealedCount, sealedFrames: sealedFrames,
        lastEntrySHA256: previousHash, producerClosed: closed, failure: failure)
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
    // admit more PCM than the half-second pending limit. requireFormat bounds
    // frames and the integer rate, so this numerator cannot overflow Int64.
    let duration = (Int64(frames) * 1_000_000_000 + rate - 1) / rate
    try lock.withLock {
      guard accepting, failure == nil, pendingBytes <= Self.maximumPendingBytes - bytes,
        pendingNanoseconds <= Self.maximumPendingNanoseconds - duration
      else {
        failure = failure ?? "raw_segment_callback_rejected"
        throw ProbeError.capture("raw segment writer is closed, failed or over capacity")
      }
      pendingBytes += bytes
      pendingNanoseconds += duration
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
            }
          }
          beforeAppend()
          guard lock.withLock({ failure == nil }) else { return }
          do { try append(payload) } catch {
            lock.withLock {
              failure = Self.storageFailure(error, fallback: "raw_segment_write_or_sync_failed")
            }
          }
        }
      } catch {
        pendingBytes -= bytes
        pendingNanoseconds -= duration
        failure = "raw_segment_callback_copy_failed"
        throw error
      }
    }
  }

  /// Non-callback inspection barrier. It does not seal an incomplete raw CAF,
  /// declare successful acquisition or advertise PCM without a synced entry.
  func drainAcceptedPCM() throws {
    try queue.sync {
      if let failure = lock.withLock({ failure }) {
        throw ProbeError.capture(CaptureFailureGuide.message(failure))
      }
    }
  }

  /// Call only after the source callbacks have drained. No acquisition-closed
  /// claim is made here; the native recorder proves that separately.
  func finish() throws {
    lock.withLock { accepting = false }
    try queue.sync {
      if let failure = lock.withLock({ failure }) {
        throw ProbeError.capture(CaptureFailureGuide.message(failure))
      }
      if lock.withLock({ closed }) { return }
      do {
        if let active {
          let maps = try active.timing.finish(
            originNanoseconds: active.origin, expectedFrames: active.frames)
          try seal(active, maps: maps)
        }
        try pointer.replace(try tip(producerClosed: true))
        lock.withLock { closed = true }
      } catch {
        let code = Self.storageFailure(error, fallback: "raw_segment_final_sync_failed")
        lock.withLock { failure = code }
        if code == "raw_segment_final_sync_failed" { throw error }
        throw ProbeError.capture(CaptureFailureGuide.message(code))
      }
    }
  }

  private static func requireFormat(_ format: AVAudioFormat, frames: Int) throws {
    guard format.commonFormat == .pcmFormatFloat32, format.sampleRate.isFinite,
      format.sampleRate.rounded() == format.sampleRate, (8000...384000).contains(format.sampleRate),
      (1...8).contains(format.channelCount), frames > 0,
      frames <= Int(format.sampleRate) / 4
    else { throw ProbeError.capture("raw segment PCM format or callback size is invalid") }
  }

  private func append(_ payload: Payload) throws {
    let rate = Int64(payload.buffer.format.sampleRate)
    let channels = Int(payload.buffer.format.channelCount)
    let frames = Int64(payload.buffer.frameLength)
    if let active {
      guard active.rate == rate, active.channels == channels else {
        // No normalization policy has been verified yet. Preserve existing
        // raw files rather than pretending a changed format is the old track.
        throw ProbeError.capture("raw segment source format changed")
      }
      if active.frames >= rate * 4 || active.timing.needsSegmentRollover {
        let maps = try active.timing.finishBeforeNext(
          frames: frames, observation: payload.observation, sourceFrame: payload.sourceFrame,
          sourceEpoch: payload.sourceEpoch, originNanoseconds: active.origin,
          expectedFrames: active.frames)
        try seal(active, maps: maps)
      }
    }
    if active == nil {
      guard case .timestamp(let origin) = payload.observation,
        lock.withLock({ sealedCount < Self.maximumSegments })
      else { throw ProbeError.capture("raw segment sequence capacity exceeded") }
      let name = String(format: "raw-%@-%08d.caf", lane, lock.withLock { sealedCount + 1 })
      let descriptor = try create(name)
      var identity = stat()
      do {
        guard fstat(descriptor, &identity) == 0 else {
          throw ProbeError.capture("raw segment identity is unavailable")
        }
        try storageIO.write(Self.header(rate: rate, channels: channels, frames: 0), descriptor)
        active = try Active(
          descriptor: descriptor, identity: identity, name: name, rate: rate, channels: channels,
          origin: origin,
          timing: CaptureTrackTiming(
            sampleRate: Double(rate), maximumSegments: Self.maximumClockSegments))
      } catch {
        close(descriptor)
        throw error
      }
    }
    guard let active, let data = payload.buffer.floatChannelData else {
      throw ProbeError.capture("raw segment PCM is unavailable")
    }
    var timing = active.timing
    try timing.recordSaved(
      frames: frames, observation: payload.observation, sourceFrame: payload.sourceFrame,
      sourceEpoch: payload.sourceEpoch)
    let interleaved = payload.buffer.format.isInterleaved
    for start in stride(from: 0, to: Int(frames), by: 4096) {
      var bytes = Data(capacity: min(4096, Int(frames) - start) * channels * 4)
      for frame in start..<min(start + 4096, Int(frames)) {
        for channel in 0..<channels {
          let value = data[interleaved ? 0 : channel][
            interleaved ? frame * channels + channel : frame]
          guard value.isFinite, abs(value) <= 16 else {
            throw ProbeError.capture("raw segment PCM sample is invalid")
          }
          var bits = value.bitPattern.littleEndian
          withUnsafeBytes(of: &bits) { bytes.append(contentsOf: $0) }
        }
      }
      try storageIO.write(bytes, active.descriptor)
      active.pcmHash.update(data: bytes)
    }
    active.frames += frames
    active.timing = timing
  }

  private func seal(_ segment: Active, maps: [CapturedClockSegment]) throws {
    guard sealedClockSegments <= maximumTotalClockSegments - maps.count else {
      throw ProbeError.capture("raw segment total clock capacity exceeded")
    }
    let header = Self.header(rate: segment.rate, channels: segment.channels, frames: segment.frames)
    guard lseek(segment.descriptor, 0, SEEK_SET) == 0 else {
      throw ProbeError.capture("raw segment header seek failed")
    }
    try storageIO.write(header, segment.descriptor)
    let size = Int64(header.count) + segment.frames * Int64(segment.channels) * 4
    try require(segment.descriptor, name: segment.name, identity: segment.identity, size: size)
    try storageIO.synchronize(segment.descriptor)
    guard lseek(segment.descriptor, 0, SEEK_SET) == 0 else {
      throw ProbeError.capture("raw segment sync failed")
    }
    var frozen = stat()
    guard fstat(segment.descriptor, &frozen) == 0 else {
      throw ProbeError.capture("raw segment sync identity is unavailable")
    }
    // Stream the actual descriptor back, comparing PCM to what was written.
    // Hashing an arbitrary later pathname is not proof of our captured audio.
    var whole = SHA256()
    var pcm = SHA256()
    var offset: Int64 = 0
    var block = [UInt8](repeating: 0, count: 128 * 1024)
    while offset < size {
      let count = Darwin.read(segment.descriptor, &block, min(block.count, Int(size - offset)))
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
    guard pcm.finalize() == segment.pcmHash.finalize() else {
      throw ProbeError.capture("raw segment stored PCM differs from accepted frames")
    }
    try require(segment.descriptor, name: segment.name, identity: segment.identity, size: size)
    try requireUnmodified(segment.descriptor, frozen: frozen)
    let sequence = lock.withLock { sealedCount + 1 }
    let entry = try Self.encode(
      Entry(
        ownerId: owner, lane: lane, sequence: sequence, audioName: segment.name,
        sourceFrameStart: lock.withLock { sealedFrames }, frames: segment.frames,
        sampleRate: segment.rate, channels: segment.channels,
        captureOriginNanoseconds: segment.origin, clockSegments: maps,
        clockNanoseconds: try segment.timing.finalizedEndpoints(),
        audioBytes: size, audioSHA256: Self.hex(whole.finalize()),
        previousEntrySHA256: previousHash))
    let entryName = String(format: "raw-%@-%08d.json", lane, sequence)
    // Keep the producer's entries plus tip within the reader's 32MiB entry
    // budget. Reserve 1KiB for bounded tip fields and producerClosed update.
    guard sealedMetadataBytes <= 32 * 1024 * 1024 - 1024 - entry.count else {
      throw ProbeError.capture("raw segment total metadata capacity exceeded")
    }
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
    let nextHash = Self.hex(SHA256.hash(data: entry))
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
    try requireUnmodified(segment.descriptor, frozen: frozen)
    let nextTip = try Self.encode(
      Tip(
        ownerId: owner, lane: lane, sealedSegments: sequence,
        sealedFrames: lock.withLock { sealedFrames } + segment.frames,
        lastEntrySHA256: nextHash, producerClosed: false))
    try pointer.replace(nextTip)
    try require(descriptor, name: entryName, identity: identity, size: Int64(entry.count))
    try require(segment.descriptor, name: segment.name, identity: segment.identity, size: size)
    try requireUnmodified(descriptor, frozen: entryFrozen)
    try requireUnmodified(segment.descriptor, frozen: frozen)
    lock.withLock {
      previousHash = nextHash
      sealedCount = sequence
      sealedFrames += segment.frames
    }
    sealedClockSegments += maps.count
    sealedMetadataBytes += entry.count
    active = nil
  }

  private func tip(producerClosed: Bool) throws -> Data {
    try Self.encode(
      Tip(
        ownerId: owner, lane: lane, sealedSegments: lock.withLock { sealedCount },
        sealedFrames: lock.withLock { sealedFrames }, lastEntrySHA256: previousHash,
        producerClosed: producerClosed))
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
  private static func header(rate: Int64, channels: Int, frames: Int64) -> Data {
    var data = Data("caff".utf8)
    func big<T: FixedWidthInteger>(_ value: T) {
      var value = value.bigEndian
      withUnsafeBytes(of: &value) { data.append(contentsOf: $0) }
    }
    big(UInt16(1))
    big(UInt16(0))
    data.append(Data("desc".utf8))
    big(UInt64(32))
    big(Double(rate).bitPattern)
    data.append(Data("lpcm".utf8))
    // Float32, little-endian (CAF-specific flags), interleaved, one frame/packet.
    big(UInt32(3))
    big(UInt32(channels * 4))
    big(UInt32(1))
    big(UInt32(channels))
    big(UInt32(32))
    data.append(Data("data".utf8))
    big(UInt64(4 + frames * Int64(channels) * 4))
    big(UInt32(0))
    return data
  }
}
