import Foundation

struct CapturedClockSegment: Encodable, Equatable {
  let sourceFrameStart: Int64
  let sourceFrameEnd: Int64
  let timelineStartUs: Int64
  let timelineEndUs: Int64
  let mappingJson = "{\"schema_version\":1,\"method\":\"linear\"}"
}

struct CapturedClockEndpoints: Encodable, Equatable {
  let startNanoseconds: Int64
  let endNanoseconds: Int64
}

/// Compress capture-time observations, never callback-arrival times. Callers
/// must supply timestamps in one shared, sleep-aware monotonic clock domain.
/// Each append represents successfully stored PCM frames. An explicit missing
/// interval remains a gap; it is not represented by fabricated input frames.
struct CapturedClockMap {
  static let maximumTrackSegments = 65_536
  var needsSegmentRollover: Bool { segments.count >= maximumSegments - 2 }
  private struct Point {
    let frame: Int64
    let time: Int64
  }

  private struct Segment {
    let first: Point
    let last: Point
  }

  let sampleRate: Int64
  let maximumErrorNanoseconds: Int64
  private let maximumSegments: Int
  private var segments: [Segment] = []
  private var anchor: Point?
  private var last: Point?
  private var pendingEnd: Point?
  private var minimumSlope = -Double.infinity
  private var maximumSlope = Double.infinity
  private var sealed = false
  private var invalid = false
  private(set) var frames: Int64 = 0
  var firstCaptureNanoseconds: Int64? { segments.first?.first.time ?? anchor?.time }

  func finalizedEndpoints() throws -> [CapturedClockEndpoints] {
    guard sealed, !invalid else { throw ProbeError.capture("capture clock is not finalized") }
    return segments.map {
      CapturedClockEndpoints(startNanoseconds: $0.first.time, endNanoseconds: $0.last.time)
    }
  }

  init(sampleRate: Int64, maximumSegments: Int = 1024) throws {
    guard (8000...384000).contains(sampleRate),
      (1...Self.maximumTrackSegments).contains(maximumSegments)
    else {
      throw ProbeError.capture("capture clock format or capacity is invalid")
    }
    self.sampleRate = sampleRate
    self.maximumSegments = maximumSegments
    // A compression bound, not an empirically established synchronization limit.
    self.maximumErrorNanoseconds = (1_000_000_000 + sampleRate - 1) / sampleRate
  }

  mutating func append(
    frames count: Int64, captureStartNanoseconds start: Int64,
    captureEndNanoseconds end: Int64, discontinuity: Bool = false
  ) throws {
    guard !sealed, !invalid else {
      throw ProbeError.capture("capture clock is sealed or invalid")
    }
    do {
      let (next, overflow) = frames.addingReportingOverflow(count)
      guard count > 0, !overflow, next <= 9_007_199_254_740_991,
        start >= 0, end > start
      else {
        throw ProbeError.capture("capture clock frame or time bounds are invalid")
      }
      let point = Point(frame: frames, time: start)
      if let pendingEnd {
        if discontinuity {
          guard start > pendingEnd.time else {
            throw ProbeError.capture("capture discontinuity has no positive missing interval")
          }
          try accept(pendingEnd)
          try emit()
          reset(point)
        } else {
          // For continuous source frames, the next capture timestamp is the
          // measured boundary. The previous nominal buffer end is not an
          // independent hardware-clock observation and must not imply a gap.
          try accept(point)
        }
      } else {
        reset(point)
      }
      // The pending buffer still needs a final segment. Refuse before its PCM
      // is written rather than discovering there is no slot only at stop.
      guard segments.count < maximumSegments else {
        throw ProbeError.capture("capture clock segment capacity exceeded")
      }
      pendingEnd = Point(frame: next, time: end)
      frames = next
    } catch {
      invalid = true
      throw error
    }
  }

  mutating func finish(originNanoseconds origin: Int64, measuredEndNanoseconds: Int64? = nil)
    throws -> [CapturedClockSegment]
  {
    guard !sealed, !invalid, let pendingEnd, let first = firstCaptureNanoseconds,
      origin >= 0, origin <= first
    else {
      throw ProbeError.capture("capture clock cannot be finalized")
    }
    sealed = true
    do {
      // A live segment is sealed only after the next continuous callback has
      // supplied its measured boundary. Never rewrite a published endpoint.
      let endpoint = Point(frame: pendingEnd.frame, time: measuredEndNanoseconds ?? pendingEnd.time)
      try accept(endpoint)
      try emit()
      return try segments.map { segment in
        let firstUs = (segment.first.time - origin) / 1000
        let lastUs = (segment.last.time - origin) / 1000
        guard lastUs > firstUs else {
          throw ProbeError.capture("capture clock interval disappears at microsecond precision")
        }
        let scale =
          Double(segment.last.frame - segment.first.frame) * 1_000_000
          / (Double(lastUs - firstUs) * Double(sampleRate))
        guard (1.0 / 64.0...64.0).contains(scale) else {
          throw ProbeError.capture("capture clock transform is outside the supported range")
        }
        return CapturedClockSegment(
          sourceFrameStart: segment.first.frame, sourceFrameEnd: segment.last.frame,
          timelineStartUs: firstUs, timelineEndUs: lastUs)
      }
    } catch {
      invalid = true
      throw error
    }
  }

  private mutating func reset(_ point: Point) {
    anchor = point
    last = point
    minimumSlope = -Double.infinity
    maximumSlope = Double.infinity
  }

  private mutating func emit() throws {
    guard let anchor, let last, last.frame > anchor.frame, last.time > anchor.time else {
      throw ProbeError.capture("capture clock segment has no physical frames")
    }
    guard segments.count < maximumSegments else {
      throw ProbeError.capture("capture clock segment capacity exceeded")
    }
    segments.append(Segment(first: anchor, last: last))
  }

  private mutating func accept(_ point: Point) throws {
    guard let anchor, let last, point.frame > last.frame, point.time > last.time,
      point.time - anchor.time <= 9_007_199_254_740_991
    else {
      throw ProbeError.capture(
        "capture clock observations are reversed or outside precision bounds")
    }
    let frameDelta = Double(point.frame - anchor.frame)
    let timeDelta = Double(point.time - anchor.time)
    let slope = timeDelta / frameDelta
    let low = max(minimumSlope, (timeDelta - Double(maximumErrorNanoseconds)) / frameDelta)
    let high = min(maximumSlope, (timeDelta + Double(maximumErrorNanoseconds)) / frameDelta)
    if low <= slope && slope <= high {
      minimumSlope = low
      maximumSlope = high
      self.last = point
      return
    }
    // Keep an observed endpoint, not an invented best-fit timestamp. All prior
    // observations constrain the accepted slope interval. Two points plus two
    // slope bounds suffice regardless of how many callbacks the run contains.
    guard last.frame > anchor.frame else {
      throw ProbeError.capture("capture clock observations cannot form a linear segment")
    }
    try emit()
    reset(last)
    try accept(point)
  }
}
