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
    var audio = 0
    var video = 0
    func uint(_ bytes: Data) -> UInt64 {
      bytes.reduce(0) { ($0 << 8) | UInt64($1) }
    }
    func read(_ offset: UInt64, _ count: Int) throws -> Data {
      try handle.seek(toOffset: offset)
      let data = try handle.read(upToCount: count) ?? Data()
      guard data.count == count else { throw ProbeError.usage("truncated M4A box") }
      return data
    }
    func walk(_ first: UInt64, _ end: UInt64, _ path: [String]) throws {
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
        if path.isEmpty && type == "moov" {
          moov += 1
          try walk(offset + headerSize, offset + size, ["moov"])
        } else if path == ["moov"] && type == "trak" {
          try walk(offset + headerSize, offset + size, ["moov", "trak"])
        } else if path == ["moov", "trak"] && type == "mdia" {
          try walk(offset + headerSize, offset + size, ["moov", "trak", "mdia"])
        } else if path == ["moov", "trak", "mdia"] && type == "hdlr" {
          guard size - headerSize >= 12 else { throw ProbeError.usage("truncated M4A handler") }
          let handler = try read(offset + headerSize + 8, 4)
          if handler == Data("soun".utf8) { audio += 1 }
          if handler == Data("vide".utf8) { video += 1 }
        }
        offset += size
      }
    }
    try walk(0, length, [])
    guard moov == 1, audio == 1, video == 0 else {
      throw ProbeError.usage("M4A requires one audio track and no video")
    }
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
