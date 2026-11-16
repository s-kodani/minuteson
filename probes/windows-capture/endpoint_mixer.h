#pragma once
#include "packet_queue.h"
#include <array>
#include <cmath>
#include <limits>
#include <map>

namespace product {
// Render endpoints share WASAPI's QPC time domain, but deliver different
// packet boundaries. Hold 250 ms to combine them before publishing one lane.
// Coverage represents received frames, including real silent packets. Empty
// intervals are never converted into invented silence or squeezed out.
class EndpointMixer {
 public:
  static constexpr std::uint64_t block_frames=480,latency_frames=12000,max_blocks=100;
  static std::uint64_t frame_at(std::uint64_t ticks){return ticks/10000000*48000+(ticks%10000000*48000+5000000)/10000000;}
  static std::uint64_t ticks_at(std::uint64_t frame){return frame/48000*10000000+(frame%48000*10000000+47999)/48000;}
  struct AddResult {std::uint32_t accepted=0,late=0,capacity=0;};
  const AlignmentDiagnostics& diagnostics()const{return diagnostics_;}
  std::size_t pending_blocks()const{return blocks_.size();}
  void discard_pending(){
    for(const auto& [first,block]:blocks_){
      for(std::uint32_t from=0;from<block_frames;){
        if(!block.covered[from]){++from;continue;}
        auto end=from+1;while(end<block_frames&&block.covered[end])++end;
        diagnostics_.reset_frames+=end-from;diagnostics_.loss(first+from,end-from,"resync");from=end;
      }
    }
    blocks_.clear();
  }
  AddResult add(std::uint64_t first,std::uint32_t frames,const float* samples,std::uint32_t flags){
    AddResult result;
    if(!frames)return result;
    if(frames>48000||first>std::numeric_limits<std::uint64_t>::max()-frames)
      throw std::runtime_error("invalid endpoint packet geometry");
    const bool silent=(flags&2)!=0;
    if(!silent&&!samples)throw std::runtime_error("PC音声のPCMがありません");
    // Validate even excluded samples, before mutating an admitted block.
    if(!silent)for(std::uint64_t i=0;i<std::uint64_t(frames)*2;++i)
      if(!std::isfinite(samples[i])||std::abs(samples[i])>16)throw std::runtime_error("invalid endpoint PCM amplitude");
    if(first<sealed_){
      result.late=static_cast<std::uint32_t>(std::min<std::uint64_t>(frames,sealed_-first));
      ++diagnostics_.late_packets;diagnostics_.late_frames+=result.late;
      diagnostics_.loss(first,result.late,"late");
    }
    for(std::uint32_t offset=result.late;offset<frames;){
      const auto frame=first+offset,base=frame/block_frames*block_frames;
      if(!blocks_.count(base)&&blocks_.size()>=max_blocks){
        result.capacity=frames-offset;diagnostics_.capacity_frames+=result.capacity;
        diagnostics_.loss(frame,result.capacity,"capacity");break;
      }
      auto& block=blocks_[base];
      block.flags|=flags&~2u;
      auto index=frame-base,count=std::min<std::uint64_t>(frames-offset,block_frames-index);
      for(std::uint64_t i=0;i<count;++i){
        block.covered[index+i]=true;
        if(!silent)for(unsigned channel=0;channel<2;++channel){
          auto sample=samples[(offset+i)*2+channel];
          block.samples[(index+i)*2+channel]+=sample;
        }
      }
      offset+=static_cast<std::uint32_t>(count);result.accepted+=static_cast<std::uint32_t>(count);
    }
    return result;
  }
  template<class Emit>void flush(std::uint64_t through,std::uint64_t arrival,Emit emit){
    while(!blocks_.empty()&&blocks_.begin()->first+block_frames<=through){
      auto it=blocks_.begin();const auto base=it->first;auto& block=it->second;
      for(std::uint32_t first=0;first<block_frames;){
        if(!block.covered[first]){++first;continue;}
        auto end=first+1;while(end<block_frames&&block.covered[end])++end;
        CapturePacket packet;packet.frames=end-first;packet.time=ticks_at(base+first);
        packet.position=base+first;packet.arrival=arrival;packet.flags=block.flags;
        bool silent=true;packet.samples.reserve(packet.frames*2);
        for(auto i=first*2;i<end*2;++i){
          auto sample=std::clamp(block.samples[i],-1.0f,1.0f);
          packet.samples.push_back(sample);silent&=sample==0;
        }
        if(silent){packet.flags|=2;packet.samples.clear();}
        emit(std::move(packet));first=end;
      }
      sealed_=base+block_frames;blocks_.erase(it);
    }
  }
 private:
  struct Block {std::array<float,block_frames*2> samples{};std::array<bool,block_frames> covered{};std::uint32_t flags=0;};
  std::map<std::uint64_t,Block> blocks_;
  std::uint64_t sealed_=0;
  AlignmentDiagnostics diagnostics_;
};
} // namespace product
