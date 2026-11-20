import AVFoundation
import Foundation

/// Check the finalized physical file, not only that a capture callback delivered samples.
/// A failed check leaves the original audio available for explicit recovery.
func verifyRecordedCAF(_ url: URL, expected: TrackStats) throws {
  guard expected.frames > 0, expected.appendFailures == 0,
    let sampleRate = expected.sampleRate, sampleRate.isFinite,
    (8000...384000).contains(sampleRate), sampleRate.rounded() == sampleRate,
    let channels = expected.channels, (1...8).contains(channels)
  else {
    throw ProbeError.capture("recorded track statistics are invalid: \(url.lastPathComponent)")
  }
  let handle = try FileHandle(forReadingFrom: url)
  defer { try? handle.close() }
  guard try handle.read(upToCount: 4) == Data("caff".utf8) else {
    throw ProbeError.capture("recorded file is not CAF: \(url.lastPathComponent)")
  }
  let file = try AVAudioFile(forReading: url)
  guard file.length == expected.frames,
    file.fileFormat.sampleRate == sampleRate,
    file.fileFormat.channelCount == channels
  else {
    throw ProbeError.capture(
      "recorded CAF differs from captured frames or format: \(url.lastPathComponent)")
  }
}

func verifyRecordedWAV(_ url: URL, expected: TrackStats) throws {
  guard expected.frames > 0, expected.appendFailures == 0,
    expected.sampleRate == 16000, expected.channels == 1
  else { throw ProbeError.capture("recorded WAV statistics are invalid") }
  let handle = try FileHandle(forReadingFrom: url)
  defer { try? handle.close() }
  let magic = try handle.read(upToCount: 4)
  guard magic == Data("RIFF".utf8) || magic == Data("RF64".utf8) else {
    throw ProbeError.capture("recorded file is not WAV")
  }
  let file = try AVAudioFile(forReading: url)
  guard file.length == expected.frames, file.fileFormat.sampleRate == 16000,
    file.fileFormat.channelCount == 1
  else { throw ProbeError.capture("recorded WAV differs from normalized capture") }
}
