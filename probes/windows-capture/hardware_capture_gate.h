#pragma once
#include <algorithm>
#include <chrono>
#include <condition_variable>
#include <cstdint>
#include <map>
#include <mutex>
#include <set>
#include <string>
#include <stdexcept>
#include <utility>

namespace product {
// A microphone can change its physical device's Bluetooth playback profile.
// The render owner releases only that device before microphone activation or
// disposal. A generation is acknowledged only after its COM clients are gone.
class HardwareCaptureGate {
  std::mutex mutex_;
  std::condition_variable changed_;
  std::map<std::string,unsigned> held_;
  std::uint64_t revision_=0,applied_=0,owner_=0,next_owner_=0;
 public:
  struct Snapshot {std::uint64_t revision;std::set<std::string> blocked;};
  class Lease {
    HardwareCaptureGate* gate_=nullptr;
    std::string container_;
    std::uint64_t revision_=0;
   public:
    Lease()=default;
    Lease(HardwareCaptureGate& gate,std::string container):gate_(container.empty()?nullptr:&gate),container_(std::move(container)){
      if(gate_){std::lock_guard lock(gate_->mutex_);++gate_->held_[container_];revision_=++gate_->revision_;}
    }
    Lease(const Lease&)=delete;Lease& operator=(const Lease&)=delete;
    ~Lease(){if(gate_){std::lock_guard lock(gate_->mutex_);auto it=gate_->held_.find(container_);if(!--it->second)gate_->held_.erase(it);++gate_->revision_;gate_->changed_.notify_all();}}
    template<class Cancelled>bool wait(Cancelled cancelled,std::chrono::milliseconds timeout){
      if(!gate_)return !cancelled();
      const auto deadline=std::chrono::steady_clock::now()+timeout;
      std::unique_lock lock(gate_->mutex_);
      while(gate_->owner_&&gate_->applied_<revision_){
        if(cancelled()||std::chrono::steady_clock::now()>=deadline)return false;
        gate_->changed_.wait_until(lock,std::min(deadline,std::chrono::steady_clock::now()+std::chrono::milliseconds(10)));
      }
      return !cancelled()&&std::chrono::steady_clock::now()<deadline;
    }
  };
  std::uint64_t attach(){std::lock_guard lock(mutex_);if(owner_)throw std::logic_error("overlapping render profile owners");owner_=++next_owner_;applied_=0;return owner_;}
  void detach(std::uint64_t owner){std::lock_guard lock(mutex_);if(owner_==owner){owner_=0;changed_.notify_all();}}
  Snapshot snapshot(){std::lock_guard lock(mutex_);Snapshot result{revision_,{}};for(const auto& [id,count]:held_)result.blocked.insert(id);return result;}
  void applied(std::uint64_t owner,std::uint64_t revision){std::lock_guard lock(mutex_);if(owner_==owner){applied_=std::max(applied_,revision);changed_.notify_all();}}
};
}
