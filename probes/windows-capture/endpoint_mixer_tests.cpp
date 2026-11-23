#include "endpoint_mixer.h"
#include <iostream>
#include <string_view>
void require(bool ok,const char* message){if(!ok)throw std::runtime_error(message);}
int main(){
  try{
    using product::EndpointMixer;using product::EndpointCursor;using product::CapturePacket;
    constexpr std::uint64_t first=480000;
    EndpointMixer mixer;EndpointCursor speaker,bluetooth;std::vector<CapturePacket> output;
    auto emit=[&](CapturePacket packet){output.push_back(std::move(packet));};
    std::vector<float> a(960,0.25f),b(1440,0.5f);
    mixer.add(speaker,first,480,a.data(),0,100000000,emit);
    mixer.add(speaker,first+480,480,nullptr,2,100000000,emit);
    mixer.add(bluetooth,first+240,720,b.data(),0,100000000,emit);
    mixer.flush(first+479,100000000,emit);require(output.empty(),"alignment window closed early");
    mixer.flush(first+960,100000000,emit);
    require(output.size()==2&&output[0].frames==480&&output[1].frames==480,"mixed frame count changed");
    for(unsigned i=0;i<480;++i)for(unsigned ch=0;ch<2;++ch){
      require(output[0].samples[i*2+ch]==(i<240?0.25f:0.75f),"first endpoint mix shifted");
      require(output[1].samples[i*2+ch]==0.5f,"silent endpoint suppressed another output");
    }
    // A late clock moves the complete packet once. Its next packet follows it
    // continuously, rather than losing the same prefix on every callback.
    EndpointCursor returning;output.clear();
    auto late=mixer.add(returning,first+720,480,a.data(),0,100000000,emit);
    require(late.accepted==480&&late.adjusted,"late PCM was shortened");
    mixer.flush(first+1440,100000000,emit);
    require(output.size()==1&&output[0].frames==480&&output[0].position==first+960,"complete late packet missing");
    auto next=mixer.add(returning,first+1200,480,a.data(),0,100000000,emit);
    mixer.flush(first+1920,100000000,emit);
    require(next.accepted==480&&!next.adjusted&&output.back().position==first+1440,"clock correction accumulated per callback");
    require(mixer.diagnostics().late_frames==240&&mixer.diagnostics().retimed_frames==480,"clock correction diagnostics changed");
    auto invalid=a;invalid[0]=std::numeric_limits<float>::quiet_NaN();bool rejected=false;
    try{mixer.add(returning,first,1,invalid.data(),0,100000000,emit);}catch(const std::runtime_error&){rejected=true;}
    require(rejected,"clock correction hid invalid PCM");

    EndpointMixer gaps;EndpointCursor gap_source;output.clear();
    gaps.add(gap_source,first+10,20,a.data(),0,100000000,emit);
    gaps.add(gap_source,first+40,20,nullptr,2,100000000,emit);
    gaps.add(gap_source,first+980,20,a.data(),4,100000000,emit);
    gaps.flush(first+1440,100000000,emit);
    require(output.size()==3&&output[0].position==first+10&&output[1].position==first+40&&output[2].position==first+980,"physical gaps were invented or squeezed out");
    require(output[1].samples.empty()&&(output[1].flags&2)&&output[2].flags==4,"silence or uncertain timestamp lost");
    EndpointMixer clipping;EndpointCursor loud_a,loud_b;output.clear();std::vector<float> loud(960,0.75f);
    clipping.add(loud_a,first,480,loud.data(),0,100000000,emit);
    clipping.add(loud_b,first,480,loud.data(),1,100000000,emit);
    clipping.flush(first+480,100000000,emit);
    require(output[0].flags==1&&output[0].samples[0]==1,"mixed level overflow or discontinuity hidden");

    // Replay the reported 127/480-frame stutter: arrivals are just beyond the
    // 250 ms deadline, and the last 10 ms block is only partly received.
    EndpointMixer continuous;EndpointCursor continuous_source;output.clear();
    std::vector<float> wave(960);constexpr unsigned packets=500;
    for(unsigned i=0;i<packets;++i){
      for(unsigned f=0;f<480;++f)wave[f*2]=wave[f*2+1]=float(int((i*480+f)%257)-128)/1024;
      const auto position=first+353+i*480;
      const auto arrival=EndpointMixer::ticks_at(position+480+EndpointMixer::latency_frames+127);
      continuous.add(continuous_source,position,480,wave.data(),0,arrival,emit);
      auto through=EndpointMixer::frame_at(arrival)-EndpointMixer::latency_frames;
      continuous.flush(continuous_source.safe_through(arrival,through),arrival,emit);
    }
    continuous.flush(std::numeric_limits<std::uint64_t>::max(),200000000,emit);
    std::uint64_t frames=0;
    for(const auto& p:output){
      require(p.position==first+353+frames,"127-frame gap returned");
      for(unsigned f=0;f<p.frames;++f)for(unsigned ch=0;ch<2;++ch)
        require(p.samples[f*2+ch]==float(int((frames+f)%257)-128)/1024,"received waveform changed");
      frames+=p.frames;
    }
    require(frames==packets*480&&continuous.diagnostics().late_frames==0,"continuous PCM was discarded");

    EndpointMixer bounded;EndpointCursor bounded_source;output.clear();
    for(unsigned i=0;i<EndpointMixer::max_blocks+25;++i){
      require(bounded.add(bounded_source,first+i*480,480,a.data(),0,100000000,emit).accepted==480,"pressure lost received PCM");
      require(bounded.pending_blocks()<=EndpointMixer::max_blocks,"alignment memory is unbounded");
    }
    bounded.flush(std::numeric_limits<std::uint64_t>::max(),100000000,emit);
    frames=0;for(const auto& p:output){require(p.position==first+frames,"pressure created a gap");frames+=p.frames;}
    require(frames==125*480&&bounded.diagnostics().pressure_flushes==25&&bounded.diagnostics().reset_frames==0,"pressure discarded a pending buffer");

    // New live-WAV acquisition must not promote buffer pressure to a sealing
    // decision: another endpoint can still have unread older packets.
    EndpointMixer ordered(true);EndpointCursor ordered_source;output.clear();
    for(unsigned i=0;i<EndpointMixer::max_blocks;++i)
      ordered.add(ordered_source,first+i*480,480,a.data(),0,100000000,emit);
    bool pressure_rejected=false;
    try{ordered.add(ordered_source,first+EndpointMixer::max_blocks*480,480,a.data(),0,100000000,emit);}
    catch(const std::runtime_error&){pressure_rejected=true;}
    require(pressure_rejected&&output.empty()&&ordered.pending_blocks()==EndpointMixer::max_blocks,
      "ordered acquisition sealed unread peer audio under pressure");

    // A control timer can see an empty output FIFO while native alignment
    // still retains an older partial block. Only the acquisition observation
    // may move the DSP frontier, and it stops before that unread PCM.
    EndpointMixer observed(true);EndpointCursor observed_source;product::PacketQueue ingress;
    auto admit=[&](CapturePacket packet){ingress.push(std::move(packet));};
    observed.add(observed_source,first+353,480,a.data(),0,100000000,admit);
    observed.flush(first+480,100000000,admit);
    // The first 127 frames are emitted; the remaining 353 are still held.
    auto preceding=ingress.pop();require(preceding&&preceding->frames==127,"partial-block fixture changed");
    require(!ingress.pop()&&observed.pending_blocks()==1,"fixture must have an empty FIFO and buffered PCM");
    const auto timer=first+48000;
    ingress.observe(EndpointMixer::ticks_at(observed.observed_through(timer)));
    require(ingress.observed_through(preceding->sequence)==EndpointMixer::ticks_at(first+480),
      "timer observation crossed PCM retained inside the endpoint mixer");
    observed.flush(timer,100000000,admit);
    ingress.observe(EndpointMixer::ticks_at(observed.observed_through(timer)));
    require(!ingress.observed_through(preceding->sequence),"frontier overtook buffered PCM after native flush");
    const auto remaining=ingress.pop();require(remaining&&remaining->frames==353&&remaining->position==first+480
      &&ingress.observed_through(remaining->sequence)==EndpointMixer::ticks_at(timer),"ordered frontier lost buffered PCM");
    const auto recovered=observed.add(observed_source,first+353,480,a.data(),0,100000000,admit);
    observed.flush(timer+480,100000000,admit);
    const auto recovered_packet=ingress.pop();
    require(recovered.accepted==480&&recovered.adjusted&&recovered_packet
      &&recovered_packet->position==timer&&recovered_packet->frames==480
      &&(recovered_packet->flags&4)&&!ingress.pop(),
      "late live acquisition entered the published frontier or lost received PCM");
    for(float sample:recovered_packet->samples)require(sample==0.25f,"late live PCM waveform changed");
    const auto& recovered_diagnostics=observed.diagnostics();
    require(recovered_diagnostics.retimed_frames==480&&recovered_diagnostics.losses.size()==1
      &&recovered_diagnostics.losses[0].first==first+353
      &&recovered_diagnostics.losses[0].mapped_first==timer
      &&recovered_diagnostics.losses[0].frames==480
      &&std::string_view(recovered_diagnostics.losses[0].reason)=="clock_rebased",
      "late live acquisition lost its original and corrected clock evidence");

    // Repeated Bluetooth generations may reuse an old QPC origin after an
    // idle observation has already sealed later audio. Keep all their PCM,
    // leave earlier output intact, and adjust each new native clock once.
    EndpointMixer reconnecting(true);EndpointCursor initial;output.clear();
    reconnecting.add(initial,first,480,a.data(),0,100000000,emit);
    reconnecting.flush(first+480,100000000,emit);
    const auto committed=output.front();
    constexpr unsigned generations=12,packets_per_generation=3;
    for(unsigned generation=0;generation<generations;++generation){
      EndpointCursor reconnected;
      const auto frontier=first+48000ULL*(generation+1);
      require(reconnecting.observed_through(frontier)==frontier,"empty native frontier did not advance");
      const auto original=first+240;
      for(unsigned packet=0;packet<packets_per_generation;++packet){
        for(unsigned frame=0;frame<480;++frame)for(unsigned channel=0;channel<2;++channel)
          wave[frame*2+channel]=float(1+generation*packets_per_generation+packet)/64;
        const auto result=reconnecting.add(reconnected,original+packet*480,480,wave.data(),0,100000000,emit);
        require(result.accepted==480&&result.adjusted==(packet==0),"reconnection shortened PCM or repeatedly adjusted its clock");
        reconnecting.flush(frontier+(packet+1)*480,100000000,emit);
        const auto& saved=output.back();
        require(saved.position==frontier+packet*480&&saved.frames==480&&(saved.flags&4),
          "reconnected output crossed its frontier or hid estimated timing");
        require(saved.samples==wave,"reconnected PCM waveform changed");
        require(reconnecting.pending_blocks()<=EndpointMixer::max_blocks,"reconnected output exceeded bounded alignment storage");
      }
      const auto& correction=reconnecting.diagnostics().losses.back();
      require(correction.first==original&&correction.mapped_first==frontier&&correction.frames==480,
        "reconnection diagnostics did not preserve both clock positions");
      require(reconnected.correction==frontier-original,"reconnection changed the correction after its first packet");
    }
    require(output.size()==1+generations*packets_per_generation&&output.front().position==committed.position
      &&output.front().samples==committed.samples&&output.front().flags==committed.flags,
      "reconnection rewrote committed output or lost a packet");
    require(reconnecting.diagnostics().retimed_packets==generations
      &&reconnecting.diagnostics().retimed_frames==generations*480
      &&reconnecting.diagnostics().losses.size()==generations
      &&reconnecting.diagnostics().pressure_flushes==0,
      "reconnection corrections were not bounded and recorded once per native clock");
    EndpointCursor invalid_connection;bool invalid_pcm=false,invalid_geometry=false;
    try{reconnecting.add(invalid_connection,first,480,invalid.data(),0,100000000,emit);}
    catch(const product::InputDataFault&){invalid_pcm=true;}
    try{reconnecting.add(invalid_connection,first,48001,a.data(),0,100000000,emit);}
    catch(const product::InputDataFault&){invalid_geometry=true;}
    require(invalid_pcm&&invalid_geometry&&reconnecting.pending_blocks()==0
      &&output.size()==1+generations*packets_per_generation
      &&reconnecting.diagnostics().retimed_packets==generations,
      "clock recovery accepted invalid PCM or geometry or changed accepted output");

    // An idle peer does not block progress forever. When it returns late, all
    // its received samples are still represented, with explicitly estimated time.
    EndpointMixer idle;EndpointCursor active,peer;output.clear();
    idle.add(peer,first,480,a.data(),0,EndpointMixer::ticks_at(first+480),emit);
    for(unsigned i=0;i<120;++i){
      const auto arrival=EndpointMixer::ticks_at(first+(i+1)*480);
      idle.add(active,first+i*480,480,nullptr,2,arrival,emit);
      auto through=first+(i+1)*480;
      through=active.safe_through(arrival,through);through=peer.safe_through(arrival,through);
      idle.flush(through,arrival,emit);
    }
    require(idle.pending_blocks()<=EndpointMixer::max_blocks,"idle peer stranded an active output");
    require(idle.add(peer,first+480,480,a.data(),0,EndpointMixer::ticks_at(first+120*480),emit).accepted==480,"returning peer PCM lost");
    idle.flush(std::numeric_limits<std::uint64_t>::max(),200000000,emit);
    double signal=0;for(const auto& p:output)for(float sample:p.samples)signal+=sample;
    require(signal==480*2*0.25*2,"returning endpoint's signal energy was discarded");
    require(idle.diagnostics().retimed_packets==1,"returning clock did not rebase once");

    for(std::uint64_t frame:{0ULL,1ULL,479ULL,480ULL,480001ULL,480000000000ULL})
      require(EndpointMixer::frame_at(EndpointMixer::ticks_at(frame))==frame,"QPC conversion lost sample position");
    std::cout<<"endpoint mixer: exact waveform, 127-frame replay, complete late PCM, bounded pressure, idle return, gaps and clipping passed\n";
    return 0;
  }catch(const std::exception& error){std::cerr<<error.what()<<'\n';return 1;}
}
