import CoreMedia
import Darwin
import Foundation

struct MachTimeUnits: Sendable {
  let numerator: UInt32
  let denominator: UInt32

  func nanoseconds(_ ticks: UInt64) throws -> Int64 {
    guard numerator > 0, denominator > 0 else {
      throw ProbeError.capture("Mach timebase is invalid")
    }
    let product = ticks.multipliedFullWidth(by: UInt64(numerator))
    let divisor = UInt64(denominator)
    guard product.high < divisor else {
      throw ProbeError.capture("Mach time conversion overflows")
    }
    let result = divisor.dividingFullWidth(product).quotient
    guard result <= UInt64(Int64.max) else {
      throw ProbeError.capture("Mach time is outside signed precision bounds")
    }
    return Int64(result)
  }
}

struct CaptureClockReading: Sendable {
  let hostBeforeNs: Int64
  let continuousNs: Int64
  let utcMs: Int64
  let hostAfterNs: Int64

  func offsetBounds() throws -> ClosedRange<Int64> {
    guard hostBeforeNs >= 0, hostAfterNs >= hostBeforeNs, continuousNs >= 0,
      (0...8_640_000_000_000_000).contains(utcMs),
      hostAfterNs < Int64.max - 2, continuousNs < Int64.max - 2
    else { throw ProbeError.capture("capture clock calibration is invalid") }
    return (continuousNs - hostAfterNs - 2)...(continuousNs - hostBeforeNs + 2)
  }
}

struct CaptureClockMetadata: Encodable {
  let domain = "mach_continuous_time"
  let originUtcMs: Int64
  let uncertaintyNs: Int64
}

enum CaptureInput: String, Sendable { case microphone, system }

struct CaptureClosure: Equatable {
  fileprivate let clockID: UUID
  fileprivate let input: CaptureInput
  fileprivate let nanoseconds: Int64
}

struct CaptureEndMetadata: Encodable {
  let method = "acquisition_closed"
  let microphoneClosedUs: Int64
  let systemClosedUs: Int64
  let endUs: Int64
}

enum CaptureClockObservation: Sendable {
  case timestamp(Int64)
  case invalid(String)
  // No PCM may be written while the control plane drains both acquisitions.
  case suspended
}

struct CaptureClockRecoveryRequest: Equatable, Sendable {
  fileprivate let clockID: UUID
  fileprivate let id: UUID
}

struct CaptureClockQuiescence: Equatable, Sendable {
  fileprivate let request: CaptureClockRecoveryRequest
  fileprivate let input: CaptureInput
}

/// Bridge hardware host time to a shared sleep-aware clock. Recovery is opt-in
/// and requires explicit control-plane proofs after both acquisitions drain.
/// A bridge must never change underneath queued audio or an open acquisition.
final class CaptureClockDomain: @unchecked Sendable {
  private enum ReadingError: Error { case suspended }
  private struct PendingRecovery {
    let request: CaptureClockRecoveryRequest
    let reading: CaptureClockReading
    let bounds: ClosedRange<Int64>
    var inputs: [String: CaptureClockQuiescence] = [:]
  }
  private let units: MachTimeUnits
  private let read: @Sendable () throws -> CaptureClockReading
  private let reference: CaptureClockReading
  private var epochBounds: ClosedRange<Int64>
  private var epochHostFloor: Int64
  private var offset: Int64
  private let allowsRecovery: Bool
  private var pendingRecovery: PendingRecovery?
  private let uncertaintyNs: Int64
  private let lock = NSLock()
  private var failure: String?
  private let clockID = UUID()
  private var closures: [String: CaptureClosure] = [:]
  private var lastReading: CaptureClockReading

  init(
    units: MachTimeUnits, allowsRecovery: Bool = false,
    read: @escaping @Sendable () throws -> CaptureClockReading
  ) throws {
    self.units = units
    self.read = read
    self.allowsRecovery = allowsRecovery
    reference = try read()
    lastReading = reference
    epochHostFloor = reference.hostBeforeNs
    epochBounds = try reference.offsetBounds()
    let width = try Self.calibrationWidth(reference)
    offset = epochBounds.lowerBound + width / 2
    // Includes the host bracket and millisecond wall-time quantization.
    uncertaintyNs = width + 1_000_000
  }

  private static func calibrationWidth(_ reading: CaptureClockReading) throws -> Int64 {
    let (width, overflow) = (reading.hostAfterNs - reading.hostBeforeNs)
      .addingReportingOverflow(4)
    guard !overflow, width <= 9_007_199_253_740_991 else {
      throw ProbeError.capture("capture clock calibration uncertainty is too large")
    }
    return width
  }

