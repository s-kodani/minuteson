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
      guard fstat(descriptor, &expected) == 0, Self.regular(expected) else {
        throw ProbeError.capture("input event journal file is invalid")
      }
      try storageIO.write(initial, descriptor)
      if synchronizes {
        try storageIO.synchronize(descriptor)
        try storageIO.synchronize(directory)
      }
      guard fstat(descriptor, &expected) == 0, Self.regular(expected) else {
        throw ProbeError.capture("input event journal initial identity is unavailable")
      }
    } catch {
      close(directory)
      directory = -1
      throw error
    }
  }

  deinit { if directory >= 0 { close(directory) } }

  /// Only the ephemeral diagnostic leaf may be removed. Pinning also keeps a
  /// replacement ancestor outside this cleanup; an altered leaf is retained.
  @discardableResult
  func removeEphemeralIfOwned() -> Bool {
    guard leaf == .status else { return false }
    do { try requireExpected() } catch { return false }
    return unlinkat(directory, leaf.rawValue, 0) == 0
  }

  func replace(_ data: Data) throws {
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
        Self.same(owned, actual), Self.regular(actual)
      {
        _ = unlinkat(directory, temporary, 0)
      }
    }
    guard fstat(descriptor, &owned) == 0, Self.regular(owned) else {
      throw ProbeError.capture("input event journal staging identity is invalid")
    }
    try storageIO.write(data, descriptor)
    if synchronizes { try storageIO.synchronize(descriptor) }
    try requireExpected()
    var staged = stat()
    var current = stat()
    guard fstat(descriptor, &staged) == 0, Self.same(staged, owned), Self.regular(staged),
      fstatat(directory, temporary, &current, AT_SYMLINK_NOFOLLOW) == 0,
      Self.same(staged, current), Self.regular(current)
    else { throw ProbeError.capture("input event journal atomic replacement failed") }
    try storageIO.replace(directory, temporary, leaf.rawValue)
    renamed = true
    // Keep the identity even if directory sync fails after rename. A subsequent
    // attempt can commit the same owned leaf, but this attempt is not a receipt.
    guard fstat(descriptor, &expected) == 0, Self.same(expected, staged), Self.regular(expected)
    else {
      throw ProbeError.capture("input event journal committed identity is unavailable")
    }
    if synchronizes { try storageIO.synchronize(directory) }
    try requireExpected()
  }

  private func requireExpected() throws {
    var actual = stat()
    guard fstatat(directory, leaf.rawValue, &actual, AT_SYMLINK_NOFOLLOW) == 0,
      Self.regular(actual), Self.same(expected, actual), actual.st_size == expected.st_size,
      actual.st_mtimespec.tv_sec == expected.st_mtimespec.tv_sec,
      actual.st_mtimespec.tv_nsec == expected.st_mtimespec.tv_nsec,
      actual.st_ctimespec.tv_sec == expected.st_ctimespec.tv_sec,
      actual.st_ctimespec.tv_nsec == expected.st_ctimespec.tv_nsec
    else { throw ProbeError.capture("input event journal was replaced or modified") }
  }

  private static func regular(_ info: stat) -> Bool {
    info.st_mode & S_IFMT == S_IFREG && info.st_uid == geteuid()
      && info.st_nlink == 1 && info.st_mode & 0o077 == 0
      && info.st_size >= 0 && info.st_size <= 65 * 1024
  }

  private static func same(_ left: stat, _ right: stat) -> Bool {
    left.st_dev == right.st_dev && left.st_ino == right.st_ino
      && left.st_birthtimespec.tv_sec == right.st_birthtimespec.tv_sec
      && left.st_birthtimespec.tv_nsec == right.st_birthtimespec.tv_nsec
  }

  private static func write(_ data: Data, to descriptor: Int32) throws {
    guard !data.isEmpty, data.count <= 65 * 1024 else {
      throw ProbeError.capture("input event journal data is outside its limit")
    }
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
