#pragma once
#include "packet_queue.h"
#include <array>
#include <cmath>
#include <limits>
#include <map>

namespace product {
// A cursor belongs to one endpoint/epoch. Clock corrections move whole
// packets; they never shorten PCM or reapply an adjustment to every packet.
struct EndpointCursor {
  std::optional<std::uint64_t> next;
  std::uint64_t correction=0,last_arrival=0;
  bool estimated=false;
  std::uint64_t safe_through(std::uint64_t now,std::uint64_t through)const {
    // Endpoint capacity (500 ms) plus the alignment window. Idle endpoints do
    // not hold an active output indefinitely; returning PCM is preserved below.
    if(next&&(now<=last_arrival||now-last_arrival<=7500000))return std::min(through,*next);
    return through;
  }
};

class EndpointMixer {
 public:
  static constexpr std::uint64_t block_frames=480,latency_frames=12000,max_blocks=100;
  static std::uint64_t frame_at(std::uint64_t ticks){return ticks/10000000*48000+(ticks%10000000*48000+5000000)/10000000;}
  static std::uint64_t ticks_at(std::uint64_t frame){return frame/48000*10000000+(frame%48000*10000000+47999)/48000;}
  struct AddResult {std::uint32_t accepted=0;bool adjusted=false,pressure=false;};
  const AlignmentDiagnostics& diagnostics()const{return diagnostics_;}
  std::size_t pending_blocks()const{return blocks_.size();}

  template<class Emit>AddResult add(EndpointCursor& source,std::uint64_t first,
      std::uint32_t frames,const float* samples,std::uint32_t flags,std::uint64_t arrival,Emit emit){
    AddResult result;
    if(!frames)return result;
    if(frames>48000||first>std::numeric_limits<std::uint64_t>::max()-frames)
      throw std::runtime_error("invalid endpoint packet geometry");
    const bool silent=(flags&2)!=0;
    if(!silent&&!samples)throw std::runtime_error("PC音声のPCMがありません");
    for(std::uint64_t i=0;!silent&&i<std::uint64_t(frames)*2;++i)
      if(!std::isfinite(samples[i])||std::abs(samples[i])>16)throw std::runtime_error("invalid endpoint PCM amplitude");
    auto mapped=checked_add(first,source.correction);
    if(source.next){
      const auto distance=mapped>*source.next?mapped-*source.next:*source.next-mapped;
      if(distance<=1)mapped=*source.next;
    }
    bool late_noted=false;
    auto advance=[&](std::uint64_t floor,std::uint32_t consumed){
      const auto current=checked_add(mapped,consumed);
      if(current>=floor)return;
      const auto adjustment=floor-current;
      if(current<sealed_){
        if(!late_noted)++diagnostics_.late_packets;
        diagnostics_.late_frames+=std::min<std::uint64_t>(frames-consumed,sealed_-current);
        late_noted=true;
      }
      mapped=checked_add(mapped,adjustment);
      source.correction=checked_add(source.correction,adjustment);source.estimated=true;
      diagnostics_.clock_adjustment_frames+=adjustment;
      diagnostics_.loss(current,frames-consumed,"clock_rebased",floor);
      result.adjusted=true;
    };
    advance(std::max(sealed_,source.next.value_or(0)),0);
    checked_add(mapped,frames);
    while(result.accepted<frames){
      advance(sealed_,result.accepted);
      checked_add(mapped,frames);
      const auto frame=checked_add(mapped,result.accepted),base=frame/block_frames*block_frames;
      if(!blocks_.count(base)&&blocks_.size()>=max_blocks){
        // Pressure publishes received PCM to the existing bounded writer queue.
        // It never discards pending audio or restarts an otherwise live input.
        diagnostics_.capacity_frames+=frames-result.accepted;
        ++diagnostics_.pressure_flushes;result.pressure=true;
        emit_first(arrival,emit);continue;
      }
      auto& block=blocks_[base];block.flags|=(flags&~2u)|(source.estimated?4u:0u);
      const auto index=frame-base,count=std::min<std::uint64_t>(frames-result.accepted,block_frames-index);
      for(std::uint64_t i=0;i<count;++i){
        block.covered[index+i]=true;
        if(!silent)for(unsigned channel=0;channel<2;++channel)
          block.samples[(index+i)*2+channel]+=samples[(result.accepted+i)*2+channel];
      }
      result.accepted+=static_cast<std::uint32_t>(count);
    }
    source.next=checked_add(mapped,frames);source.last_arrival=arrival;
    if(result.adjusted){++diagnostics_.retimed_packets;diagnostics_.retimed_frames+=frames;}
    return result;
  }
  template<class Emit>void flush(std::uint64_t through,std::uint64_t arrival,Emit emit){
    while(!blocks_.empty()&&blocks_.begin()->first+block_frames<=through)emit_first(arrival,emit);
  }
 private:
  struct Block {std::array<float,block_frames*2> samples{};std::array<bool,block_frames> covered{};std::uint32_t flags=0;};
  std::map<std::uint64_t,Block> blocks_;
  std::uint64_t sealed_=0;
  AlignmentDiagnostics diagnostics_;
  static std::uint64_t checked_add(std::uint64_t a,std::uint64_t b){
    if(b>std::numeric_limits<std::uint64_t>::max()-a)throw std::runtime_error("endpoint clock overflow");
    return a+b;
  }
  template<class Emit>void emit_first(std::uint64_t arrival,Emit emit){
    auto it=blocks_.begin();const auto base=it->first;auto& block=it->second;
    for(std::uint32_t first=0;first<block_frames;){
      if(!block.covered[first]){++first;continue;}
      auto end=first+1;while(end<block_frames&&block.covered[end])++end;
      CapturePacket packet;packet.frames=end-first;packet.time=ticks_at(base+first);
      packet.position=base+first;packet.arrival=arrival;packet.flags=block.flags;
      bool silent=true;packet.samples.reserve(packet.frames*2);
      for(auto i=first*2;i<end*2;++i){
        const auto sample=std::clamp(block.samples[i],-1.0f,1.0f);
        packet.samples.push_back(sample);silent&=sample==0;
      }
      if(silent){packet.flags|=2;packet.samples.clear();}
      emit(std::move(packet));first=end;
    }
    sealed_=checked_add(base,block_frames);blocks_.erase(it);
  }
};
} // namespace product
