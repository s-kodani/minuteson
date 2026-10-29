#pragma once
#include <algorithm>
#include <cstdint>
#include <string>
#include <utility>

namespace mon::capture {
// A driver produces the same packet metadata and loss/recovery events as an OS
// adapter. CaptureSession retains ownership of Lane, sinks and publication.
struct SyntheticOptions {
  std::string scenario;
  bool overlap=false, cases=false, resampling=false, drift=false;
};
struct SyntheticPacket {
  std::uint64_t time=0, position=0, arrival=0;
  std::uint32_t frames=480, flags=0;
  bool microphone_lost=false, microphone_connected=false;
  bool system_lost=false, system_connected=false;
};
class SyntheticDriver {
  SyntheticOptions options_;
  std::uint64_t packets_=0;
public:
  explicit SyntheticDriver(SyntheticOptions options):options_(std::move(options)){}
  bool finished()const{return packets_>=500;}
  SyntheticPacket next(std::uint64_t start,std::uint64_t now){
    const auto index=packets_++;
    SyntheticPacket packet;
    packet.microphone_lost=(index==0&&options_.scenario=="system-only")
      ||(index==25&&options_.scenario=="input-recovery");
    packet.microphone_connected=index==60&&options_.scenario=="input-recovery";
    packet.system_lost=index==80&&options_.scenario=="input-recovery";
    packet.system_connected=index==120&&options_.scenario=="input-recovery";
    packet.time=start+index*100000-(options_.overlap&&index?10:0);
    if(options_.drift)packet.time+=std::min<std::uint64_t>(index,399)*10;
    packet.position=index*480;
    packet.flags=options_.overlap&&index==1?1:0;
    if(options_.resampling){packet.time=start+index*75000;packet.position=index*160;packet.frames=479;}
    if(options_.cases){
      if(index==1)packet.time=start;
      if(index==2)packet.time=start-100;
      if(index==3){packet.time=0;packet.flags|=4;}
      if(index>=4)packet.position=(index-4)*480;
      if(index==4)packet.flags|=1;
      if(index>=7)packet.position+=960;
    }
    // Synthetic clocks advance independently of scheduler latency.
    packet.arrival=std::max(now,start+(index+1)*100000);
    return packet;
  }
};
}
