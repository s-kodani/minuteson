#include "packet_clock.h"
#include <iostream>
#include <string>

void require(bool condition,const char* message) {
  if(!condition)throw std::runtime_error(message);
}
int main() {
  using product::PacketClock;
  constexpr std::uint64_t start=100000000;
  try {
    PacketClock clock;
    clock.append(0,480,start,false);
    // The starts advance by 9.999 ms. The old check failed because the
    // preceding nominal packet end lay 1 us after this valid observation.
    clock.append(480,480,start+99990,true);
    require(clock.parts.size()==2,"discontinuity must remain explicit");
    require(clock.parts[0].last_end==clock.parts[1].start,"maps overlap");
    require(clock.parts[0].frames+clock.parts[1].frames==960,"PCM frames lost");
    require(clock.boundary_adjustments==1&&clock.max_boundary_adjustment_ticks==10,"correction not retained");
    // Repeated discontinuity flags and changing packet lengths remain valid.
    clock.append(960,240,start+199980,true);
    clock.append(1200,479,start+249970,true);
    require(clock.parts.size()==4,"later packet lost");
    const auto before=clock.parts.back().last_end;
    clock.append(1679,480,before+300000,false);
    require(clock.parts[3].last_end==before,"positive capture gap erased");
    require(clock.parts.back().start==before+300000,"gap changed");
    PacketClock continuous;
    for(std::uint64_t i=0;i<1000;++i)continuous.append(i*480,480,start+i*100000,false);
    require(continuous.parts.size()==1&&continuous.parts[0].frames==480000,"continuous stream changed");
    require(!continuous.boundary_adjustments,"continuous stream adjusted");
    PacketClock anomalous;
    auto save=[&](std::uint64_t frame,std::uint64_t raw,std::uint64_t pos,std::uint32_t flags=0){
      auto packet=anomalous.observe(480,raw,pos,flags,start+10000000);
      anomalous.append(frame,480,packet.start,packet.boundary);return packet;
    };
    save(0,start,0);
    auto equal=save(480,start,480);
    require(equal.start==start+100000&&std::string(equal.source)=="device_position","equal QPC not recovered from device position");
    auto reversed=save(960,start-100,960);
    require(reversed.start==start+200000,"backwards QPC changed sample timeline");
    save(1440,0,1440,4);
    auto reset=save(1920,start+400000,0,1);
    require(reset.start==start+400000&&std::string(reset.source)=="sample_count","uncorrelated counter not recovered from samples");
    auto gap=save(2400,start+500000,1440);
    require(gap.start==start+700000,"known missing device frames erased");
    require(anomalous.parts.back().start-anomalous.parts[anomalous.parts.size()-2].last_end==200000,"known gap stretched into PCM");
    require(anomalous.equal_timestamps==1&&anomalous.reversed_timestamps==2&&anomalous.timestamp_errors==1,"original anomalies not counted");
    require(anomalous.estimated_packets==5&&anomalous.frame_fallbacks==1,"fallback counts differ");
    std::uint64_t total=0,end=0;
    for(auto p:anomalous.parts){require(p.start>=end&&p.last_end>p.start,"non-monotone clock map");end=p.last_end;total+=p.frames;}
    require(total==2880,"anomalous PCM frames lost");
    // Repeated equal/backwards clocks must not create one segment per packet.
    PacketClock repeated;
    for(std::uint64_t i=0;i<10000;++i){auto p=repeated.observe(479,start,i*479,1,start+20000000);repeated.append(i*479,479,p.start,p.boundary);}
    require(repeated.parts.size()==2,"fallback fragments long recordings");
    require(repeated.parts.back().last_end-start==10000ULL*479*10000000/48000,"fractional frame rounding drift");
    repeated.begin_stream();
    auto restarted=repeated.observe(480,start,0,0,start+2000000000);
    require(restarted.start>=repeated.parts.back().last_end,"restart reversed saved timeline");
    PacketClock unavailable;
    auto first=unavailable.observe(480,0,0,4,start);
    require(first.start==start-100000&&std::string(first.source)=="host_arrival","initial invalid timestamp lacks arrival anchor");
    PacketClock future;
    require(std::string(future.observe(480,start+1,0,0,start).source)=="host_arrival","future QPC accepted");
    PacketClock corrupt_position;
    corrupt_position.observe(480,start,0,0,start+100000);
    auto huge=corrupt_position.observe(480,start,UINT64_MAX,0,start+200000);
    require(huge.start==start+100000&&std::string(huge.source)=="sample_count","corrupt device position generated huge gap");
    PacketClock converted;
    std::uint64_t converted_end=0;
    for(std::uint64_t i=0;i<1000;++i) {
      auto packet=converted.observe(479,start+i*75000,i*160,0,start+(i+1)*100000);
      require(!i||packet.start>=converted_end,"resampled packets reverse committed raw fragments");
      converted.append(i*479,479,packet.start,packet.boundary);
      converted_end=packet.end;
    }
    require(converted.estimated_packets==999,"resampled overlap must be reported as estimated");
    require(converted_end-start==479000ULL*10000000/48000,"resampled PCM changed duration");
    PacketClock different_rate;
    different_rate.begin_stream(false);
    different_rate.observe(480,start,0,0,start+100000);
    auto different=different_rate.observe(480,start,960,0,start+200000);
    require(different.start==start+100000&&std::string(different.source)=="sample_count","native counter at a different rate invented a gap");
    PacketClock frontier;
    frontier.begin_stream();
    auto opening=frontier.observe(480,start,0,0,start+100000);
    frontier.append(0,480,opening.start,opening.boundary);
    // The stream already uses sample fallback, then resumes after 880 ms with
    // a silent/discontinuous packet and a reset device counter (real QA case).
    auto estimated=frontier.observe(480,start+100000,480,4,start+200000);
    frontier.append(480,480,estimated.start,estimated.boundary);
    const auto mixed=start+8800000;
    auto late=frontier.observe(480,start+200000,0,3,mixed+100000,mixed);
    require(late.start==mixed&&late.end==mixed+100000&&late.boundary,"same-stream delayed PCM crossed emitted WAV frontier");
    require(late.frontier_adjustment==mixed-(start+200000)&&std::string(late.correction)=="wav_frontier_rebased","WAV correction lost its measured adjustment");
    require(std::string(late.reason)=="device_position_uncorrelated"&&std::string(late.source)=="sample_count","WAV correction erased the original clock failure");
    auto following=frontier.observe(480,start+300000,480,0,mixed+200000,mixed);
    require(following.start==late.end&&!following.frontier_adjustment,"WAV clock correction was not carried forward exactly once");
    frontier.append(960,480,late.start,late.boundary);
    frontier.append(1440,480,following.start,following.boundary);
    frontier.begin_stream();
    auto fresh=frontier.observe(480,start+400000,0,0,mixed+300000,mixed+200000);
    require(fresh.start==mixed+200000&&std::string(fresh.reason)=="stream_restart_behind_wav_frontier","stale new-stream QPC reopened behind the WAV frontier");
    require(frontier.frontier_rebases==1&&frontier.max_frontier_adjustment_ticks==late.frontier_adjustment,"frontier diagnostics counted a correction twice");
    PacketClock valid_frontier;
    valid_frontier.observe(480,start,0,0,start+100000);
    const auto monotone=valid_frontier.observe(480,start+100000,480,0,mixed+100000,mixed);
    require(monotone.start==mixed&&monotone.end==mixed+100000&&monotone.boundary,"valid monotone QPC entered an already emitted WAV interval");
    require(std::string(monotone.reason)=="none"&&std::string(monotone.source)=="wasapi_qpc"&&monotone.before_frontier==start+100000,"frontier correction replaced valid native timing evidence");
    require(monotone.frontier_adjustment==mixed-(start+100000)&&std::string(monotone.correction)=="wav_frontier_rebased"&&valid_frontier.estimated_packets==1,"valid-QPC correction lacks its explicit estimate");
    for(const auto capacity:{PacketClock::bounded_capacity,PacketClock::recording_capacity}){
      PacketClock bounded(capacity);
      for(std::uint64_t i=0;i<capacity;++i)bounded.append(i*480,480,start+i*400000,true);
      require(bounded.parts.size()==capacity,"clock failed before its selected capacity");
      bool rejected=false;try{bounded.append(capacity*480,480,start+capacity*400000,true);}catch(const std::runtime_error&){rejected=true;}
      require(rejected&&bounded.parts.size()==capacity,"clock capacity overflow changed committed maps");
      require(bounded.parts.back().first+bounded.parts.back().frames==capacity*480,"capacity boundary lost PCM");
    }
    std::cout<<"packet clock: overlap, discontinuities, gaps, clock recovery, bounded 1024 and recording 65536 capacity passed\n";
    return 0;
  } catch(const std::exception& error) {
    std::cerr<<error.what()<<'\n';return 1;
  }
}
