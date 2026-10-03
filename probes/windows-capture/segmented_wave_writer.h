#pragma once

#include <algorithm>
#include <array>
#include <cstdint>
#include <filesystem>
#include <fstream>
#include <iomanip>
#include <sstream>
#include <stdexcept>
#include <string>
#include <vector>

namespace mon {

class SegmentedWaveWriter {
 public:
  SegmentedWaveWriter(std::filesystem::path directory, std::string stem,
                      const std::uint8_t* format_bytes, std::uint32_t format_size,
                      std::uint32_t segment_bytes = 512 * 1024 * 1024)
      : directory_(std::move(directory)), stem_(std::move(stem)),
        segment_bytes_(segment_bytes) {
    if (!format_bytes || format_size < 16 || segment_bytes == 0)
      throw std::invalid_argument("invalid WAV format or segment limit");
    format_.assign(format_bytes, format_bytes + format_size);
    open_next();
  }

  ~SegmentedWaveWriter() {
    try { finish(); } catch (...) {}
  }

  void append(const std::uint8_t* bytes, std::uint32_t length, bool silent) {
    if (length > segment_bytes_)
      throw std::runtime_error("audio packet exceeds segment limit");
    if (!silent && length && !bytes)
      throw std::runtime_error("audio packet data is null");
    if (data_size_ > segment_bytes_ - length) {
      finish_current();
      open_next();
    }
    if (silent) {
      static constexpr std::array<char, 65536> zeros{};
      std::uint32_t remaining = length;
      while (remaining) {
        auto chunk = std::min<std::uint32_t>(remaining, zeros.size());
        file_.write(zeros.data(), chunk);
        remaining -= chunk;
      }
    } else {
      file_.write(reinterpret_cast<const char*>(bytes), length);
    }
    if (!file_) throw std::runtime_error("audio segment write failed");
    data_size_ += length;
  }

  void finish() {
    if (file_.is_open()) finish_current();
  }

  const std::vector<std::string>& files() const { return files_; }

 private:
  std::filesystem::path directory_;
  std::string stem_;
  std::vector<std::uint8_t> format_;
  std::uint32_t segment_bytes_;
  std::ofstream file_;
  std::vector<std::string> files_;
  std::uint32_t data_size_ = 0;

  void write_u32(std::uint32_t value) {
    const char bytes[4] = {static_cast<char>(value & 0xff),
                           static_cast<char>((value >> 8) & 0xff),
                           static_cast<char>((value >> 16) & 0xff),
                           static_cast<char>((value >> 24) & 0xff)};
    file_.write(bytes, sizeof(bytes));
  }

  void open_next() {
    std::ostringstream name;
    name << stem_ << '-' << std::setw(4) << std::setfill('0') << files_.size()
         << ".wav";
    file_.open(directory_ / name.str(), std::ios::binary | std::ios::trunc);
    if (!file_) throw std::runtime_error("audio segment could not be created");
    files_.push_back(name.str());
    file_.write("RIFF", 4);
    write_u32(0);
    file_.write("WAVEfmt ", 8);
    write_u32(static_cast<std::uint32_t>(format_.size()));
    file_.write(reinterpret_cast<const char*>(format_.data()), format_.size());
    if (format_.size() & 1) file_.put('\0');
    file_.write("data", 4);
    write_u32(0);
    if (!file_) throw std::runtime_error("audio segment header write failed");
    data_size_ = 0;
  }

  void finish_current() {
    file_.seekp(0, std::ios::end);
    if (data_size_ & 1) file_.put('\0');
    file_.seekp(4);
    const auto format_padding = static_cast<std::uint32_t>(format_.size() & 1);
    write_u32(4 + 8 + static_cast<std::uint32_t>(format_.size()) + format_padding +
              8 + data_size_ + (data_size_ & 1));
    file_.seekp(24 + static_cast<std::streamoff>(format_.size()) + format_padding);
    write_u32(data_size_);
    file_.flush();
    if (!file_) throw std::runtime_error("audio segment finalization failed");
    file_.close();
  }
};

}  // namespace mon
