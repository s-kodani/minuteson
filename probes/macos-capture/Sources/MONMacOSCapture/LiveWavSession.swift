import CryptoKit
import Darwin
import Foundation
import MONCaptureCore

struct SingleWavStatus: Codable {
  let schemaVersion: Int
  let ownerId: String
  let fileName: String
  let frames: UInt64
  let checkpoints: UInt32
  let lastEntrySHA256: String?
  let captureOriginNanoseconds: Int64?
  let producerClosed: Bool
}

/// The DSP owner never waits for disk. Immutable mixed blocks are handed to
/// the existing bounded commit worker; receipts advertise only read-back PCM.
final class LiveWavSession: @unchecked Sendable {
  private struct Credit {
    let lane: String
    let through: Int64
    let nanoseconds: Int64
  }
  private let output: URL
  private let owner: String
  private let fileName: String
  private let persistent: Bool
  private let onlyLane: UInt32?
  private let commits: RawSegmentCommitWorker
  private let directory: Int32
  private let dsp = DispatchQueue(label: "app.mon.capture.wav-dsp")
  private let lock = NSLock()
  private let mixer: OpaquePointer
  private var descriptor: Int32 = -1
  private var identity = stat()
  private var reservation: InputEventJournalFile?
  private var tip: InputEventJournalFile?
  private var prefix = mon_wav_prefix_empty()
  private var status: SingleWavStatus
  private var failure: String?
  private var sequence = [UInt64(0), UInt64(0)]
  private var ends = [Int64(0), Int64(0)]
  private var closed = [false, false]
  private var emittedFrames: UInt64 = 0
  private var origin: Int64?
  private var firstInputStart: Int64?
  private var active = Data()
  private var activeFirst: UInt64 = 0
  private var activeCredits: [Credit] = []
  private var activeMissing: [[UInt64]] = []
  private var activeCoverage = [UInt64(0), UInt64(0)]
  private var credits: [Credit] = []
  private var finished = false
  private var callbackError: Error?

  init(
    output: URL, owner: String, commits: RawSegmentCommitWorker,
    fileName: String = "recording.wav", onlyLane: UInt32? = nil
  ) throws {
    guard InputLevelReport.validOwner(owner), onlyLane.map({ $0 < 2 }) ?? true,
      ["recording.wav", "microphone.wav", "system.wav"].contains(fileName),
      let mixer = mon_wav_mixer_new()
    else { throw ProbeError.capture("WAV session owner or mode is invalid") }
    self.output = output
    self.owner = owner
    self.fileName = fileName
    self.onlyLane = onlyLane
    self.persistent = onlyLane == nil
    self.commits = commits
    self.mixer = mixer
    do { directory = try InputEventJournalFile.pin(output) } catch {
      mon_wav_mixer_free(mixer)
      throw error
    }
    status = SingleWavStatus(
      schemaVersion: 1, ownerId: owner, fileName: fileName, frames: 0, checkpoints: 0,
      lastEntrySHA256: nil, captureOriginNanoseconds: nil, producerClosed: false)
    if persistent {
      try openFile()
      let birth =
        Int64(identity.st_birthtimespec.tv_sec) * 1_000_000_000
        + Int64(identity.st_birthtimespec.tv_nsec)
      let data = try JSONSerialization.data(
        withJSONObject: [
          "schemaVersion": 1, "ownerId": owner, "fileName": fileName,
          "device": UInt64(UInt32(bitPattern: identity.st_dev)),
          "inode": UInt64(identity.st_ino), "birthNanoseconds": birth,
        ], options: [.sortedKeys])
      reservation = try InputEventJournalFile(
        output: output, initial: data, leaf: .singleWavOwner)
      tip = try InputEventJournalFile(
        output: output, initial: try prefixTip(closed: false), leaf: .singleWav)
      try reservation?.requireDirectoryIdentity(directory)
      try tip?.requireDirectoryIdentity(directory)
    }
  }

  deinit {
    if descriptor >= 0 { close(descriptor) }
    close(directory)
    mon_wav_mixer_free(mixer)
  }

  var failureDetail: String? { lock.withLock { failure } }
  func snapshot() -> SingleWavStatus { lock.withLock { status } }
  func requireHealthy() throws {
    if let failure = failureDetail { throw ProbeError.capture(failure) }
  }
  private func failed(_ error: Error) {
    lock.withLock { if failure == nil { failure = String(describing: error) } }
  }
  private func index(_ lane: String) -> UInt32 { lane == "microphone" ? 0 : 1 }

