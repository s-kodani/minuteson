import Foundation
// swift-tools-version: 6.2
import PackageDescription

let captureCoreLibrary = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
  .appendingPathComponent(".build/capture-core/release").path

let package = Package(
  name: "mon-macos-capture",
  platforms: [.macOS(.v15)],
  products: [.executable(name: "mon-macos-capture", targets: ["MONMacOSCaptureCLI"])],
  targets: [
    .systemLibrary(name: "MONCaptureCore", path: "Sources/MONCaptureCore"),
    .target(
      name: "MONMacOSCapture",
      dependencies: ["MONCaptureCore"],
      linkerSettings: [
        .unsafeFlags(["-L", captureCoreLibrary]),
        .linkedLibrary("mon_capture_ffi"),
        .linkedFramework("AVFoundation"),
        .linkedFramework("CoreAudio"),
        .linkedFramework("CoreMedia"),
        .linkedFramework("IOKit"),
        .linkedFramework("ScreenCaptureKit"),
      ]
    ),
    .executableTarget(name: "MONMacOSCaptureCLI", dependencies: ["MONMacOSCapture"]),
    .testTarget(
      name: "MONMacOSCaptureTests", dependencies: ["MONMacOSCapture"],
      path: "Tests/MONMacOSCaptureTests"
    ),
  ]
)
