import AVFoundation
import AudioToolbox
import Foundation

let directory = FileManager.default.temporaryDirectory.appendingPathComponent(
  "mon-caf-smoke-\(UUID().uuidString)", isDirectory: true)
try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
defer { try? FileManager.default.removeItem(at: directory) }

let source = directory.appendingPathComponent("source.caf")
let result = directory.appendingPathComponent("result.caf")
let format = AVAudioFormat(standardFormatWithSampleRate: 48_000, channels: 2)!
let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 4_800)!
buffer.frameLength = 4_800
for channel in 0..<2 {
  for frame in 0..<4_800 {
    buffer.floatChannelData![channel][frame] =
      (Float(frame % 101) - 50) / 100 * Float(channel + 1) / 2
  }
}
var sourceSettings = format.settings
sourceSettings[AVLinearPCMIsNonInterleaved] = false
try AVAudioFile(
  forWriting: source, settings: sourceSettings, commonFormat: .pcmFormatFloat32,
  interleaved: false
).write(from: buffer)

let asset = AVURLAsset(url: source)
let tracks = try await asset.loadTracks(withMediaType: .audio)
let reader = try AVAssetReader(asset: asset)
let readerOutput = AVAssetReaderTrackOutput(track: tracks[0], outputSettings: nil)
reader.add(readerOutput)
let writer = try AVAssetWriter(outputURL: result, fileType: .caf)
let settings: [String: Any] = [
  AVFormatIDKey: kAudioFormatLinearPCM,
  AVSampleRateKey: 48_000,
  AVNumberOfChannelsKey: 2,
  AVLinearPCMBitDepthKey: 32,
  AVLinearPCMIsFloatKey: true,
  AVLinearPCMIsNonInterleaved: false,
  AVLinearPCMIsBigEndianKey: false,
]
guard writer.canApply(outputSettings: settings, forMediaType: .audio) else {
  fatalError("CAF PCM format is not supported")
}
let input = AVAssetWriterInput(mediaType: .audio, outputSettings: settings)
writer.add(input)
guard reader.startReading(), writer.startWriting() else {
  fatalError("reader or writer did not start")
}
writer.startSession(atSourceTime: .zero)
var frames = 0
while let sample = readerOutput.copyNextSampleBuffer() {
  if sample.numSamples == 0 { continue }
  while !input.isReadyForMoreMediaData { Thread.sleep(forTimeInterval: 0.005) }
  guard input.append(sample) else {
    fatalError("CAF append failed: \(String(describing: writer.error))")
  }
  frames += sample.numSamples
}
guard frames == 4_800 else { fatalError("source was truncated to \(frames) frames") }
writer.endSession(atSourceTime: CMTime(value: 4_800, timescale: 48_000))
input.markAsFinished()
await writer.finishWriting()
guard writer.status == .completed else {
  fatalError("CAF finalization failed: \(String(describing: writer.error))")
}
let readback = try AVAudioFile(forReading: result)
guard readback.length == 4_800 else {
  fatalError("CAF round-trip lost \(4_800 - readback.length) frames")
}
if CommandLine.arguments.count == 3, CommandLine.arguments[1] == "--preview-fixtures" {
  let destination = URL(fileURLWithPath: CommandLine.arguments[2], isDirectory: true)
  try FileManager.default.createDirectory(at: destination, withIntermediateDirectories: true)
  // copyItem refuses to overwrite an existing fixture.
  try FileManager.default.copyItem(at: source, to: destination.appendingPathComponent("source.caf"))
  try FileManager.default.copyItem(at: result, to: destination.appendingPathComponent("result.caf"))
}
print("CAF writer smoke test: 4800 of 4800 frames preserved")