  /// Called only after the lane's admitted PCM has passed its serial converter.
  func append(
    lane: String, epoch: UInt64, pcm: [Int16], start: Int64, end: Int64,
    acquiredNanoseconds: Int64
  ) throws {
    try dsp.sync {
      try requireHealthy()
      let lane = index(lane)
      guard !finished, !closed[Int(lane)], onlyLane.map({ $0 == lane }) ?? true else {
        throw ProbeError.capture("PCM entered a closed WAV lane")
      }
      sequence[Int(lane)] += 1
      let accepted = pcm.withUnsafeBufferPointer {
        mon_wav_mixer_push(
          mixer, lane, epoch, sequence[Int(lane)], $0.baseAddress, $0.count, start, end)
      }
      guard accepted else {
        throw ProbeError.capture("WAV input order or capture clock is invalid")
      }
      firstInputStart = firstInputStart.map { min($0, start) } ?? start
      ends[Int(lane)] = end
      credits.append(
        Credit(
          lane: lane == 0 ? "microphone" : "system", through: end,
          nanoseconds: acquiredNanoseconds))
      guard mon_wav_mixer_advance(mixer, lane, sequence[Int(lane)], end) else {
        throw ProbeError.capture("WAV conversion frontier is invalid")
      }
      if onlyLane != nil {
        let peer = 1 - lane
        ends[Int(peer)] = end
        guard mon_wav_mixer_advance(mixer, peer, 0, end) else {
          throw ProbeError.capture("single-input WAV frontier is invalid")
        }
      }
      try drain(through: min(ends[0], ends[1]))
    }
  }

  /// A FIFO observation follows every PCM callback already admitted by this
  /// lane. It must never be issued by a timer directly against the mixer.
  func observeUnreceived(lane: String, through: Int64) throws {
    try dsp.sync {
      try requireHealthy()
      guard !finished, onlyLane == nil else { return }
      let lane = index(lane)
      let through = max(through, ends[Int(lane)])
      guard mon_wav_mixer_advance(mixer, lane, sequence[Int(lane)], through) else {
        throw ProbeError.capture("WAV observation overtook accepted PCM")
      }
      ends[Int(lane)] = through
      try drain(through: min(ends[0], ends[1]))
    }
  }

  func closeLane(_ lane: String) throws {
    try dsp.sync {
      try requireHealthy()
      closed[Int(index(lane))] = true
    }
  }

  func finish(endNanoseconds: Int64? = nil) throws {
    try dsp.sync {
      try requireHealthy()
      if let onlyLane { closed[Int(1 - onlyLane)] = true }
      guard closed.allSatisfy({ $0 }) else {
        throw ProbeError.capture("both WAV acquisitions must close before finalization")
      }
      if finished { return }
      if let endNanoseconds {
        for lane in UInt32(0)...UInt32(1) {
          let end = max(endNanoseconds, ends[Int(lane)])
          guard mon_wav_mixer_advance(mixer, lane, sequence[Int(lane)], end) else {
            throw ProbeError.capture("WAV closure frontier is invalid")
          }
          ends[Int(lane)] = end
        }
        try drain(through: min(ends[0], ends[1]))
      }
      callbackError = nil
      let success = mon_wav_mixer_finish(
        mixer, Unmanaged.passUnretained(self).toOpaque(), Self.receive)
      if let callbackError { throw callbackError }
      guard success else { throw ProbeError.capture("WAV mixer finalization failed") }
      try flush()
      finished = true
    }
    do {
      commits.drain(lane: "system")
      try requireHealthy()
      if emittedFrames == 0 { return }
      try requireFile()
      try Self.write(try VoicePCM.wavHeader(frames: Int64(emittedFrames)), to: descriptor, at: 0)
      guard fsync(descriptor) == 0 else { throw POSIXError(.EIO) }
      if persistent {
        let data = try prefixTip(closed: true)
        try tip?.replace(data)
        let value = try JSONDecoder().decode(SingleWavStatus.self, from: data)
        lock.withLock { status = value }
      }
    } catch {
      failed(error)
      throw error
    }
  }

  /// Geometry belongs to the accepted shared PCM, not an auxiliary lane map.
  func captureGeometry() -> (origin: Int64?, microphoneEnd: Int64, systemEnd: Int64) {
    dsp.sync { (origin ?? firstInputStart, ends[0], ends[1]) }
  }

