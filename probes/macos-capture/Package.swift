// swift-tools-version: 6.2
import PackageDescription

let package = Package(
  name: "mon-macos-capture",
  platforms: [.macOS(.v15)],
  products: [.executable(name: "mon-macos-capture", targets: ["MONMacOSCapture"])],
  targets: [
    .executableTarget(
      name: "MONMacOSCapture",
      linkerSettings: [
        .linkedFramework("AVFoundation"),
        .linkedFramework("CoreAudio"),
        .linkedFramework("CoreMedia"),
        .linkedFramework("IOKit"),
        .linkedFramework("ScreenCaptureKit"),
      ]
    ),
    .testTarget(
      name: "MONMacOSCaptureTests", dependencies: ["MONMacOSCapture"],
      path: "Tests/MONMacOSCaptureTests"
    ),
  ]
)
