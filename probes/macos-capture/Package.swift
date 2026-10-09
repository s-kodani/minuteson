// swift-tools-version: 6.2
import PackageDescription

let package = Package(
  name: "mon-macos-capture",
  platforms: [.macOS(.v15)],
  products: [.executable(name: "mon-macos-capture", targets: ["MONMacOSCaptureCLI"])],
  targets: [
    .target(
      name: "MONMacOSCapture",
      linkerSettings: [
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
