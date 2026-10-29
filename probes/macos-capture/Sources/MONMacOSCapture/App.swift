import AVFoundation
import AudioToolbox
import CoreAudio
import CoreMedia
import Darwin
import Foundation
import ScreenCaptureKit

/// The executable owns @main; importing the core must not import a C entry
/// point into an optimized test runner. Preserve async-main actor isolation.
public enum CaptureCommand {
  @MainActor
  public static func run() async {
    do {
      let arguments = Array(CommandLine.arguments.dropFirst())
      if arguments.first == "--import-audio" {
        try await AudioImporter.command(arguments)
        return
      }
      if arguments == ["--list-apps"] {
        try await listApps()
        return
      }
      if arguments == ["--list-audio-processes"] {
        try listAudioProcesses()
        return
      }
      if arguments == ["--list-input-devices"] {
        let data = try JSONEncoder().encode(InputDevices.live())
        guard data.count <= 256 * 1024 else {
          throw ProbeError.capture("input device report is too large")
        }
        print(String(decoding: data, as: UTF8.self))
        return
      }
      if arguments == ["--list-capture-applications"] {
        let data = try JSONEncoder().encode(CaptureApplications.live())
        guard data.count <= 256 * 1024 else {
          throw ProbeError.capture("application report is too large")
        }
        print(String(decoding: data, as: UTF8.self))
        return
      }
      let options = try Options.parse(arguments)
      try await CaptureSession.run(options)
    } catch ProbeError.help {
      print(usage)
    } catch {
      FileHandle.standardError.write(Data("error: \(error)\n".utf8))
      FileHandle.standardError.write(Data(usage.utf8))
      Foundation.exit(2)
    }
  }

  private static let usage = """
    Usage: mon-macos-capture --list-apps
           mon-macos-capture --list-capture-applications
           mon-macos-capture --list-audio-processes
           mon-macos-capture --list-input-devices
           mon-macos-capture --output <directory> [--duration <seconds>]
             [--scenario <id>] [--mode system|application] [--pid <process-id> ...]
             [--backend screencapturekit|coreaudio] [--stop-on-file] [--parent-pid <pid>]
             [--until-stopped] [--microphone-uid <stable-device-uid>] [--application-identity <json>]
             [--input-level-owner <session-uuid>]
             [--five-second-test <test-uuid>]
             [--capture-test-lease-fd <inherited-descriptor>]
    --until-stopped requires --stop-on-file and --parent-pid, without --duration.
    --five-second-test requires --stop-on-file, --parent-pid and --microphone-uid.
    Its shared 5-second capture window opens only after both inputs initialize.
    A supplied capture-test lease must be inherited, private, locked and bound to that test's output.
    Records microphone.caf and system.caf after the required macOS permissions are granted.
    """

  private struct ListedApplication: Encodable {
    let processID: pid_t
    let name: String
    let bundleIdentifier: String
  }

  private static func listApps() async throws {
    let content = try await SCShareableContent.excludingDesktopWindows(
      false, onScreenWindowsOnly: false)
    let applications = content.applications.map {
      ListedApplication(
        processID: $0.processID, name: $0.applicationName,
        bundleIdentifier: $0.bundleIdentifier)
    }.sorted { $0.name.localizedCaseInsensitiveCompare($1.name) == .orderedAscending }
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
    let data = try encoder.encode(applications)
    print(String(decoding: data, as: UTF8.self))
  }

  private struct ListedAudioProcess: Encodable {
    let processID: pid_t
    let audioObjectID: AudioObjectID
    let name: String?
    let bundleIdentifier: String?
  }

  private static func listAudioProcesses() throws {
    let processes = try TapAudioProcesses.list().map { item in
      let application = NSRunningApplication(processIdentifier: item.pid)
      return ListedAudioProcess(
        processID: item.pid, audioObjectID: item.objectID,
        name: application?.localizedName, bundleIdentifier: application?.bundleIdentifier)
    }
    let encoder = JSONEncoder()
    encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
    print(String(decoding: try encoder.encode(processes), as: UTF8.self))
  }

}
