#pragma once
#include <algorithm>
#include <cstdint>
#include <deque>
#include <exception>
#include <mutex>
#include <memory>
#include <optional>
#include <stdexcept>
#include <vector>
#include "../../crates/mon-capture-ffi/include/mon_capture.h"
#include "storage_pending.h"

namespace product {
struct AlignmentLoss {
  std::uint64_t first=0,frames=0;
  const char* reason="late";
  std::uint64_t mapped_first=0;
};
struct AlignmentDiagnostics {
  std::uint64_t late_packets=0,late_frames=0,capacity_frames=0,reset_frames=0;
  std::uint64_t retimed_packets=0,retimed_frames=0,clock_adjustment_frames=0,pressure_flushes=0;
  std::vector<AlignmentLoss> losses;
  bool truncated=false;
  void loss(std::uint64_t first,std::uint64_t frames,const char* reason,std::uint64_t mapped_first=0){
    if(!frames)return;
    if(losses.size()<32)losses.push_back({first,frames,reason,mapped_first});else truncated=true;
  }
  void merge(const AlignmentDiagnostics& other){
    late_packets+=other.late_packets;late_frames+=other.late_frames;
    capacity_frames+=other.capacity_frames;reset_frames+=other.reset_frames;
    retimed_packets+=other.retimed_packets;retimed_frames+=other.retimed_frames;
    clock_adjustment_frames+=other.clock_adjustment_frames;pressure_flushes+=other.pressure_flushes;
    for(const auto& entry:other.losses)loss(entry.first,entry.frames,entry.reason,entry.mapped_first);
    truncated|=other.truncated;
  }
};
struct CapturePacket {
  std::vector<float> samples;
  std::uint32_t frames=0,flags=0;
  std::uint64_t time=0,position=0,arrival=0;
  std::uint64_t storage_end=0;
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
    if(storage_)packet.storage_end=storage_->acquire(lane_,packet.frames);
    bytes_+=bytes;nanoseconds_+=duration;frames_+=packet.frames;peak_=std::max(peak_,frames_);packets_.push_back(std::move(packet));
  }
  // Preparation may be cancelled before adoption, so only product-owned
  // ingress is charged. Stamp the already bounded preparation queue atomically
  // before any of its packets can enter the recording converter.
  void attach_storage(std::shared_ptr<StoragePending> storage,unsigned lane){
    std::lock_guard lock(mutex_);
    if(storage_)throw std::logic_error("recording ingress already adopted");
    auto end=storage->acquire(lane,frames_);auto cursor=end-frames_;
    for(auto& packet:packets_){cursor+=packet.frames;packet.storage_end=cursor;}
    storage_=std::move(storage);lane_=lane;
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
  std::shared_ptr<StoragePending> storage_;unsigned lane_=0;
};
class CaptureReader {
 public:
  PacketQueue queue;
  virtual AlignmentDiagnostics alignment_diagnostics(){return {};}
  virtual void stop()=0;
  virtual ~CaptureReader()=default;
};
} // namespace product
