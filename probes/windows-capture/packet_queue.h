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
struct InputDataFault : std::runtime_error {
  explicit InputDataFault(const char* reason):std::runtime_error(reason){}
};
// The OS or disk could not keep up with this input. Accepted audio stays
// queued and is never discarded; only this input is reacquired, so the
// recording continues and the unacquired span is stored as a recorded gap.
struct InputOverload : InputDataFault {
  using InputDataFault::InputDataFault;
};
inline void require_input_pcm(const float* samples,std::uint32_t frames,std::uint32_t channels,bool silent=false){
  if(!mon_capture_pcm_geometry(48000,channels,frames))throw InputDataFault("invalid PCM geometry");
  if(!silent&&!mon_capture_pcm_samples(samples,std::size_t(frames)*channels))throw InputDataFault("invalid PCM samples");
}
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
  std::uint64_t sequence=0;
};
// Render endpoints have independent native owners, but share one transient
// ingress allowance. This is not durable-storage credit: a pop releases it.
class PacketIngressBudget {
  std::mutex mutex_;
  std::uint64_t bytes_=0,nanoseconds_=0;
  std::uint32_t packets_=0;
 public:
  void acquire(std::uint64_t bytes,std::uint64_t duration){
    std::lock_guard lock(mutex_);
    if(!mon_capture_permits_pending(bytes_,nanoseconds_,packets_,bytes,duration))
      throw InputOverload("PC音声の取得待ちが上限に達しました。入力を取得し直します。確定済み音声は保持しています");
    bytes_+=bytes;nanoseconds_+=duration;++packets_;
  }
  void release(std::uint64_t bytes,std::uint64_t duration,std::uint32_t packets=1){
    std::lock_guard lock(mutex_);bytes_-=bytes;nanoseconds_-=duration;packets_-=packets;
  }
  std::uint64_t pending_nanoseconds(){std::lock_guard lock(mutex_);return nanoseconds_;}
};
// At most 20 seconds per lane (11 MiB total PCM for mono + stereo).
// Never discard old audio to make room; an exhausted writer stops explicitly.
class PacketQueue {
 public:
  static inline const std::uint64_t max_frames=48000*mon_capture_limit(7)/1000000000;
  ~PacketQueue(){if(ingress_)ingress_->release(bytes_,nanoseconds_,static_cast<std::uint32_t>(packets_.size()));}
  void attach_ingress(std::shared_ptr<PacketIngressBudget> budget){
    std::lock_guard lock(mutex_);
    if(ingress_||!packets_.empty())throw std::logic_error("transient ingress already active");
    ingress_=std::move(budget);
  }
  void push(CapturePacket packet) {
    // Rejection cannot acquire storage credits or contaminate the converter.
    const bool silent=(packet.flags&2)!=0;
    if(!packet.frames||(!silent&&(packet.samples.size()%packet.frames!=0)))throw InputDataFault("invalid queued PCM layout");
    const auto channels=packet.samples.empty()?1u:std::uint32_t(packet.samples.size()/packet.frames);
    require_input_pcm(packet.samples.data(),packet.frames,channels,silent);
    std::lock_guard lock(mutex_);
    const auto bytes=std::uint64_t(packet.samples.size())*sizeof(float);
    const auto duration=(std::uint64_t(packet.frames)*1000000000+47999)/48000;
    if(!mon_capture_permits_pending(bytes_,nanoseconds_,
        static_cast<std::uint32_t>(packets_.size()),bytes,duration))
      throw InputOverload("音声の保存待ちが上限に達しました。入力を取得し直します。保存先の負荷を確認してください。確定済み音声は保持しています");
    if(ingress_)ingress_->acquire(bytes,duration);
    try{
      if(storage_)packet.storage_end=storage_->acquire(lane_,packet.frames);
      const auto frames=packet.frames;packet.sequence=sequence_+1;
      packets_.push_back(std::move(packet));++sequence_;
      bytes_+=bytes;nanoseconds_+=duration;frames_+=frames;peak_=std::max(peak_,frames_);
    }catch(...){if(ingress_)ingress_->release(bytes,duration);throw;}
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
    auto packet=std::move(packets_.front());packets_.pop_front();const auto bytes=std::uint64_t(packet.samples.size())*sizeof(float),duration=(std::uint64_t(packet.frames)*1000000000+47999)/48000;
    frames_-=packet.frames;bytes_-=bytes;nanoseconds_-=duration;if(ingress_)ingress_->release(bytes,duration);return packet;
  }
  // Only the acquisition owner may publish this after draining the native
  // buffers and any adapter-held PCM preceding `through`. It belongs after
  // the exact accepted sequence, rather than a control-thread timer.
  void observe(std::uint64_t through){
    std::lock_guard lock(mutex_);
    observation_=Observation{sequence_,through};
  }
  std::optional<std::uint64_t> observed_through(std::uint64_t consumed){
    std::lock_guard lock(mutex_);
    if(!observation_||!packets_.empty()||observation_->sequence!=consumed)return std::nullopt;
    return observation_->through;
  }
  std::uint64_t pending_frames(){std::lock_guard lock(mutex_);return frames_;}
  void fail(std::exception_ptr error){std::lock_guard lock(mutex_);failure_=error;}
  std::uint64_t peak(){std::lock_guard lock(mutex_);return peak_;}
 private:
  std::mutex mutex_;
  std::deque<CapturePacket> packets_;
  std::uint64_t frames_=0,peak_=0,bytes_=0,nanoseconds_=0;
  std::uint64_t sequence_=0;
  struct Observation {std::uint64_t sequence,through;};
  std::optional<Observation> observation_;
  std::exception_ptr failure_;
  std::shared_ptr<StoragePending> storage_;unsigned lane_=0;
  std::shared_ptr<PacketIngressBudget> ingress_;
};
class CaptureReader {
 public:
  PacketQueue queue;
  virtual AlignmentDiagnostics alignment_diagnostics(){return {};}
  virtual void stop()=0;
  virtual ~CaptureReader()=default;
};
} // namespace product
