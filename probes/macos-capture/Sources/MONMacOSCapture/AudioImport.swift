import AVFoundation
import AudioToolbox
import Darwin
import Foundation

struct ImportedAudioInfo: Codable, Equatable {
  let schemaVersion: Int
  let detectedFormat: String
  let codec: String
  let sampleRate: UInt32
  let channels: UInt32
  let frames: UInt64
  let durationUs: Int64
}

enum AudioImporter {
  // Inspect ISO BMFF track handlers without asking AVURLAsset to infer a
  // container from the filename. Skip payloads (including mdat/artwork).
  static func checkM4ATracks(_ handle: FileHandle) throws {
    let length = try handle.seekToEnd()
    var boxes = 0
    var moov = 0
    struct Track {
      var id: UInt64?
      var handler: String?
      var media = false
      var references = false
      var chapters: [UInt64] = []
    }
    var tracks: [Track] = []
    func uint(_ bytes: Data) -> UInt64 { bytes.reduce(0) { ($0 << 8) | UInt64($1) } }
    func read(_ offset: UInt64, _ count: Int) throws -> Data {
      try handle.seek(toOffset: offset)
      let data = try handle.read(upToCount: count) ?? Data()
      guard data.count == count else { throw ProbeError.usage("truncated M4A box") }
      return data
    }
    func walk(_ first: UInt64, _ end: UInt64, _ path: [String], _ track: Int? = nil) throws {
      var offset = first
      while offset < end {
        boxes += 1
        guard boxes <= 4096, end - offset >= 8 else {
          throw ProbeError.usage("invalid or excessive M4A boxes")
        }
        let header = try read(offset, 8)
        let type = String(data: header.suffix(4), encoding: .ascii) ?? ""
        var size = uint(header.prefix(4))
        var headerSize: UInt64 = 8
        if size == 1 {
          guard end - offset >= 16 else { throw ProbeError.usage("truncated M4A size") }
          size = uint(try read(offset + 8, 8))
          headerSize = 16
        } else if size == 0 {
          size = end - offset
        }
        guard size >= headerSize, size <= end - offset else {
          throw ProbeError.usage("invalid M4A box size")
        }
        let body = offset + headerSize
        let count = size - headerSize
        if path.isEmpty && type == "moov" {
          moov += 1
          try walk(body, offset + size, ["moov"])
        } else if path == ["moov"] && type == "trak" {
          guard tracks.count < 128 else { throw ProbeError.usage("excessive M4A tracks") }
          tracks.append(Track())
          try walk(body, offset + size, ["trak"], tracks.count - 1)
        } else if let track {
          if path == ["trak"] && type == "tkhd" {
            guard tracks[track].id == nil, count >= 4 else {
              throw ProbeError.usage("invalid M4A track ID")
            }
            let version = try read(body, 1)[0]
            let idOffset: UInt64 = version == 0 ? 12 : 20
            guard version <= 1, count >= idOffset + 4 else {
              throw ProbeError.usage("truncated M4A track ID")
            }
            let id = uint(try read(body + idOffset, 4))
            guard id > 0 else { throw ProbeError.usage("invalid M4A track ID") }
            tracks[track].id = id
          } else if path == ["trak"] && type == "mdia" {
            guard !tracks[track].media else { throw ProbeError.usage("duplicate M4A media") }
            tracks[track].media = true
            try walk(body, offset + size, ["mdia"], track)
          } else if path == ["mdia"] && type == "hdlr" {
            guard count >= 12, tracks[track].handler == nil else {
              throw ProbeError.usage("invalid M4A handler")
            }
            tracks[track].handler = String(data: try read(body + 8, 4), encoding: .ascii)
          } else if path == ["trak"] && type == "tref" {
            guard !tracks[track].references else {
              throw ProbeError.usage("duplicate M4A references")
            }
            tracks[track].references = true
            try walk(body, offset + size, ["tref"], track)
          } else if path == ["tref"] {
            guard type == "chap", tracks[track].chapters.isEmpty,
              count > 0, count % 4 == 0, count <= 128 * 4
            else { throw ProbeError.usage("invalid M4A chapter references") }
            for position in stride(from: UInt64(0), to: count, by: 4) {
              tracks[track].chapters.append(uint(try read(body + position, 4)))
            }
          }
        }
        offset += size
      }
    }
    try walk(0, length, [])
    let ids = tracks.compactMap(\.id)
    let audio = tracks.filter { $0.handler == "soun" }
    guard moov == 1, !tracks.isEmpty, ids.count == tracks.count,
      Set(ids).count == ids.count, audio.count == 1,
      tracks.allSatisfy({ $0.handler == "soun" || $0.handler == "text" }),
      let sound = audio.first
    else { throw ProbeError.usage("M4A requires one audio track and no video or unknown tracks") }
    let chapters = tracks.filter { $0.handler == "text" }
    guard Set(sound.chapters).count == sound.chapters.count,
      Set(sound.chapters) == Set(chapters.compactMap(\.id)),
      chapters.allSatisfy({ !$0.references })
    else { throw ProbeError.usage("M4A text tracks require valid audio chapter references") }
  }
  // Files only. This does not request microphone, screen or recording power.
  static func decode(
    input: URL, output: URL?, parentPID: pid_t? = nil,
    cancelled: () -> Bool = { false }
  ) async throws -> ImportedAudioInfo {
    guard input.isFileURL else { throw ProbeError.usage("local audio is required") }
    let handle = try FileHandle(forReadingFrom: input)
    defer { try? handle.close() }
    let header = try handle.read(upToCount: 12) ?? Data()
    // Content-derived type hint: do not let an unrelated filename extension
    // select a different decoder or make a valid audio file unreadable.
    let hint: AudioFileTypeID
    if header.starts(with: Data("fLaC".utf8)) {
      hint = kAudioFileFLACType
    } else if header.count >= 12, header.prefix(4) == Data("RIFF".utf8),
      header.suffix(4) == Data("WAVE".utf8)
    {
      hint = kAudioFileWAVEType
    } else if header.count >= 12, header.subdata(in: 4..<8) == Data("ftyp".utf8) {
      hint = kAudioFileM4AType
    } else if header.starts(with: Data("ID3".utf8))
      || (header.count >= 2 && header[0] == 0xff && header[1] & 0xe0 == 0xe0)
    {
      hint = kAudioFileMP3Type
    } else {
      throw ProbeError.usage("supported audio is PCM WAV, MP3, AAC/ALAC M4A or FLAC")
    }
    func check(_ status: OSStatus, _ operation: String) throws {
      if status != noErr { throw ProbeError.capture("\(operation) failed: \(status)") }
    }
    var opened: AudioFileID?
    try check(AudioFileOpenURL(input as CFURL, .readPermission, hint, &opened), "open audio")
    guard let opened else { throw ProbeError.capture("audio handle is missing") }
    defer { AudioFileClose(opened) }
    var source: ExtAudioFileRef?
    try check(ExtAudioFileWrapAudioFileID(opened, false, &source), "wrap audio")
    guard let source else { throw ProbeError.capture("decoder handle is missing") }
    defer { ExtAudioFileDispose(source) }
    var description = AudioStreamBasicDescription()
    var size = UInt32(MemoryLayout<AudioStreamBasicDescription>.size)
    try check(
      ExtAudioFileGetProperty(source, kExtAudioFileProperty_FileDataFormat, &size, &description),
      "read format")
    let id = description.mFormatID
    let detected: String
    let codec: String
    if header.starts(with: Data("fLaC".utf8)), id == kAudioFormatFLAC {
      detected = "flac"
      codec = "flac"
    } else if header.count >= 12, header.prefix(4) == Data("RIFF".utf8),
      header.suffix(4) == Data("WAVE".utf8), id == kAudioFormatLinearPCM
    {
      detected = "wav"
      codec = "pcm"
    } else if id == kAudioFormatMPEGLayer3 {
      detected = "mp3"
      codec = "mp3"
    } else if header.count >= 12, header.subdata(in: 4..<8) == Data("ftyp".utf8),
      [kAudioFormatMPEG4AAC, kAudioFormatAppleLossless].contains(id)
    {
      try checkM4ATracks(handle)
      detected = "m4a"
      codec = id == kAudioFormatAppleLossless ? "alac" : "aac"
    } else {
      throw ProbeError.usage("supported audio is PCM WAV, MP3, AAC/ALAC M4A or FLAC")
    }
    let rate = description.mSampleRate
    let channels = description.mChannelsPerFrame
    var declaredFrames: Int64 = 0
    size = UInt32(MemoryLayout<Int64>.size)
    try check(
      ExtAudioFileGetProperty(
        source, kExtAudioFileProperty_FileLengthFrames, &size, &declaredFrames), "read frame count")
    guard rate.isFinite, rate.rounded() == rate, (8_000...384_000).contains(rate),
      (1...8).contains(channels), declaredFrames > 0,
      Double(declaredFrames) <= rate * 14_400,
      let format = AVAudioFormat(
        commonFormat: .pcmFormatFloat32, sampleRate: rate, channels: channels, interleaved: true),
      let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: 4096)
    else { throw ProbeError.usage("audio format or duration is outside supported bounds") }
    var client = format.streamDescription.pointee
    try check(
      ExtAudioFileSetProperty(
        source, kExtAudioFileProperty_ClientDataFormat,
        UInt32(MemoryLayout<AudioStreamBasicDescription>.size), &client), "configure PCM")
    var destination: AVAudioFile?
    if let output {
      guard output.isFileURL, output.standardizedFileURL != input.standardizedFileURL else {
        throw ProbeError.usage("a separate local CAF destination is required")
      }
      let descriptor = open(
        output.path, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW, S_IRUSR | S_IWUSR)
      guard descriptor >= 0 else {
        throw ProbeError.capture("CAF destination already exists or is unavailable")
      }
      close(descriptor)
      destination = try AVAudioFile(
        forWriting: output,
        settings: [
          AVFormatIDKey: kAudioFormatLinearPCM, AVSampleRateKey: rate,
          AVNumberOfChannelsKey: channels, AVLinearPCMBitDepthKey: 32,
          AVLinearPCMIsFloatKey: true, AVLinearPCMIsBigEndianKey: false,
          AVLinearPCMIsNonInterleaved: false,
        ], commonFormat: .pcmFormatFloat32, interleaved: true)
    }
    var frames: UInt64 = 0
    while true {
      if cancelled() || (parentPID != nil && getppid() != parentPID) {
        throw ProbeError.capture("audio import was cancelled or its parent exited")
      }
      buffer.frameLength = 4096
      var decodedFrames: UInt32 = 4096
      try check(
        ExtAudioFileRead(source, &decodedFrames, buffer.mutableAudioBufferList), "decode audio")
      buffer.frameLength = decodedFrames
      if decodedFrames == 0 { break }
      guard let samples = buffer.floatChannelData else {
        throw ProbeError.capture("decoded PCM is unavailable")
      }
      for index in 0..<(Int(buffer.frameLength) * Int(channels)) {
        guard samples[0][index].isFinite else {
          throw ProbeError.capture("decoded PCM contains a non-finite sample")
        }
      }
      frames += UInt64(buffer.frameLength)
      guard frames <= UInt64(rate) * 14_400 else {
        throw ProbeError.usage("audio must be no longer than four hours")
      }
      try destination?.write(from: buffer)
    }
    guard frames > 0, frames == UInt64(declaredFrames) else {
      throw ProbeError.capture("decoded frame count does not match the audio file")
    }
    destination = nil  // Close/finalize before publishing the report.
    return ImportedAudioInfo(
      schemaVersion: 1, detectedFormat: detected, codec: codec,
      sampleRate: UInt32(rate), channels: channels, frames: frames,
      durationUs: Int64(frames * 1_000_000 / UInt64(rate)))
  }

  static func command(_ arguments: [String]) async throws {
    guard arguments.count == 4, let parent = pid_t(arguments[3]), parent > 0 else {
      throw ProbeError.usage("--import-audio requires input, CAF output or -, and parent PID")
    }
    do {
      let info = try await decode(
        input: URL(fileURLWithPath: arguments[1]),
        output: arguments[2] == "-" ? nil : URL(fileURLWithPath: arguments[2]),
        parentPID: parent)
      FileHandle.standardOutput.write(try JSONEncoder().encode(info))
    } catch {
      let report = ["errorMessage": String(describing: error)]
      FileHandle.standardOutput.write(try JSONEncoder().encode(report))
      throw error
    }
  }
}
