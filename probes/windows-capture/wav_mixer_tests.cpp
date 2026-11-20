#include "../../crates/mon-capture-ffi/include/mon_capture.h"
#include <algorithm>
#include <cstdint>
#include <iostream>
#include <stdexcept>
#include <vector>

static void require(bool value,const char* message){if(!value)throw std::runtime_error(message);}
struct Output {
  std::vector<std::int16_t> pcm;
  std::vector<std::uint32_t> coverage;
  static bool write(void* context,const std::int16_t* pcm,std::size_t count,std::uint64_t first,std::int64_t origin,std::uint32_t mask){
    auto& output=*static_cast<Output*>(context);
    if(first!=output.pcm.size()||origin!=1000000000LL)return false;
    output.pcm.insert(output.pcm.end(),pcm,pcm+count);
    output.coverage.insert(output.coverage.end(),count,mask);
    return true;
  }
};
int main(){try{
  auto* mixer=mon_wav_mixer_new();require(mixer,"mixer unavailable");
  struct Free {LiveMixer* value;~Free(){mon_wav_mixer_free(value);}} free{mixer};
  std::vector<std::int16_t> mic(320,10000),pc(160,20000);
  require(mon_wav_mixer_push(mixer,0,1,1,mic.data(),mic.size(),1000000000LL,1020000000LL),"microphone rejected");
  Output output;
  require(mon_wav_mixer_drain(mixer,1020000000LL,&output,Output::write)&&output.pcm.empty(),
    "microphone overtook unprocessed PC input");
  require(mon_wav_mixer_push(mixer,1,1,1,pc.data(),pc.size(),1010000000LL,1020000000LL),"PC input rejected");
  require(mon_wav_mixer_advance(mixer,0,1,1030000000LL)&&mon_wav_mixer_advance(mixer,1,1,1030000000LL),"ordered barrier rejected");
  require(mon_wav_mixer_drain(mixer,1030000000LL,&output,Output::write),"mix failed");
  require(mon_wav_mixer_finish(mixer,&output,Output::write),"mix close failed");
  require(output.pcm.size()==480,"actual closure padding missing");
  require(std::all_of(output.pcm.begin(),output.pcm.begin()+160,[](auto n){return n==10000;}),"single-input volume changed");
  require(std::all_of(output.pcm.begin()+160,output.pcm.begin()+320,[](auto n){return n==15000;}),"two-input interval average changed");
  require(std::all_of(output.pcm.begin()+320,output.pcm.end(),[](auto n){return n==0;}),"unreceived gap was squeezed out");
  require(output.coverage[0]==1&&output.coverage[160]==3&&output.coverage[320]==0,"silence/gap provenance missing");
  require(!mon_wav_mixer_push(mixer,0,1,2,mic.data(),1,1030000000LL,1030062500LL),"closed mixer accepted PCM");
  std::cout<<"live WAV C ABI: ordered barriers, interval gain, gap provenance and closure passed\n";
  return 0;
}catch(const std::exception& error){std::cerr<<error.what()<<'\n';return 1;}}