  static func live(allowsRecovery: Bool = false) throws -> CaptureClockDomain {
    var info = mach_timebase_info_data_t()
    guard mach_timebase_info(&info) == KERN_SUCCESS else {
      throw ProbeError.capture("cannot read Mach timebase")
    }
    let units = MachTimeUnits(numerator: info.numer, denominator: info.denom)
    return try CaptureClockDomain(units: units, allowsRecovery: allowsRecovery) {
      let before = try units.nanoseconds(mach_absolute_time())
      let continuous = try units.nanoseconds(mach_continuous_time())
      let milliseconds = (Date().timeIntervalSince1970 * 1000).rounded(.down)
      let after = try units.nanoseconds(mach_absolute_time())
      guard milliseconds.isFinite, milliseconds >= 0, milliseconds <= 8_640_000_000_000_000
      else { throw ProbeError.capture("capture UTC reference is invalid") }
      return CaptureClockReading(
        hostBeforeNs: before, continuousNs: continuous, utcMs: Int64(milliseconds),
        hostAfterNs: after)
    }
  }

  private func checkedReading() throws -> CaptureClockReading {
    if let failure { throw ProbeError.capture(failure) }
    if pendingRecovery != nil { throw ReadingError.suspended }
    let current = try read()
    let bounds = try current.offsetBounds()
    _ = try Self.calibrationWidth(current)
    guard current.hostBeforeNs >= lastReading.hostBeforeNs,
      current.continuousNs >= lastReading.continuousNs
    else {
      throw ProbeError.capture("capture clock moved backwards")
    }
    if !bounds.overlaps(epochBounds) {
      guard allowsRecovery, closures.isEmpty, bounds.lowerBound > epochBounds.upperBound else {
        throw ProbeError.capture("capture clock bridge changed; explicit recovery is required")
      }
      pendingRecovery = PendingRecovery(
        request: CaptureClockRecoveryRequest(clockID: clockID, id: UUID()),
        reading: current, bounds: bounds)
      throw ReadingError.suspended
    }
    lastReading = current
    return current
  }

  /// Poll even when no PCM arrives. A positive bridge discontinuity is evidence
  /// for a new epoch, not evidence that either input has actually stopped.
  func recoveryRequest() throws -> CaptureClockRecoveryRequest? {
    try lock.withLock {
      do {
        if let failure { throw ProbeError.capture(failure) }
        if let pendingRecovery { return pendingRecovery.request }
        _ = try checkedReading()
        return nil
      } catch ReadingError.suspended {
        return pendingRecovery?.request
      } catch {
        failure = failure ?? String(describing: error)
        throw error
      }
    }
  }

  /// Only the serialized acquisition owner calls this, after invalidating its
  /// binding, stopping/detaching the source and draining admitted callbacks.
  /// Receipt identity prevents mixing clocks, inputs or separate sleep cycles;
  /// it is not independent evidence that an OS stop succeeded.
  func quiesced(_ input: CaptureInput, for request: CaptureClockRecoveryRequest) throws
    -> CaptureClockQuiescence
  {
    try lock.withLock {
      guard failure == nil, closures.isEmpty, request.clockID == clockID,
        pendingRecovery?.request == request
      else { throw ProbeError.capture("capture clock quiescence is obsolete") }
      if let existing = pendingRecovery?.inputs[input.rawValue] { return existing }
      let receipt = CaptureClockQuiescence(request: request, input: input)
      pendingRecovery?.inputs[input.rawValue] = receipt
      return receipt
    }
  }

  func resume(
    _ request: CaptureClockRecoveryRequest,
    microphone: CaptureClockQuiescence, system: CaptureClockQuiescence
  ) throws {
    try lock.withLock {
      guard failure == nil, closures.isEmpty, let pending = pendingRecovery,
        request.clockID == clockID, pending.request == request,
        microphone.request == request, microphone.input == .microphone,
        system.request == request, system.input == .system,
        pending.inputs[CaptureInput.microphone.rawValue] == microphone,
        pending.inputs[CaptureInput.system.rawValue] == system
      else { throw ProbeError.capture("both inputs must quiesce in the requested clock epoch") }
      do {
        let current = try read()
        let bounds = try current.offsetBounds()
        let width = try Self.calibrationWidth(current)
        guard current.hostBeforeNs >= pending.reading.hostBeforeNs,
          current.continuousNs >= pending.reading.continuousNs,
          bounds.overlaps(pending.bounds)
        else { throw ProbeError.capture("capture clock changed again during quiescence") }
        // No timestamp from the retired epoch can be admitted into this one.
        epochHostFloor = current.hostAfterNs
        epochBounds = bounds
        offset = bounds.lowerBound + width / 2
        lastReading = current
        pendingRecovery = nil
      } catch {
        failure = failure ?? String(describing: error)
        throw error
      }
    }
  }

  private func requireNotSuspended() throws {
    guard pendingRecovery == nil else {
      throw ProbeError.capture("capture clock bridge changed; explicit recovery is required")
    }
  }