  /// Retire accepted PCM after acquisition/metadata failure without claiming
  /// that either native acquisition closed. Never retry a failed disk owner.
  func preserveAcceptedPrefix() throws {
    try dsp.sync {
      try requireHealthy()
      if finished { return }
      callbackError = nil
      _ = mon_wav_mixer_finish(
        mixer, Unmanaged.passUnretained(self).toOpaque(), Self.receive)
      if let callbackError { throw callbackError }
      // A rejected later input can poison the mixer while already emitted
      // PCM is still valid. Its active block remains eligible for a receipt.
      try flush()
      finished = true
    }
    commits.drain(lane: "system")
    try requireHealthy()
  }

  private func drain(through: Int64) throws {
    guard through > 0 else { return }
    callbackError = nil
    let success = mon_wav_mixer_drain(
      mixer, through, Unmanaged.passUnretained(self).toOpaque(), Self.receive)
    if let callbackError { throw callbackError }
    guard success else { throw ProbeError.capture("WAV timestamp ordering failed") }
  }

  private static let receive:
    @convention(c) (
      UnsafeMutableRawPointer?, UnsafePointer<Int16>?, Int, UInt64, Int64, UInt32
    ) -> Bool = { context, pcm, count, first, origin, coverage in
      guard let context, let pcm else { return false }
      let session = Unmanaged<LiveWavSession>.fromOpaque(context).takeUnretainedValue()
      do {
        try session.receive(
          UnsafeBufferPointer(start: pcm, count: count), first: first, origin: origin,
          coverage: coverage)
        return true
      } catch {
        session.callbackError = error
        return false
      }
    }

  private func receive(
    _ pcm: UnsafeBufferPointer<Int16>, first: UInt64, origin: Int64, coverage: UInt32
  ) throws {
    guard first == emittedFrames, self.origin == nil || self.origin == origin else {
      throw ProbeError.capture("WAV output geometry changed")
    }
    self.origin = origin
    var consumed = 0
    while consumed < pcm.count {
      if active.isEmpty {
        try commits.beginSegment()
        activeFirst = emittedFrames
      }
      let count = min(pcm.count - consumed, 64_000 - active.count / 2)
      if coverage & 1 != 0 { activeCoverage[0] += UInt64(count) }
      if coverage & 2 != 0 { activeCoverage[1] += UInt64(count) }
      if coverage == 0 {
        if let last = activeMissing.last, last[1] == emittedFrames {
          activeMissing[activeMissing.count - 1][1] = emittedFrames + UInt64(count)
        } else {
          activeMissing.append([emittedFrames, emittedFrames + UInt64(count)])
        }
      }
      for sample in pcm[consumed..<(consumed + count)] {
        var value = sample.littleEndian
        withUnsafeBytes(of: &value) { active.append(contentsOf: $0) }
      }
      emittedFrames += UInt64(count)
      consumed += count
      let end = origin + Int64(emittedFrames * 1_000_000_000 / 16000)
      while let credit = credits.first, credit.through <= end {
        activeCredits.append(credits.removeFirst())
      }
      if active.count == 128_000 { try flush() }
    }
  }

