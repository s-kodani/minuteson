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
}

extension SystemCaptureStream {
  var synchronizationClock: CMClock? { native?.synchronizationClock }
}

final class ScreenCaptureStream: SystemCaptureStream, @unchecked Sendable {
  let native: SCStream?
  let applicationName: String?
  let bundleIdentifier: String?
  private let output: any SCStreamOutput

  init(
    stream: SCStream, output: any SCStreamOutput, applicationName: String?,
    bundleIdentifier: String?
  ) {
    native = stream
    self.output = output
    self.applicationName = applicationName
    self.bundleIdentifier = bundleIdentifier
  }

  func start() async throws { try await native!.startCapture() }
  func stop() async throws { try await native!.stopCapture() }
  func requestStop(_ completion: @escaping @Sendable (Error?) -> Void) {
    native!.stopCapture(completionHandler: completion)
  }
  func detach() throws { try native!.removeStreamOutput(output, type: .audio) }
}
