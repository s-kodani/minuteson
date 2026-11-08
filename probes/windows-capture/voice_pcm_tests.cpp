#include "voice_pcm.h"
#include "packet_clock.h"
#include <algorithm>
#include <cmath>
#include <iostream>
#include <vector>

static void require(bool ok,const char* message){if(!ok)throw std::runtime_error(message);}
int main(){try{
  for(const auto rate:{8000u,16000u,44100u,48000u,96000u}){
    const std::size_t frames=rate*4+7;
    std::vector<float> input(frames*2);
    for(std::size_t i=0;i<frames;++i){input[i*2]=0.5f;input[i*2+1]=-0.25f;}
    std::vector<std::int16_t> expected;
    for(const auto block:{4096u,7u}){
      VoicePCM converter(rate,2);std::vector<std::int16_t> output;std::int64_t last=0;
      auto sink=[&](const std::int16_t* pcm,std::size_t n,std::int64_t start,std::int64_t end){
        require(start>=last&&end>start,"converted clock reversed");last=end;
        output.insert(output.end(),pcm,pcm+n);
      };
      for(std::size_t first=0;first<frames;){const auto n=std::min<std::size_t>(block,frames-first);
        converter.push(input.data()+first*2,n*2,first*1000000000/rate,(first+n)*1000000000/rate,sink);first+=n;}
      converter.finish(sink);
      require(output.size()==(frames*16000+rate-1)/rate,"converted tail/count differs");
      require(std::all_of(output.begin(),output.end(),[](auto n){return n==4096;}),"mono/PCM16 scaling differs");
      if(expected.empty())expected=output;else require(expected==output,"packet boundaries changed PCM");
      const auto header=VoicePCM::header(output.size());require(header.size()==68&&header[51]==16,"compact CAF header differs");
    }
  }
  product::PacketClock compact(product::PacketClock::bounded_capacity,16000);
  compact.append(0,16000,10000000,false,20000000);
  compact.append(16000,16000,30000000,true,40000000);
  require(compact.parts.size()==2&&compact.parts.back().first==16000,"16k capture gap lost");
  VoicePCM failing(48000,1);std::vector<float> packet(4096,0.1f);bool failed=false;
  try{failing.push(packet.data(),packet.size(),0,85333333,[](auto*,auto,auto,auto){throw std::runtime_error("disk full");});}catch(...){failed=true;}
  require(failed,"native IO failure was swallowed");
  failed=false;try{failing.finish([](auto*,auto,auto,auto){});}catch(...){failed=true;}
  require(failed,"failed converter resumed");
  std::cout<<"voice PCM C ABI: rates, mono, packet independence, compact CAF, clocks and terminal failures passed\n";
  return 0;
}catch(const std::exception& error){std::cerr<<error.what()<<'\n';return 1;}}