  private func flush() throws {
    guard !active.isEmpty, let origin else { return }
    let data = active
    let first = activeFirst
    let releases = activeCredits
    let missing = activeMissing
    let coverage = activeCoverage
    active = Data()
    activeCredits = []
    activeMissing = []
    activeCoverage = [0, 0]
    commits.submit(lane: "system", bytes: RawSegmentCommitWorker.segmentReserveBytes) { [self] in
      do {
        try requireHealthy()
        if descriptor < 0 { try openFile() }
        try requireFile()
        try Self.write(data, to: descriptor, at: 80 + first * 2)
        if persistent {
          guard fsync(descriptor) == 0 else { throw POSIXError(.EIO) }
          let actual = try Self.read(descriptor, at: 80 + first * 2, count: data.count)
          guard actual == data else { throw ProbeError.capture("WAV saved PCM differs from input") }
          let sequence = snapshot().checkpoints + 1
          let entry = try JSONSerialization.data(
            withJSONObject: [
              "schemaVersion": 1, "ownerId": owner, "sequence": sequence,
              "frameStart": first, "frames": data.count / 2,
              "captureOriginNanoseconds": origin,
              "pcmSHA256": Self.hex(SHA256.hash(data: actual)),
              "missingRanges": missing,
              "coverageFrames": coverage,
              "previousEntrySHA256": snapshot().lastEntrySHA256 as Any? ?? NSNull(),
            ], options: [.sortedKeys])
          var next = prefix
          var bytes = [UInt8](repeating: 0, count: 4096)
          let count = entry.withUnsafeBytes { input in
            bytes.withUnsafeMutableBufferPointer { output in
              mon_wav_prepare_commit(
                &prefix, input.bindMemory(to: UInt8.self).baseAddress, entry.count, &next,
                output.baseAddress, output.count)
            }
          }
          guard count > 0 else { throw ProbeError.capture("WAV checkpoint failed core validation") }
          let entryName = String(format: "wav-%08u.json", sequence)
          let entryFD = openat(
            directory, entryName, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC, 0o600)
          guard entryFD >= 0 else { throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO) }
          defer { close(entryFD) }
          try Self.write(entry, to: entryFD, at: 0)
          guard fsync(entryFD) == 0, fsync(directory) == 0 else { throw POSIXError(.EIO) }
          let tipData = Data(bytes.prefix(count))
          try tip?.replace(tipData)
          let value = try JSONDecoder().decode(SingleWavStatus.self, from: tipData)
          prefix = next
          lock.withLock { status = value }
        }
        for credit in releases {
          commits.releaseAcquired(lane: credit.lane, nanoseconds: credit.nanoseconds)
        }
      } catch { failed(error) }
    }
  }

  private func prefixTip(closed: Bool) throws -> Data {
    var bytes = [UInt8](repeating: 0, count: 4096)
    let count = owner.utf8CString.dropLast().withUnsafeBufferPointer { owner in
      bytes.withUnsafeMutableBufferPointer { output in
        mon_wav_prefix_tip(
          &prefix, UnsafeRawPointer(owner.baseAddress!).assumingMemoryBound(to: UInt8.self),
          owner.count, closed, output.baseAddress, output.count)
      }
    }
    guard count > 0 else { throw ProbeError.capture("WAV prefix cannot be encoded") }
    return Data(bytes.prefix(count))
  }

  private func openFile() throws {
    descriptor = openat(
      directory, fileName, O_RDWR | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC, 0o600)
    guard descriptor >= 0, fstat(descriptor, &identity) == 0 else {
      throw ProbeError.capture("WAV destination already exists or is unavailable")
    }
    try Self.write(try VoicePCM.wavHeader(frames: 0), to: descriptor, at: 0)
    guard fsync(descriptor) == 0, fsync(directory) == 0 else { throw POSIXError(.EIO) }
  }

  private func requireFile() throws {
    var actual = stat()
    var named = stat()
    guard descriptor >= 0, fstat(descriptor, &actual) == 0,
      fstatat(directory, fileName, &named, AT_SYMLINK_NOFOLLOW) == 0,
      actual.st_dev == identity.st_dev, actual.st_ino == identity.st_ino,
      actual.st_birthtimespec.tv_sec == identity.st_birthtimespec.tv_sec,
      actual.st_birthtimespec.tv_nsec == identity.st_birthtimespec.tv_nsec,
      actual.st_dev == named.st_dev, actual.st_ino == named.st_ino,
      actual.st_mode & S_IFMT == S_IFREG, actual.st_uid == geteuid(), actual.st_nlink == 1,
      actual.st_mode & 0o077 == 0
    else { throw ProbeError.capture("WAV destination identity changed") }
  }

  private static func write(_ data: Data, to file: Int32, at position: UInt64) throws {
    try data.withUnsafeBytes { bytes in
      var offset = 0
      while offset < data.count {
        let count = pwrite(
          file, bytes.baseAddress!.advanced(by: offset), data.count - offset,
          off_t(position + UInt64(offset)))
        if count < 0 && errno == EINTR { continue }
        guard count > 0 else { throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO) }
        offset += count
      }
    }
  }
  private static func read(_ file: Int32, at position: UInt64, count: Int) throws -> Data {
    var data = Data(count: count)
    try data.withUnsafeMutableBytes { bytes in
      var offset = 0
      while offset < count {
        let got = pread(
          file, bytes.baseAddress!.advanced(by: offset), count - offset,
          off_t(position + UInt64(offset)))
        if got < 0 && errno == EINTR { continue }
        guard got > 0 else { throw ProbeError.capture("WAV readback ended early") }
        offset += got
      }
    }
    return data
  }
  private static func hex(_ digest: SHA256.Digest) -> String {
    digest.map { String(format: "%02x", $0) }.joined()
  }
}
