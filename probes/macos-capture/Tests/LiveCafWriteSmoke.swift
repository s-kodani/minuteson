// Synthetic PCM only. Keep AVAudioFile open while copying its live CAF bytes.
import AVFoundation
import Foundation

guard CommandLine.arguments.count == 3 else {
  fatalError("provide a fresh output directory and a finalized synthetic CAF fixture directory")
}
let output = URL(fileURLWithPath: CommandLine.arguments[1], isDirectory: true)
let finalized = URL(fileURLWithPath: CommandLine.arguments[2], isDirectory: true)
let live = output.appendingPathComponent("live.caf")
guard !FileManager.default.fileExists(atPath: live.path) else {
  fatalError("fixture already exists")
}
let format = AVAudioFormat(standardFormatWithSampleRate: 48_000, channels: 2)!
let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 4_800)!
buffer.frameLength = 4_800
for channel in 0..<2 {
  for frame in 0..<4_800 {
    buffer.floatChannelData![channel][frame] = channel == 0 ? 0.25 : -0.5
  }
}
let file = try AVAudioFile(
  forWriting: live, settings: format.settings, commonFormat: .pcmFormatFloat32,
  interleaved: false)
try file.write(from: buffer)
try withExtendedLifetime(file) {
  let bytes = try Data(contentsOf: live)
  try bytes.write(to: output.appendingPathComponent("source.caf"), options: .withoutOverwriting)
  print("Live CAF snapshot contains \(bytes.count) bytes")
}
try FileManager.default.copyItem(
  at: finalized.appendingPathComponent("result.caf"),
  to: output.appendingPathComponent("result.caf"))
print("4800 synthetic frames submitted; writer retained during snapshot")
