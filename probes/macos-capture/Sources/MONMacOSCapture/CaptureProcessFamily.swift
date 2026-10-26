import Darwin
import Foundation

/// Kernel ancestry and creation times, never executable names or bundle guesses.
/// A verified descendant stays a member after reparenting until that exact
/// process exits. PID reuse cannot inherit membership.
struct CaptureProcess: Equatable, Sendable {
  let pid: pid_t
  let parentPID: pid_t
  let launchTimeUs: Int64

  static func read(_ pid: pid_t) throws -> CaptureProcess {
    var info = proc_bsdinfo()
    let size = Int32(MemoryLayout<proc_bsdinfo>.stride)
    guard pid > 0, proc_pidinfo(pid, PROC_PIDTBSDINFO, 0, &info, size) == size,
      info.pbi_pid == UInt32(pid), info.pbi_status != UInt32(SZOMB),
      let parent = pid_t(exactly: info.pbi_ppid)
    else { throw ProbeError.capture("application process is unavailable") }
    return CaptureProcess(
      pid: pid, parentPID: parent,
      launchTimeUs: try CaptureApplications.startTime(
        seconds: info.pbi_start_tvsec, microseconds: info.pbi_start_tvusec))
  }

  static func list() throws -> [CaptureProcess] {
    for _ in 0..<3 {
      let count = proc_listallpids(nil, 0)
      guard count > 0, count <= 16_384 else {
        throw ProbeError.capture("application process catalog exceeds capacity")
      }
      var pids = [pid_t](repeating: 0, count: min(Int(count) + 256, 16_384))
      let read = pids.withUnsafeMutableBytes {
        proc_listallpids($0.baseAddress, Int32($0.count))
      }
      guard read > 0 else { throw ProbeError.capture("application process catalog is unavailable") }
      if read >= pids.count { continue }
      // Processes can exit during enumeration. Missing metadata grants no scope.
      return pids.prefix(Int(read)).compactMap { try? Self.read($0) }
    }
    throw ProbeError.capture("application process catalog did not stabilize")
  }
}

final class CaptureProcessFamily: @unchecked Sendable {
  typealias Members = [pid_t: Int64]
  let root: CaptureApplicationIdentity
  private let lock = NSLock()
  private var retained: Members = [:]
  private let list: @Sendable () throws -> [CaptureProcess]
  private let read: @Sendable (pid_t) throws -> CaptureProcess
  private let requireRoot: @Sendable (CaptureApplicationIdentity) throws -> Void

  init(
    root: CaptureApplicationIdentity,
    list: @escaping @Sendable () throws -> [CaptureProcess] = CaptureProcess.list,
    read: @escaping @Sendable (pid_t) throws -> CaptureProcess = CaptureProcess.read,
    requireRoot: @escaping @Sendable (CaptureApplicationIdentity) throws -> Void =
      CaptureApplications.current
  ) {
    self.root = root
    self.list = list
    self.read = read
    self.requireRoot = requireRoot
  }

  func snapshot() throws -> Members {
    try lock.withLock {
      try requireRoot(root)
      let processes = try list()
      guard root.valid, processes.count <= 16_384,
        processes.allSatisfy({
          $0.pid > 0 && $0.parentPID >= 0 && $0.launchTimeUs > 0
            && $0.launchTimeUs <= 9_007_199_254_740_991
        }),
        Set(processes.map(\.pid)).count == processes.count
      else { throw ProbeError.capture("application process catalog is invalid or ambiguous") }
      let rows = Dictionary(uniqueKeysWithValues: processes.map { ($0.pid, $0) })
      guard rows[root.pid]?.launchTimeUs == root.launchTimeUs else {
        throw ProbeError.capture("selected application process identity changed")
      }
      var members = retained.filter { rows[$0.key]?.launchTimeUs == $0.value }
      members[root.pid] = root.launchTimeUs
      // Resolve descendants in bounded passes, independent of enumeration order.
      var children: [pid_t: [CaptureProcess]] = [:]
      for row in processes { children[row.parentPID, default: []].append(row) }
      var queue = Array(members.keys)
      var cursor = 0
      while cursor < queue.count {
        let parent = queue[cursor]
        cursor += 1
        for child in children[parent, default: []] {
          if members[child.pid] == nil, child.launchTimeUs >= members[parent]! {
            members[child.pid] = child.launchTimeUs
            queue.append(child.pid)
          }
        }
      }
      try verify(members)
      retained = members
      return members
    }
  }

  func verify(_ members: Members) throws {
    try requireRoot(root)
    guard members[root.pid] == root.launchTimeUs else {
      throw ProbeError.capture("application process family has another root")
    }
    for (pid, start) in members {
      let current = try read(pid)
      guard current.pid == pid, current.launchTimeUs == start else {
        throw ProbeError.capture("application descendant process identity changed")
      }
    }
    try requireRoot(root)
  }
}
