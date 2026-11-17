import Foundation

/// One owned lane keeps every received frame when its hardware timestamps
/// regress. Previously emitted endpoints remain immutable. A persistent offset
/// belongs to one binding and is reset only when that binding is retired.
struct CapturePCMAlignment {
  struct Anomaly: Encodable, Equatable {
    let sourceEpoch: UInt64
    let frames: Int64
    let observedStartNanoseconds: Int64
    let mappedStartNanoseconds: Int64
  }
  struct Diagnostics: Encodable, Equatable {
    let timingEstimated = true
    var retimedPackets = 0
    var clockAdjustments = 0
    var correctionNanoseconds: Int64 = 0
    var firstAnomalies: [Anomaly] = []
    var truncated = false
  }
  private var epoch: UInt64?
  private var correction: Int64 = 0
  private var previousStart: Int64?
  private var previousEnd: Int64?
  private(set) var diagnostics: Diagnostics?

  func projectedStart(_ start: Int64, sourceEpoch: UInt64) throws -> Int64 {
    try Self.add(start, epoch == sourceEpoch ? correction : 0)
  }

  mutating func map(
    start: Int64, duration: Int64, frames: Int64, sourceEpoch: UInt64,
    emittedThrough: Int64?
  ) throws -> (start: Int64, end: Int64) {
    guard start >= 0, duration > 0, frames > 0, sourceEpoch <= InputEventHistory.safe,
      epoch.map({ sourceEpoch >= $0 }) ?? true
    else { throw ProbeError.capture("invalid PCM alignment input or retired source epoch") }
    let changed = epoch != nil && epoch != sourceEpoch
    if epoch != sourceEpoch { correction = 0 }
    var mapped = try Self.add(start, correction)
    if previousStart.map({ mapped <= $0 }) == true
      || emittedThrough.map({ mapped < $0 }) == true
      || (changed && previousEnd.map({ mapped < $0 }) == true)
    {
      let floor = max(previousEnd ?? mapped, emittedThrough ?? mapped)
      let adjustment = floor - mapped
      correction = try Self.add(correction, adjustment)
      mapped = floor
      var report = diagnostics ?? Diagnostics()
      report.clockAdjustments += 1
      report.correctionNanoseconds = try Self.add(report.correctionNanoseconds, adjustment)
      if report.firstAnomalies.count < 16 {
        report.firstAnomalies.append(
          Anomaly(
            sourceEpoch: sourceEpoch, frames: frames,
            observedStartNanoseconds: start, mappedStartNanoseconds: mapped))
      } else {
        report.truncated = true
      }
      diagnostics = report
    }
    let end = try Self.add(mapped, duration)
    if correction > 0 { diagnostics?.retimedPackets += 1 }
    epoch = sourceEpoch
    previousStart = mapped
    previousEnd = end
    return (mapped, end)
  }

  private static func add(_ value: Int64, _ increment: Int64) throws -> Int64 {
    let (result, overflow) = value.addingReportingOverflow(increment)
    guard !overflow, result >= 0 else { throw ProbeError.capture("PCM alignment clock overflow") }
    return result
  }
}
