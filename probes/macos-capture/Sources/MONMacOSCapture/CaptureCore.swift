import Darwin
import Foundation
import MONCaptureCore

/// Native wrappers never reproduce policy decisions made by the Rust core.
enum CaptureCore {
  static let maximumSegments = Int(mon_capture_limit(1))
  static let maximumEntryBytes = Int(mon_capture_limit(2))
  static let maximumPendingBytes = Int(mon_capture_limit(6))
  static let maximumPendingNanoseconds = Int64(mon_capture_limit(7))
  static let maximumPendingPackets = Int(mon_capture_limit(8))
  static let acquisitionTimeoutNanoseconds = Int64(mon_capture_limit(9))
  static let retryNanoseconds = Int64(mon_capture_limit(10))

  static func validOwner(_ value: String) -> Bool {
    Array(value.utf8).withUnsafeBufferPointer { bytes in
      mon_capture_abi_version() == 1 && mon_capture_valid_owner(bytes.baseAddress, bytes.count)
    }
  }
  static func validUID(_ value: String) -> Bool {
    Array(value.utf8).withUnsafeBufferPointer { mon_capture_valid_uid($0.baseAddress, $0.count) }
  }
  static func validSelection(_ bytes: Data, owner: String) -> Bool {
    bytes.withUnsafeBytes { input in
      Array(owner.utf8).withUnsafeBufferPointer { ownerBytes in
        mon_capture_valid_selection(
          input.bindMemory(to: UInt8.self).baseAddress, input.count,
          ownerBytes.baseAddress, ownerBytes.count)
      }
    }
  }
  static func permitsPending(
    bytes: Int, nanoseconds: Int64, packets: Int,
    nextBytes: Int, nextNanoseconds: Int64
  ) -> Bool {
    guard bytes >= 0, nanoseconds >= 0, packets >= 0, packets <= Int(UInt32.max),
      nextBytes >= 0, nextNanoseconds >= 0
    else { return false }
    return mon_capture_permits_pending(
      UInt64(bytes), UInt64(nanoseconds), UInt32(packets),
      UInt64(nextBytes), UInt64(nextNanoseconds))
  }
  static func permitsCommit(
    segments: Int, metadata: Int, maps: Int,
    nextBytes: Int, nextMaps: Int
  ) -> Bool {
    guard segments >= 0, segments <= Int(UInt32.max), metadata >= 0, maps >= 0,
      nextBytes >= 0, nextMaps >= 0
    else { return false }
    return mon_capture_permits_commit(
      UInt32(segments), UInt64(metadata), UInt64(maps),
      UInt64(nextBytes), UInt64(nextMaps))
  }
  static func catalogOrder<T: Encodable>(_ rows: [T], applications: Bool) throws -> [Int] {
    let data = try JSONEncoder().encode(rows)
    var indices = [UInt32](repeating: 0, count: 128)
    let count = data.withUnsafeBytes { input in
      indices.withUnsafeMutableBufferPointer { output in
        mon_capture_catalog_order(
          input.bindMemory(to: UInt8.self).baseAddress, input.count,
          applications, UInt32(getpid()), UInt32(getppid()), output.baseAddress, output.count)
      }
    }
    guard count >= 0 else {
      throw ProbeError.capture("capture catalog exceeds capacity or has invalid identities")
    }
    return indices.prefix(count).map(Int.init)
  }
  static func validateM4A(_ handle: FileHandle) throws {
    let length = try handle.seekToEnd()
    let valid = mon_media_validate_m4a(length, Unmanaged.passUnretained(handle).toOpaque()) {
      context, offset, output, count in
      guard let context, let output else { return -1 }
      let handle = Unmanaged<FileHandle>.fromOpaque(context).takeUnretainedValue()
      do {
        try handle.seek(toOffset: offset)
        guard let data = try handle.read(upToCount: count), data.count == count else { return -1 }
        data.copyBytes(to: output, count: count)
        return count
      } catch { return -1 }
    }
    guard valid else { throw ProbeError.usage("invalid M4A audio/chapter track structure") }
  }
  static func cafHeader(rate: Int64, channels: Int, frames: Int64) throws -> Data {
    guard rate >= 0, rate <= UInt32.max, channels >= 0, channels <= UInt32.max, frames >= 0 else {
      throw ProbeError.capture("invalid CAF PCM geometry")
    }
    var bytes = [UInt8](repeating: 0, count: 68)
    let count = bytes.withUnsafeMutableBufferPointer {
      mon_capture_caf_header(
        UInt32(rate), UInt32(channels), UInt64(frames), $0.baseAddress, $0.count)
    }
    guard count == bytes.count else { throw ProbeError.capture("invalid CAF PCM geometry") }
    return Data(bytes)
  }
}

