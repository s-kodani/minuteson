#include "voice_pcm.h"
#include "packet_clock.h"
#include <algorithm>
#include <cmath>
#include <iostream>
#include <memory>
#include <vector>

static void require(bool ok,const char* message){if(!ok)throw std::runtime_error(message);}
static void fractional_mapped_boundaries_survive_converter_epochs(){
  // WASAPI's clock is quantized to 100 ns. A 479-frame 48 kHz packet is
  // 9,979,166 2/3 ns, so deriving its end independently overlaps the next
  // sample-clock boundary by 66 ns when an overlap starts a converter epoch.
  constexpr std::uint64_t origin=10000000;
  product::PacketClock clock;
  std::unique_ptr<VoicePCM> converter;
  std::unique_ptr<LiveMixer,decltype(&mon_wav_mixer_free)> mixer(mon_wav_mixer_new(),mon_wav_mixer_free);
  require(bool(mixer),"fractional mixer unavailable");
  std::vector<float> packet(479,0.125f);
  std::vector<std::int16_t> mixed;
  std::uint64_t sequence=0,epoch=0,frames=0;
  std::int64_t last=0;
  auto sink=[&](const std::int16_t* pcm,std::size_t n,std::int64_t start,std::int64_t end){
    require(start>=last&&end>start,"mapped converter epochs overlap");
    require(mon_wav_mixer_push(mixer.get(),0,epoch,++sequence,pcm,n,start,end),"fractional mixer order rejected");
    frames+=n;last=end;
  };
  auto write=[](void* context,const std::int16_t* pcm,std::size_t n,std::uint64_t first,std::int64_t start,std::uint32_t mask)->bool{
    auto& output=*static_cast<std::vector<std::int16_t>*>(context);
    if(first!=output.size()||start!=std::int64_t(origin*100)||mask!=1)return false;
    output.insert(output.end(),pcm,pcm+n);return true;
  };
  auto drain=[&]{
    require(mon_wav_mixer_advance(mixer.get(),0,sequence,last)&&mon_wav_mixer_advance(mixer.get(),1,0,last),"fractional frontier rejected");
    require(mon_wav_mixer_drain(mixer.get(),last,&mixed,write),"fractional mixed output failed");
  };
  for(std::uint64_t index=0;index<500;++index){
    const auto mapped=clock.observe(479,origin+index*75000,index*160,0,origin+(index+1)*100000);
    if(mapped.boundary){if(converter)converter->finish(sink);converter.reset();++epoch;}
    if(!converter)converter=std::make_unique<VoicePCM>(48000,1);
    converter->push(packet.data(),packet.size(),mapped.start*100,mapped.end*100,sink);
    drain();
  }
  converter->finish(sink);drain();
  require(mon_wav_mixer_finish(mixer.get(),&mixed,write),"fractional mixer close failed");
  require(frames==79834&&last==std::int64_t((origin+500ULL*479*10000000/48000)*100),"fractional mapped tail changed");
  require(mixed.size()==79834&&std::all_of(mixed.begin(),mixed.end(),[](auto n){return n==4096;}),"fractional mixer moved or changed PCM");
  require(clock.estimated_packets==499,"fractional clock lost anomaly evidence");
}
int main(){try{
  fractional_mapped_boundaries_survive_converter_epochs();
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
      const auto wav=VoicePCM::wave_header(output.size());
      require(wav.size()==80&&std::string(wav.begin(),wav.begin()+4)=="RIFF"
        &&std::string(wav.begin()+8,wav.begin()+12)=="WAVE"
        &&std::string(wav.begin()+12,wav.begin()+16)=="JUNK"
        &&std::string(wav.begin()+72,wav.begin()+76)=="data","compact WAV header differs");
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
  const auto rf64=VoicePCM::wave_header(std::uint64_t(1)<<31);
  require(std::string(rf64.begin(),rf64.begin()+4)=="RF64"
    &&std::string(rf64.begin()+12,rf64.begin()+16)=="ds64","long recording failed RF64 upgrade");
  std::cout<<"voice PCM C ABI: rates, mono, packet independence, CAF/WAV/RF64, clocks and terminal failures passed\n";
  return 0;
}catch(const std::exception& error){std::cerr<<error.what()<<'\n';return 1;}}
