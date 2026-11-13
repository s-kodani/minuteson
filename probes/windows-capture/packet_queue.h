#pragma once
#include <algorithm>
#include <cstdint>
#include <deque>
#include <exception>
#include <mutex>
#include <optional>
#include <stdexcept>
#include <vector>
#include "../../crates/mon-capture-ffi/include/mon_capture.h"

namespace product {
struct CapturePacket {
  std::vector<float> samples;
  std::uint32_t frames=0,flags=0;
  std::uint64_t time=0,position=0,arrival=0;
};
// At most 20 seconds per lane (11 MiB total PCM for mono + stereo).
// Never discard old audio to make room; an exhausted writer stops explicitly.
class PacketQueue {
 public:
  static inline const std::uint64_t max_frames=48000*mon_capture_limit(7)/1000000000;
  void push(CapturePacket packet) {
    std::lock_guard lock(mutex_);
    const auto bytes=std::uint64_t(packet.samples.size())*sizeof(float);
    const auto duration=(std::uint64_t(packet.frames)*1000000000+47999)/48000;
    if(!mon_capture_permits_pending(bytes_,nanoseconds_,
        static_cast<std::uint32_t>(packets_.size()),bytes,duration))
      throw std::runtime_error("音声の保存待ちが上限に達しました。保存先の負荷を確認してください。確定済み音声は保持しています");
    bytes_+=bytes;nanoseconds_+=duration;frames_+=packet.frames;peak_=std::max(peak_,frames_);packets_.push_back(std::move(packet));
  }
  std::optional<CapturePacket> pop() {
    std::lock_guard lock(mutex_);
    if(packets_.empty()){if(failure_)std::rethrow_exception(failure_);return std::nullopt;}
    auto packet=std::move(packets_.front());packets_.pop_front();frames_-=packet.frames;bytes_-=std::uint64_t(packet.samples.size())*sizeof(float);nanoseconds_-=(std::uint64_t(packet.frames)*1000000000+47999)/48000;return packet;
  }
  std::uint64_t pending_frames(){std::lock_guard lock(mutex_);return frames_;}
  void fail(std::exception_ptr error){std::lock_guard lock(mutex_);failure_=error;}
  std::uint64_t peak(){std::lock_guard lock(mutex_);return peak_;}
 private:
  std::mutex mutex_;
  std::deque<CapturePacket> packets_;
  std::uint64_t frames_=0,peak_=0,bytes_=0,nanoseconds_=0;
  std::exception_ptr failure_;
};
class CaptureReader {
 public:
  PacketQueue queue;
  virtual void stop()=0;
  virtual ~CaptureReader()=default;
};
} // namespace product
