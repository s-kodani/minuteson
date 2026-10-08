import Darwin
import Foundation

/// Validate a lease passed by the durable native owner, without acquiring a
/// different lock or treating parent death as proof that capture has stopped.
/// The inherited descriptor is deliberately process-lifetime, not ARC-scoped.
enum CaptureTestLease {
  static func requireInherited(descriptor: Int32, output: URL, owner: String) throws {
    guard descriptor >= 3, InputLevelReport.validOwner(owner),
      output.standardizedFileURL == output.resolvingSymlinksInPath().standardizedFileURL,
      output.lastPathComponent == "audio-\(owner)"
    else { throw ProbeError.capture("capture test lease target is invalid") }
    var inherited = stat()
    guard fstat(descriptor, &inherited) == 0, validLease(inherited) else {
      throw ProbeError.capture("capture test inherited lease is not a private empty regular file")
    }
    let root = output.deletingLastPathComponent()
    let rootFD = open(root.path, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)
    guard rootFD >= 0 else { throw ProbeError.capture("capture test lease root is unavailable") }
    defer { close(rootFD) }
    var rootInfo = stat()
    guard fstat(rootFD, &rootInfo) == 0, privateDirectory(rootInfo) else {
      throw ProbeError.capture("capture test lease root is not private")
    }
    let outputFD = openat(
      rootFD, output.lastPathComponent, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)
    guard outputFD >= 0 else { throw ProbeError.capture("capture test output is unavailable") }
    defer { close(outputFD) }
    var outputInfo = stat()
    guard fstat(outputFD, &outputInfo) == 0, privateDirectory(outputInfo) else {
      throw ProbeError.capture("capture test output is not private")
    }
    let checkFD = openat(rootFD, "lease-\(owner)", O_RDONLY | O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC)
    guard checkFD >= 0 else { throw ProbeError.capture("capture test lease path is unavailable") }
    defer { close(checkFD) }
    var actual = stat()
    guard fstat(checkFD, &actual) == 0, validLease(actual), sameFile(inherited, actual) else {
      throw ProbeError.capture("capture test inherited lease does not match its test")
    }
    // A separate description must not acquire the already-held inherited
    // flock. Closing this check descriptor never unlocks the shared owner.
    let acquired = flock(checkFD, LOCK_EX | LOCK_NB)
    guard acquired == -1, errno == EWOULDBLOCK else {
      throw ProbeError.capture("capture test inherited lease is not held")
    }
    let flags = fcntl(descriptor, F_GETFD)
    guard flags >= 0, fcntl(descriptor, F_SETFD, flags | FD_CLOEXEC) == 0 else {
      throw ProbeError.capture("capture test inherited lease cannot be retained safely")
    }
    // Do not close(descriptor) or LOCK_UN here, in a defer, or on failure.
    // Both would allow collection before the capture process actually exits.
  }

  private static func privateDirectory(_ value: stat) -> Bool {
    value.st_mode & S_IFMT == S_IFDIR && value.st_uid == geteuid()
      && value.st_mode & 0o077 == 0
  }

  private static func validLease(_ value: stat) -> Bool {
    value.st_mode & S_IFMT == S_IFREG && value.st_uid == geteuid()
      && value.st_mode & 0o077 == 0 && value.st_nlink == 1 && value.st_size == 0
  }

  private static func sameFile(_ left: stat, _ right: stat) -> Bool {
    left.st_dev == right.st_dev && left.st_ino == right.st_ino
      && left.st_birthtimespec.tv_sec == right.st_birthtimespec.tv_sec
      && left.st_birthtimespec.tv_nsec == right.st_birthtimespec.tv_nsec
  }
}