struct CaptureLifecycle {
  private var value = MonCaptureLifecycle(epoch: 0, interruptions: 0, phase: 0, interrupted: 0)
  var epoch: UInt64 { value.epoch }
  var interruptions: UInt64 { value.interruptions }
  var stopped: Bool { value.phase == 4 }
  var state: String {
    switch value.phase {
    case 0: "reconnecting"
    case 1: "connected"
    case 2: "disconnected"
    case 3: "unavailable"
    default: "stopped"
    }
  }
  mutating func begin() -> Bool { mon_capture_lifecycle(&value, 1) == 1 }
  mutating func connect() -> Bool { mon_capture_lifecycle(&value, 2) == 1 }
  mutating func interrupt(unavailable: Bool) -> Bool? {
    let result = mon_capture_lifecycle(&value, unavailable ? 4 : 3)
    return result < 0 ? nil : result == 1
  }
  mutating func stop() { _ = mon_capture_lifecycle(&value, 5) }
}

struct CaptureRawPrefix {
  private var value = mon_capture_prefix_empty()
  init() {}
  private init(value: MonRawPrefix) { self.value = value }
  var segments: Int { Int(value.segments) }
  var frames: Int64 { Int64(value.frames) }
  var metadataBytes: Int { Int(value.metadata_bytes) }
  var clockMaps: Int { Int(value.clock_maps) }
  var hash: String? {
    var input = value
    var bytes = [UInt8](repeating: 0, count: 64)
    let valid = bytes.withUnsafeMutableBufferPointer {
      mon_capture_prefix_hash(&input, $0.baseAddress, $0.count)
    }
    return valid ? String(bytes: bytes, encoding: .utf8) : nil
  }
  func prepare(_ receipt: Data) throws -> (prefix: CaptureRawPrefix, tip: Data) {
    var input = value
    var next = mon_capture_prefix_empty()
    var tip = [UInt8](repeating: 0, count: 1024)
    let count = receipt.withUnsafeBytes { bytes in
      tip.withUnsafeMutableBufferPointer { output in
        mon_capture_prepare_commit(
          &input, bytes.bindMemory(to: UInt8.self).baseAddress,
          bytes.count, &next, output.baseAddress, output.count)
      }
    }
    guard count > 0 else {
      throw ProbeError.capture("raw segment commit does not extend the accepted prefix")
    }
    return (CaptureRawPrefix(value: next), Data(tip.prefix(count)))
  }
  func tip(owner: String, lane: String, closed: Bool) throws -> Data {
    var input = value
    var tip = [UInt8](repeating: 0, count: 1024)
    let count = Array(owner.utf8).withUnsafeBufferPointer { owner in
      Array(lane.utf8).withUnsafeBufferPointer { lane in
        tip.withUnsafeMutableBufferPointer { output in
          mon_capture_prefix_tip(
            &input, owner.baseAddress, owner.count,
            lane.baseAddress, lane.count, closed, output.baseAddress, output.count)
        }
      }
    }
    guard count > 0 else { throw ProbeError.capture("raw segment prefix identity is invalid") }
    return Data(tip.prefix(count))
  }
}
