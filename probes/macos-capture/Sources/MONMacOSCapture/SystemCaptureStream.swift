import CoreMedia
import Foundation
import ScreenCaptureKit

/// The same owned lifecycle is exercised by silent, controlled test sources.
/// Acquisition implementations share the same owned recovery and PCM writer.
protocol SystemCaptureStream: AnyObject, Sendable {
  var native: SCStream? { get }
  var synchronizationClock: CMClock? { get }
  var applicationName: String? { get }
  var bundleIdentifier: String? { get }
  func start() async throws
  func stop() async throws
  func requestStop(_ completion: @escaping @Sendable (Error?) -> Void)
  func detach() throws
  func afterAcceptedPCM(_ observation: @escaping @Sendable () -> Void)
}

extension SystemCaptureStream {
  var synchronizationClock: CMClock? { native?.synchronizationClock }
  // Controlled sources deliver synchronously; queued native sources override.
  func afterAcceptedPCM(_ observation: @escaping @Sendable () -> Void) { observation() }
}

final class ScreenCaptureStream: SystemCaptureStream, @unchecked Sendable {
  let native: SCStream?
  let applicationName: String?
  let bundleIdentifier: String?
  private let output: any SCStreamOutput
  private let sampleHandlerQueue: DispatchQueue

  init(
    stream: SCStream, output: any SCStreamOutput, applicationName: String?,
    bundleIdentifier: String?, sampleHandlerQueue: DispatchQueue
  ) {
    native = stream
    self.output = output
    self.applicationName = applicationName
    self.bundleIdentifier = bundleIdentifier
    self.sampleHandlerQueue = sampleHandlerQueue
  }

  func start() async throws { try await native!.startCapture() }
  func stop() async throws { try await native!.stopCapture() }
  func requestStop(_ completion: @escaping @Sendable (Error?) -> Void) {
    native!.stopCapture(completionHandler: completion)
  }
  func detach() throws { try native!.removeStreamOutput(output, type: .audio) }
  func afterAcceptedPCM(_ observation: @escaping @Sendable () -> Void) {
    sampleHandlerQueue.async(execute: observation)
  }
}