  // Call after this acquisition has stopped and callbacks have drained, before
  // CAF finalization. A caller cannot construct or refresh this clock-bound proof.
  func close(_ input: CaptureInput) throws -> CaptureClosure {
    try lock.withLock {
      do {
        if let failure { throw ProbeError.capture(failure) }
        if let existing = closures[input.rawValue] { return existing }
        try requireNotSuspended()
        let reading = try checkedReading()
        let closure = CaptureClosure(
          clockID: clockID, input: input, nanoseconds: reading.continuousNs)
        closures[input.rawValue] = closure
        return closure
      } catch {
        failure = failure ?? String(describing: error)
        throw error
      }
    }
  }

  func ending(
    originNanoseconds origin: Int64, microphone: CaptureClosure, system: CaptureClosure
  ) throws -> (CaptureClockMetadata, CaptureEndMetadata) {
    try lock.withLock {
      guard failure == nil, pendingRecovery == nil,
        microphone.clockID == clockID, system.clockID == clockID,
        microphone.input == .microphone, system.input == .system,
        closures[CaptureInput.microphone.rawValue] == microphone,
        closures[CaptureInput.system.rawValue] == system,
        origin >= 0, origin < microphone.nanoseconds, origin < system.nanoseconds
      else { throw ProbeError.capture("both acquisitions must close in the same proven clock") }
      let mic = (microphone.nanoseconds - origin) / 1000
      let pc = (system.nanoseconds - origin) / 1000
      guard mic > 0, pc > 0, max(mic, pc) <= 9_007_199_254_740_991 else {
        throw ProbeError.capture("capture ending is outside supported bounds")
      }
      // Freeze acquisition time, excluding disk flush, verification and later sleep.
      return (
        try originMetadata(origin),
        CaptureEndMetadata(microphoneClosedUs: mic, systemClosedUs: pc, endUs: max(mic, pc))
      )
    }
  }

  private func originMetadata(_ origin: Int64) throws -> CaptureClockMetadata {
    guard origin >= 0 else { throw ProbeError.capture("capture origin is invalid") }
    let delta = origin - reference.continuousNs
    let milliseconds = delta / 1_000_000 - (delta < 0 && delta % 1_000_000 != 0 ? 1 : 0)
    let (utc, overflow) = reference.utcMs.addingReportingOverflow(milliseconds)
    guard !overflow, (0...8_640_000_000_000_000).contains(utc) else {
      throw ProbeError.capture("capture origin UTC is outside supported bounds")
    }
    return CaptureClockMetadata(originUtcMs: utc, uncertaintyNs: uncertaintyNs)
  }

  func observe(hostTicks: UInt64?) -> CaptureClockObservation {
    lock.withLock {
      do {
        guard closures.count < 2 else {
          throw ProbeError.capture("capture observation arrived after both acquisitions closed")
        }
        if let failure { throw ProbeError.capture(failure) }
        if pendingRecovery != nil { throw ReadingError.suspended }
        guard let hostTicks else { throw ProbeError.capture("capture host timestamp is missing") }
        let current = try checkedReading()
        let host = try units.nanoseconds(hostTicks)
        guard host >= epochHostFloor, host <= current.hostAfterNs else {
          throw ProbeError.capture("capture timestamp is outside the proven clock epoch")
        }
        let (capture, overflow) = host.addingReportingOverflow(offset)
        guard !overflow, capture >= 0 else {
          throw ProbeError.capture("capture timestamp conversion overflows")
        }
        return .timestamp(capture)
      } catch ReadingError.suspended {
        return .suspended
      } catch {
        let message = String(describing: error)
        failure = failure ?? message
        return .invalid(message)
      }
    }
  }

  func observe(presentation: CMTime, clock: CMClock?) -> CaptureClockObservation {
    guard let clock else { return .invalid("ScreenCaptureKit synchronization clock is missing") }
    let host = CMSyncConvertTime(presentation, from: clock, to: CMClockGetHostTimeClock())
    guard host.isNumeric, host.epoch == 0, host.value >= 0 else {
      return .invalid("ScreenCaptureKit host time conversion is invalid")
    }
    return observe(hostTicks: CMClockConvertHostTimeToSystemUnits(host))
  }

  func metadata(originNanoseconds origin: Int64) throws -> CaptureClockMetadata {
    try lock.withLock {
      do {
        _ = try checkedReading()
        return try originMetadata(origin)
      } catch ReadingError.suspended {
        throw ProbeError.capture("capture clock metadata cannot be published during recovery")
      } catch {
        failure = failure ?? String(describing: error)
        throw error
      }
    }
  }
}

