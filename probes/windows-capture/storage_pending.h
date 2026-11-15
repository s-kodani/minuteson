#pragma once
#include "../../crates/mon-capture-ffi/include/mon_capture.h"
#include <array>
#include <cstdint>
#include <exception>
#include <mutex>
#include <stdexcept>

namespace product {
// Source frames are charged before acceptance, including the native ingress
// queue and converter halo. Only a durable segment tip releases this charge.
// Compact bytes/jobs include both assemblers and the in-flight disk job.
class StoragePending {
  mutable std::mutex mutex_;
  std::array<std::uint64_t,2> acquired_{},committed_{};
  std::uint64_t bytes_=0;
  std::uint32_t jobs_=0;
  std::exception_ptr error_;
  void check_locked() const {if(error_)std::rethrow_exception(error_);}
 public:
  static constexpr std::uint64_t active_bytes=256*1024;
  std::uint64_t acquire(unsigned lane,std::uint64_t frames){
    std::lock_guard lock(mutex_);check_locked();
    const auto pending=acquired_.at(lane)-committed_.at(lane);
    const auto limit=mon_capture_limit(12)*48000/1000000000;
    if(frames>limit||pending>limit-frames)
      throw std::runtime_error("保存先への書込みが遅れ、未保存音声が120秒の上限に達しました。確定済み音声を保持しています");
    acquired_[lane]+=frames;return acquired_[lane];
  }
  void active(){
    std::lock_guard lock(mutex_);check_locked();
    if(!mon_capture_permits_storage_pending(bytes_,0,jobs_,active_bytes,0))
      throw std::runtime_error("未保存音声のメモリ上限に達しました。確定済み音声を保持しています");
    bytes_+=active_bytes;++jobs_;
  }
  void replace_active(std::uint64_t job_bytes,bool next_active){
    std::lock_guard lock(mutex_);check_locked();
    if(bytes_<active_bytes||!jobs_)throw std::logic_error("missing active recording buffer");
    const auto base=bytes_-active_bytes;const auto jobs=jobs_-1;
    if(!mon_capture_permits_storage_pending(base,0,jobs,job_bytes,0)
      ||(next_active&&!mon_capture_permits_storage_pending(base+job_bytes,0,jobs+1,active_bytes,0)))
      throw std::runtime_error("未保存音声のメモリ上限に達しました。確定済み音声を保持しています");
    bytes_=base+job_bytes+(next_active?active_bytes:0);jobs_=jobs+1+(next_active?1:0);
  }
  void abandon_active(){std::lock_guard lock(mutex_);bytes_-=active_bytes;--jobs_;}
  void committed(unsigned lane,std::uint64_t source_end,std::uint64_t job_bytes){
    std::lock_guard lock(mutex_);
    if(source_end<committed_.at(lane)||source_end>acquired_.at(lane)||job_bytes>bytes_||!jobs_)
      throw std::logic_error("invalid durable recording acknowledgement");
    committed_[lane]=source_end;bytes_-=job_bytes;--jobs_;
  }
  void fail(std::exception_ptr error){std::lock_guard lock(mutex_);if(!error_)error_=error;}
  void check()const{std::lock_guard lock(mutex_);check_locked();}
  struct Snapshot {std::array<std::uint64_t,2> frames;std::uint64_t bytes;std::uint32_t jobs;};
  Snapshot snapshot()const{std::lock_guard lock(mutex_);return {{{acquired_[0]-committed_[0],acquired_[1]-committed_[1]}},bytes_,jobs_};}
};
} // namespace product
