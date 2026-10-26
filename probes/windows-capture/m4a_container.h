#pragma once
#include <cstdint>
#include <filesystem>
#include <fstream>
#include <functional>
#include <optional>
#include <set>
#include <stdexcept>
#include <vector>

// Pure container validation shared by the native decoder and portable tests.
// Only the sound track is decoded; referenced text chapters carry no PCM.
namespace m4a_container {
inline void validate(const std::filesystem::path& path) {
  std::ifstream input(path, std::ios::binary);
  if (!input) throw std::runtime_error("cannot open M4A");
  const auto size = std::filesystem::file_size(path);
  std::size_t boxes = 0, moov = 0;
  struct Track {
    std::optional<std::uint64_t> id, handler;
    bool media = false, references = false;
    std::vector<std::uint64_t> chapters;
  };
  std::vector<Track> tracks;
  auto fail = [] { throw std::runtime_error("invalid M4A audio/chapter track structure"); };
  auto uint_at = [&](std::uint64_t offset, int count) {
    unsigned char bytes[8] = {};
    input.seekg(static_cast<std::streamoff>(offset));
    input.read(reinterpret_cast<char*>(bytes), count);
    if (!input) fail();
    std::uint64_t value = 0;
    for (int i = 0; i < count; ++i) value = (value << 8) | bytes[i];
    return value;
  };
  // 0 root, 1 moov, 2 trak, 3 mdia, 4 tref. No arbitrary recursion.
  std::function<void(std::uint64_t, std::uint64_t, int, std::size_t)> walk;
  walk = [&](std::uint64_t first, std::uint64_t end, int depth, std::size_t index) {
    for (auto at = first; at < end;) {
      if (++boxes > 4096 || end - at < 8) fail();
      auto length = uint_at(at, 4), type = uint_at(at + 4, 4);
      std::uint64_t header = 8;
      if (length == 1) {
        if (end - at < 16) fail();
        length = uint_at(at + 8, 8); header = 16;
      } else if (length == 0) length = end - at;
      if (length < header || length > end - at) fail();
      const auto body = at + header, count = length - header;
      if (depth == 0 && type == 0x6d6f6f76) { ++moov; walk(body, at + length, 1, 0); }
      else if (depth == 1 && type == 0x7472616b) {
        if (tracks.size() >= 128) fail();
        tracks.emplace_back(); walk(body, at + length, 2, tracks.size() - 1);
      } else if (depth == 2 && type == 0x746b6864) {
        if (tracks[index].id || count < 4) fail();
        const auto version = uint_at(body, 1);
        const std::uint64_t offset = version == 0 ? 12 : 20;
        if (version > 1 || count < offset + 4) fail();
        const auto id = uint_at(body + offset, 4);
        if (!id) fail();
        tracks[index].id = id;
      } else if (depth == 2 && type == 0x6d646961) {
        if (tracks[index].media) fail();
        tracks[index].media = true; walk(body, at + length, 3, index);
      } else if (depth == 3 && type == 0x68646c72) {
        if (count < 12 || tracks[index].handler) fail();
        tracks[index].handler = uint_at(body + 8, 4);
      } else if (depth == 2 && type == 0x74726566) {
        if (tracks[index].references) fail();
        tracks[index].references = true; walk(body, at + length, 4, index);
      } else if (depth == 4) {
        if (type != 0x63686170 || !tracks[index].chapters.empty() || !count || count % 4 || count > 512) fail();
        for (std::uint64_t offset = 0; offset < count; offset += 4)
          tracks[index].chapters.push_back(uint_at(body + offset, 4));
      }
      at += length;
    }
  };
  walk(0, size, 0, 0);
  std::set<std::uint64_t> ids, text;
  const Track* audio = nullptr;
  for (const auto& track : tracks) {
    if (!track.id || !ids.insert(*track.id).second || !track.handler) fail();
    if (*track.handler == 0x736f756e) { if (audio) fail(); audio = &track; }
    else if (*track.handler == 0x74657874) { if (track.references) fail(); text.insert(*track.id); }
    else fail();
  }
  if (moov != 1 || !audio) fail();
  const std::set<std::uint64_t> references(audio->chapters.begin(), audio->chapters.end());
  if (references.size() != audio->chapters.size() || references != text) fail();
}
}
