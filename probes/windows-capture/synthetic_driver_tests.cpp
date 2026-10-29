#include "synthetic_driver.h"
#include <stdexcept>

void require(bool condition){if(!condition)throw std::runtime_error("synthetic driver contract differs");}

int main(){
  using namespace mon::capture;
  constexpr std::uint64_t start=10000000;
  SyntheticDriver recovery({"input-recovery"});
  unsigned mic_losses=0,mic_connections=0,system_losses=0,system_connections=0;
  for(std::uint64_t i=0;i<500;++i){
    require(!recovery.finished());
    const auto packet=recovery.next(start,0);
    require(packet.time==start+i*100000&&packet.position==i*480&&packet.frames==480);
    require(packet.arrival==start+(i+1)*100000);
    if(packet.microphone_lost){require(i==25);++mic_losses;}
    if(packet.microphone_connected){require(i==60);++mic_connections;}
    if(packet.system_lost){require(i==80);++system_losses;}
    if(packet.system_connected){require(i==120);++system_connections;}
  }
  require(recovery.finished()&&mic_losses==1&&mic_connections==1&&system_losses==1&&system_connections==1);
  SyntheticDriver only({"system-only"});
  require(only.next(start,0).microphone_lost&&!only.next(start,0).microphone_lost);
  SyntheticDriver cases({"",false,true});
  for(unsigned i=0;i<8;++i){const auto packet=cases.next(start,0);
    if(i==1)require(packet.time==start);
    if(i==2)require(packet.time==start-100);
    if(i==3)require(packet.time==0&&(packet.flags&4));
    if(i==4)require(packet.position==0&&(packet.flags&1));
    if(i==7)require(packet.position==2400);
  }
  SyntheticDriver resampling({"",false,false,true});
  resampling.next(start,0);const auto resampled=resampling.next(start,0);
  require(resampled.time==start+75000&&resampled.position==160&&resampled.frames==479);
  SyntheticDriver drift({"",true,false,false,true});
  drift.next(start,0);const auto overlap=drift.next(start,0);
  require(overlap.time==start+100000&&overlap.flags==1);
}
