#include "endpoint_mixer.h"
#include <iostream>
void require(bool ok,const char* message){if(!ok)throw std::runtime_error(message);}
int main(){
  try{
    using product::EndpointMixer;using product::CapturePacket;
    constexpr std::uint64_t first=480000;
    EndpointMixer mixer;std::vector<CapturePacket> output;
    auto emit=[&](CapturePacket packet){output.push_back(std::move(packet));};
    std::vector<float> a(960,0.25f),b(1440,0.5f);
    // Two endpoints, staggered and with different packet boundaries. The
    // second endpoint must be audible while the first is delivering zeros.
    mixer.add(first,480,a.data(),0);
    mixer.add(first+480,480,nullptr,2);
    mixer.add(first+240,720,b.data(),0);
    mixer.flush(first+479,100000000,emit);
    require(output.empty(),"mixer published before the alignment window closed");
    mixer.flush(first+960,100000000,emit);
    require(output.size()==2&&output[0].frames==480&&output[1].frames==480,"mixed frame count changed");
    for(unsigned i=0;i<480;++i)for(unsigned ch=0;ch<2;++ch){
      require(output[0].samples[i*2+ch]==(i<240?0.25f:0.75f),"first endpoint mix shifted or lost audio");
      require(output[1].samples[i*2+ch]==0.5f,"silent endpoint suppressed another output");
    }
    require(output[1].position-output[0].position==480&&output[1].time-output[0].time==100000,"endpoint clocks not aligned");
    bool late=false;try{mixer.add(first,1,a.data(),0);}catch(const std::runtime_error&){late=true;}
    require(late,"late audio was silently discarded or duplicated");
    // A physical gap, including a hole inside a block, is absent from PCM.
    EndpointMixer gaps;output.clear();
    gaps.add(first+10,20,a.data(),0);gaps.add(first+40,20,nullptr,2);
    gaps.add(first+980,20,a.data(),4);
    gaps.flush(first+1440,100000000,emit);
    require(output.size()==3&&output[0].frames==20&&output[1].frames==20&&output[2].frames==20,"mixer invented samples in a capture gap");
    require(output[0].position==first+10&&output[1].position==first+40&&output[2].position==first+980,"mixer squeezed capture gaps out of the timeline");
    require(output[1].samples.empty()&&(output[1].flags&2)&&output[2].flags==4,"silence or uncertain timestamp lost");
    EndpointMixer clipping;output.clear();std::vector<float> loud(960,0.75f);
    clipping.add(first,480,loud.data(),0);clipping.add(first,480,loud.data(),1);
    clipping.flush(first+480,100000000,emit);
    require(output[0].flags==1&&output[0].samples[0]==1,"mixed level overflow or discontinuity hidden");
    EndpointMixer bounded;bool overflow=false;
    try{for(unsigned i=0;i<=EndpointMixer::max_blocks;++i)bounded.add(first+i*480,480,nullptr,2);}
    catch(const std::runtime_error&){overflow=true;}
    require(overflow,"endpoint alignment wait is unbounded");
    for(std::uint64_t frame:{0ULL,1ULL,479ULL,480ULL,480001ULL,480000000000ULL})
      require(EndpointMixer::frame_at(EndpointMixer::ticks_at(frame))==frame,"QPC conversion lost sample position");
    std::cout<<"endpoint mixer: concurrent output signals, silent peer, packet alignment, gaps, bounded wait and clipping passed\n";
    return 0;
  }catch(const std::exception& error){std::cerr<<error.what()<<'\n';return 1;}
}