struct CaptureTrackTiming {
  private var map: CapturedClockMap
  private var previousSourceEnd: Int64?
  private var previousCaptureEnd: Int64?
  private var sourceEpoch: UInt64?
  var firstCaptureNanoseconds: Int64? { map.firstCaptureNanoseconds }
  var needsSegmentRollover: Bool { map.needsSegmentRollover }
  func finalizedEndpoints() throws -> [CapturedClockEndpoints] { try map.finalizedEndpoints() }

  init(sampleRate: Double, maximumSegments: Int = 1024) throws {
    guard sampleRate.isFinite, sampleRate.rounded() == sampleRate,
      (8000...384000).contains(sampleRate)
    else { throw ProbeError.capture("capture clock sample rate is invalid") }
    map = try CapturedClockMap(sampleRate: Int64(sampleRate), maximumSegments: maximumSegments)
  }

  mutating func recordSaved(
    frames: Int64, observation: CaptureClockObservation, sourceFrame: Int64?,
    sourceEpoch epoch: UInt64 = 0, captureEndNanoseconds: Int64? = nil
  ) throws {
    guard case .timestamp(let start) = observation else {
      if case .invalid(let message) = observation { throw ProbeError.capture(message) }
      throw ProbeError.capture("capture timestamp is missing")
    }
    guard frames > 0, frames <= Int64.max / 1_000_000_000 else {
      throw ProbeError.capture("capture buffer length is invalid")
    }
    let duration = frames * 1_000_000_000 / map.sampleRate
    let (nominalEnd, overflow) = start.addingReportingOverflow(duration)
    let end = captureEndNanoseconds ?? nominalEnd
    guard !overflow, end > start else { throw ProbeError.capture("capture buffer end overflows") }
    guard epoch <= 9_007_199_254_740_991,
      sourceEpoch.map({ epoch >= $0 }) ?? true
    else { throw ProbeError.capture("capture source epoch is reversed or outside bounds") }
    var discontinuity = false
    if let previousCaptureEnd {
      if let sourceEpoch, epoch != sourceEpoch {
        // Only the control path that binds a verified input may advance this
        // epoch. An engine's reset sample counter cannot erase elapsed time.
        guard start >= previousCaptureEnd else {
          throw ProbeError.capture("rebound input overlaps previously saved capture time")
        }
        discontinuity = start > previousCaptureEnd
      } else if let previousSourceEnd, let sourceFrame {
        guard sourceFrame >= previousSourceEnd else {
          throw ProbeError.capture("source frame counter restarted; explicit recovery is required")
        }
        discontinuity = sourceFrame > previousSourceEnd
      } else {
        // Timestamp-only sources have no independent frame counter. Preserve
        // measured holes larger than one source frame; do not use arrival time.
        discontinuity =
          start > previousCaptureEnd
          && start - previousCaptureEnd > map.maximumErrorNanoseconds
      }
    }
    var sourceEnd: Int64?
    if let sourceFrame {
      let (value, overflow) = sourceFrame.addingReportingOverflow(frames)
      guard !overflow else { throw ProbeError.capture("source frame counter overflows") }
      sourceEnd = value
    }
    try map.append(
      frames: frames, captureStartNanoseconds: start, captureEndNanoseconds: end,
      discontinuity: discontinuity)
    previousSourceEnd = sourceEnd
    previousCaptureEnd = end
    sourceEpoch = epoch
  }

  mutating func finish(originNanoseconds: Int64, expectedFrames: Int64) throws
    -> [CapturedClockSegment]
  {
    guard map.frames == expectedFrames else {
      throw ProbeError.capture("capture clock does not cover every saved frame")
    }
    return try map.finish(originNanoseconds: originNanoseconds)
  }

  /// Check the next callback using the same source/epoch/gap rules as ordinary
  /// capture. Its measured start seals a continuous boundary; a real hole keeps
  /// the old nominal end. This does not append the new PCM to the old segment.
  mutating func finishBeforeNext(
    frames: Int64, observation: CaptureClockObservation, sourceFrame: Int64?,
    sourceEpoch epoch: UInt64, originNanoseconds: Int64, expectedFrames: Int64
  ) throws -> [CapturedClockSegment] {
    guard map.frames == expectedFrames, case .timestamp(let start) = observation,
      let previousCaptureEnd
    else { throw ProbeError.capture("raw segment boundary lacks saved capture evidence") }
    var validated = self
    try validated.recordSaved(
      frames: frames, observation: observation, sourceFrame: sourceFrame, sourceEpoch: epoch)
    let gap: Bool
    if let sourceEpoch, sourceEpoch != epoch {
      gap = start > previousCaptureEnd
    } else if let previousSourceEnd, let sourceFrame {
      gap = sourceFrame > previousSourceEnd
    } else {
      gap = start > previousCaptureEnd && start - previousCaptureEnd > map.maximumErrorNanoseconds
    }
    return try map.finish(
      originNanoseconds: originNanoseconds, measuredEndNanoseconds: gap ? nil : start)
  }
}
