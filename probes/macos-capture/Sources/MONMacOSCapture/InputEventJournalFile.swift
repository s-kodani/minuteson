import Darwin
import Foundation

/// Descriptor-relative, exclusive creation followed by synced atomic replace.
/// An unexpected file/link is retained, not truncated, adopted or removed.
/// The pin never follows a later replacement of an ancestor directory.
final class InputEventJournalFile {
  // Internal fault injection only. Native operations always write and sync;
  // no application option replaces durability or the identity checks below.
  struct StorageIO: Sendable {
    var create: @Sendable (Int32, String) throws -> Int32 = { directory, name in
      let fd = openat(
        directory, name, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC, 0o600)
      guard fd >= 0 else { throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO) }
      return fd
    }
    var write: @Sendable (Data, Int32) throws -> Void = {
      try InputEventJournalFile.write($0, to: $1)
    }
    var synchronize: @Sendable (Int32) throws -> Void = {
      guard fsync($0) == 0 else { throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO) }
    }
    var replace: @Sendable (Int32, String, String) throws -> Void = { directory, source, target in
      guard renameat(directory, source, directory, target) == 0 else {
        throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO)
      }
    }
  }
  static let name = "input-events.json"
  enum Leaf: String {
    case journal = "input-events.json"
    case status = "input-journal-status.json"
    case rawMicrophone = "raw-microphone.json"
    case rawSystem = "raw-system.json"
    case singleWav = "single-wav.json"
    case singleWavOwner = "single-wav-owner.json"
    case captureManifest = "manifest.json"

    var maximumBytes: Int { self == .captureManifest ? 32 * 1024 * 1024 : 65 * 1024 }
  }
  private var directory: Int32
  private let leaf: Leaf
  private let synchronizes: Bool
  private let storageIO: StorageIO
  private var expected: stat

  init(
    output: URL, initial: Data, leaf: Leaf = .journal, synchronizes: Bool = true,
    storageIO: StorageIO = StorageIO()
  ) throws {
    try Self.requireData(initial, leaf: leaf)
    guard leaf == .status || synchronizes else {
      throw ProbeError.capture("retained input history requires synced writes")
    }
    self.leaf = leaf
    self.synchronizes = synchronizes
    self.storageIO = storageIO
    directory = try Self.pin(output)
    let descriptor: Int32
    do { descriptor = try storageIO.create(directory, leaf.rawValue) } catch {
      close(directory)
      directory = -1
      throw error
    }
    defer { close(descriptor) }
    expected = stat()
    do {
      guard fstat(descriptor, &expected) == 0, Self.regular(expected, leaf: leaf) else {
        throw ProbeError.capture("input event journal file is invalid")
      }
      try storageIO.write(initial, descriptor)
      if synchronizes {
        try storageIO.synchronize(descriptor)
        try storageIO.synchronize(directory)
      }
      guard fstat(descriptor, &expected) == 0, Self.regular(expected, leaf: leaf) else {
        throw ProbeError.capture("input event journal initial identity is unavailable")
      }
    } catch {
      close(directory)
      directory = -1
      throw error
    }
  }

  deinit { if directory >= 0 { close(directory) } }

  /// The WAV and its separately pinned evidence must share the same owned
  /// directory even if a pathname was replaced while the session opened.
  func requireDirectoryIdentity(_ descriptor: Int32) throws {
    var expected = stat()
    var actual = stat()
    guard fstat(descriptor, &expected) == 0, fstat(directory, &actual) == 0,
      Self.same(expected, actual)
    else { throw ProbeError.capture("WAV and receipt directories do not match") }
  }

  /// Only the ephemeral diagnostic leaf may be removed. Pinning also keeps a
  /// replacement ancestor outside this cleanup; an altered leaf is retained.
  @discardableResult
  func removeEphemeralIfOwned() -> Bool {
    guard leaf == .status else { return false }
    do { try requireExpected() } catch { return false }
    return unlinkat(directory, leaf.rawValue, 0) == 0
  }

  func replace(_ data: Data) throws {
    try Self.requireData(data, leaf: leaf)
    try requireExpected()
    let temporary = ".\(leaf.rawValue)-\(UUID().uuidString.lowercased()).tmp"
    let descriptor = try storageIO.create(directory, temporary)
    var owned = stat()
    var renamed = false
    defer {
      close(descriptor)
      // Remove only our exact staging inode, never an arbitrary matching name.
      var actual = stat()
      if !renamed, fstatat(directory, temporary, &actual, AT_SYMLINK_NOFOLLOW) == 0,
        Self.same(owned, actual), Self.regular(actual, leaf: leaf)
      {
        _ = unlinkat(directory, temporary, 0)
      }
    }
    guard fstat(descriptor, &owned) == 0, Self.regular(owned, leaf: leaf) else {
      throw ProbeError.capture("input event journal staging identity is invalid")
    }
    try storageIO.write(data, descriptor)
    if synchronizes { try storageIO.synchronize(descriptor) }
    try requireExpected()
    var staged = stat()
    var current = stat()
    guard fstat(descriptor, &staged) == 0, Self.same(staged, owned),
      Self.regular(staged, leaf: leaf),
      fstatat(directory, temporary, &current, AT_SYMLINK_NOFOLLOW) == 0,
      Self.same(staged, current), Self.regular(current, leaf: leaf)
    else { throw ProbeError.capture("input event journal atomic replacement failed") }
    try storageIO.replace(directory, temporary, leaf.rawValue)
    renamed = true
    // Keep the identity even if directory sync fails after rename. A subsequent
    // attempt can commit the same owned leaf, but this attempt is not a receipt.
    guard fstat(descriptor, &expected) == 0, Self.same(expected, staged),
      Self.regular(expected, leaf: leaf)
    else {
      throw ProbeError.capture("input event journal committed identity is unavailable")
    }
    if synchronizes { try storageIO.synchronize(directory) }
    try requireExpected()
  }

  /// Verify the exact receipt in the pinned directory without following a
  /// replacement ancestor or accepting an independently-created pathname.
  func verifyCurrent(_ data: Data) throws {
    try requireExpected()
    let descriptor = openat(directory, leaf.rawValue, O_RDONLY | O_NOFOLLOW | O_CLOEXEC)
    guard descriptor >= 0 else { throw POSIXError(POSIXErrorCode(rawValue: errno) ?? .EIO) }
    defer { close(descriptor) }
    var identity = stat()
    guard fstat(descriptor, &identity) == 0, Self.same(identity, expected),
      identity.st_size == Int64(data.count)
    else { throw ProbeError.capture("input receipt identity changed") }
    var actual = Data(count: data.count)
    try actual.withUnsafeMutableBytes { bytes in
      var offset = 0
      while offset < data.count {
        let count = Darwin.read(
          descriptor, bytes.baseAddress!.advanced(by: offset), data.count - offset)
        if count < 0 && errno == EINTR { continue }
        guard count > 0 else { throw ProbeError.capture("input receipt readback failed") }
        offset += count
      }
    }
    guard actual == data else {
      throw ProbeError.capture("input receipt differs from committed prefix")
    }
    try requireExpected()
  }

  private func requireExpected() throws {
    var actual = stat()
    guard fstatat(directory, leaf.rawValue, &actual, AT_SYMLINK_NOFOLLOW) == 0,
      Self.regular(actual, leaf: leaf), Self.same(expected, actual),
      actual.st_size == expected.st_size,
      actual.st_mtimespec.tv_sec == expected.st_mtimespec.tv_sec,
      actual.st_mtimespec.tv_nsec == expected.st_mtimespec.tv_nsec,
      actual.st_ctimespec.tv_sec == expected.st_ctimespec.tv_sec,
      actual.st_ctimespec.tv_nsec == expected.st_ctimespec.tv_nsec
    else { throw ProbeError.capture("input event journal was replaced or modified") }
  }

  private static func regular(_ info: stat, leaf: Leaf) -> Bool {
    info.st_mode & S_IFMT == S_IFREG && info.st_uid == geteuid()
      && info.st_nlink == 1 && info.st_mode & 0o077 == 0
      && info.st_size >= 0 && info.st_size <= Int64(leaf.maximumBytes)
  }

  private static func same(_ left: stat, _ right: stat) -> Bool {
    left.st_dev == right.st_dev && left.st_ino == right.st_ino
      && left.st_birthtimespec.tv_sec == right.st_birthtimespec.tv_sec
      && left.st_birthtimespec.tv_nsec == right.st_birthtimespec.tv_nsec
  }

  private static func requireData(_ data: Data, leaf: Leaf) throws {
    guard !data.isEmpty, data.count <= leaf.maximumBytes else {
      throw ProbeError.capture("input event journal data is outside its limit")
    }
  }

  private static func write(_ data: Data, to descriptor: Int32) throws {
    try data.withUnsafeBytes { buffer in
      guard let base = buffer.baseAddress else { throw ProbeError.capture("empty journal data") }
      var offset = 0
      while offset < data.count {
        let count = Darwin.write(descriptor, base.advanced(by: offset), data.count - offset)
        if count < 0 && errno == EINTR { continue }
        guard count > 0 else {
          throw POSIXError(POSIXErrorCode(rawValue: count == 0 ? EIO : errno) ?? .EIO)
        }
        offset += count
      }
    }
  }

  static func pin(_ output: URL) throws -> Int32 {
    guard let canonical = realpath(output.path, nil) else {
      throw ProbeError.capture("input event journal directory is unavailable")
    }
    defer { free(canonical) }
    guard output.isFileURL, output.path.hasPrefix("/"),
      String(cString: canonical) == output.path
    else { throw ProbeError.capture("input event journal directory is not canonical") }
    var parent = open("/", O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)
    guard parent >= 0 else { throw ProbeError.capture("input event journal root is unavailable") }
    for component in output.pathComponents.dropFirst() {
      let child = openat(parent, component, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)
      close(parent)
      guard child >= 0 else {
        throw ProbeError.capture("input event journal directory is unavailable")
      }
      parent = child
    }
    var info = stat()
    guard fstat(parent, &info) == 0, info.st_mode & S_IFMT == S_IFDIR,
      info.st_uid == geteuid()
    else {
      close(parent)
      throw ProbeError.capture("input event journal directory owner is invalid")
    }
    return parent
  }
}
