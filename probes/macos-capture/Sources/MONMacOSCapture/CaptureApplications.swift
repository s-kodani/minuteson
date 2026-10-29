import AppKit
import Darwin
import Foundation

struct CaptureApplicationIdentity: Codable, Equatable, Sendable {
  let pid: pid_t
  let bundleIdentifier: String
  let launchTimeUs: Int64

  var valid: Bool {
    pid > 0 && InputDevices.validUID(bundleIdentifier)
      && launchTimeUs > 0 && launchTimeUs <= 9_007_199_254_740_991
  }
}

struct CaptureApplication: Encodable, Equatable {
  let pid: pid_t
  let bundleIdentifier: String
  let launchTimeUs: Int64
  let name: String
  var identity: CaptureApplicationIdentity {
    CaptureApplicationIdentity(
      pid: pid, bundleIdentifier: bundleIdentifier, launchTimeUs: launchTimeUs)
  }
}

struct CaptureApplicationsReport: Encodable {
  let schemaVersion = 1
  let systemPermission: String
  let applications: [CaptureApplication]
}

enum CaptureApplications {
  static func startTime(seconds: UInt64, microseconds: UInt64) throws -> Int64 {
    let (scaled, multiplyOverflow) = seconds.multipliedReportingOverflow(by: 1_000_000)
    let (value, addOverflow) = scaled.addingReportingOverflow(microseconds)
    guard microseconds < 1_000_000, !multiplyOverflow, !addOverflow,
      value > 0, value <= 9_007_199_254_740_991
    else { throw ProbeError.capture("application start time is invalid") }
    return Int64(value)
  }

  // Kernel process start time avoids treating a reused PID as the same app.
  // This reads metadata only and neither requests permission nor starts capture.
  static func processStart(_ pid: pid_t) throws -> Int64 {
    guard pid > 0 else { throw ProbeError.capture("application PID is invalid") }
    var info = proc_bsdinfo()
    let size = Int32(MemoryLayout<proc_bsdinfo>.stride)
    let read = proc_pidinfo(pid, PROC_PIDTBSDINFO, 0, &info, size)
    guard read == size, info.pbi_pid == UInt32(pid) else {
      throw ProbeError.capture("selected application is unavailable")
    }
    return try startTime(seconds: info.pbi_start_tvsec, microseconds: info.pbi_start_tvusec)
  }

  static func require(
    _ identity: CaptureApplicationIdentity, from applications: [CaptureApplication]
  ) throws {
    guard identity.valid, applications.filter({ $0.identity == identity }).count == 1 else {
      throw ProbeError.capture(
        "selected application exited or restarted; no global fallback was used")
    }
  }

  static func current(_ identity: CaptureApplicationIdentity) throws {
    guard identity.valid,
      try processStart(identity.pid) == identity.launchTimeUs,
      let app = NSRunningApplication(processIdentifier: identity.pid),
      !app.isTerminated, app.bundleIdentifier == identity.bundleIdentifier,
      try processStart(identity.pid) == identity.launchTimeUs
    else { throw ProbeError.capture("selected application identity changed; no fallback was used") }
  }

  static func live() throws -> CaptureApplicationsReport {
    var rows: [CaptureApplication] = []
    for app in NSWorkspace.shared.runningApplications {
      guard !app.isTerminated, app.activationPolicy != .prohibited,
        app.processIdentifier > 0, app.processIdentifier != getpid(),
        app.processIdentifier != getppid(),
        let bundle = app.bundleIdentifier, InputDevices.validUID(bundle),
        let name = app.localizedName, !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
        name.utf8.count <= 1024,
        let start = try? processStart(app.processIdentifier)
      else { continue }
      let identity = CaptureApplicationIdentity(
        pid: app.processIdentifier, bundleIdentifier: bundle, launchTimeUs: start)
      guard (try? current(identity)) != nil else { continue }
      rows.append(
        CaptureApplication(
          pid: identity.pid, bundleIdentifier: bundle, launchTimeUs: start, name: name))
    }
    guard rows.count <= 128, Set(rows.map(\.pid)).count == rows.count else {
      throw ProbeError.capture("application catalog exceeds capacity or has ambiguous identities")
    }
    return CaptureApplicationsReport(
      // Screen permission is not evidence about audio-only tap permission.
      // No public audio-only preflight is used; explicit capture validates PCM.
      systemPermission: "unknown",
      applications: try CaptureCore.catalogOrder(rows, applications: true).map { rows[$0] })
  }
}
