// Exercise the actual native append-only writer and the Rust mixer without
// requesting microphone/loopback permissions or opening any audio hardware.
#define wmain unused_capture_entry
#include "main.cpp"
#undef wmain
#include <fstream>

static void require(bool ok,const char* message){if(!ok)throw std::runtime_error(message);}
struct Directory {
  fs::path path=fs::temp_directory_path()/(L"mon-live-wav-"+std::to_wstring(GetCurrentProcessId())+L"-"+std::to_wstring(product::qpc()));
  Directory(){if(!fs::create_directory(path))throw std::runtime_error("test directory exists");}
  ~Directory(){std::error_code error;fs::remove_all(path,error);}
};
static std::vector<BYTE> bytes(const fs::path& path){std::ifstream file(path,std::ios::binary);if(!file)throw std::runtime_error("saved fixture missing");return {std::istreambuf_iterator<char>(file),{}};}
static product::Json json(const fs::path& path){const auto value=bytes(path);return product::Json::parse(value.begin(),value.end());}
static constexpr auto recording_owner="735027d4-a03e-43d3-830a-21e767dfaf88";

static void feed(product::LiveWav& recording,const std::shared_ptr<product::StoragePending>& pending,unsigned seconds){
  std::vector<std::int16_t> pcm(160,10000);
  for(unsigned packet=0;packet<seconds*100;++packet){
    const auto start=1000000000LL+packet*10000000LL,end=start+10000000LL;
    for(unsigned lane=0;lane<2;++lane){const auto source_end=pending->acquire(lane,480);recording.push(lane,1,pcm.data(),pcm.size(),start,end,source_end);}
    recording.advance(0,end);recording.advance(1,end);recording.drain(end);
  }
}

static void slow_storage_keeps_dsp_independent(){
  Directory directory;auto pending=std::make_shared<product::StoragePending>();
  std::mutex mutex;std::condition_variable changed;bool entered=false,released=false;
  product::LiveWav recording(directory.path,recording_owner,pending,[&](const char* stage,std::uint64_t){
    if(std::string_view(stage)!="pcm_write")return;
    std::unique_lock lock(mutex);if(released)return;entered=true;changed.notify_all();
    if(!changed.wait_for(lock,std::chrono::seconds(30),[&]{return released;}))throw std::runtime_error("test storage gate timed out");
  });
  feed(recording,pending,100);
  {std::unique_lock lock(mutex);require(changed.wait_for(lock,std::chrono::seconds(5),[&]{return entered;}),"WAV IO worker missed gate");}
  const auto snapshot=pending->snapshot();
  require(snapshot.frames[0]==48000*100&&snapshot.frames[1]==48000*100,"dequeued WAV job released unsynced source audio");
  require(snapshot.bytes<=mon_capture_limit(11)&&snapshot.jobs==26,"slow storage escaped bounded compact queues");
  require(recording.status()["frames"]==0,"unwritten PCM was advertised durable");
  {std::lock_guard lock(mutex);released=true;changed.notify_all();}
  recording.finish();
  const auto done=pending->snapshot();require(!done.frames[0]&&!done.frames[1]&&!done.bytes&&!done.jobs,"durable WAV did not release storage credits");
}

static void failed_checkpoint_preserves_prior_prefix(const char* fault){
  Directory directory;auto pending=std::make_shared<product::StoragePending>();bool failed=false;
  {
    product::LiveWav recording(directory.path,recording_owner,pending,[&](const char* stage,std::uint64_t frames){
      if(frames>=128000&&std::string_view(stage)==fault)throw std::runtime_error("injected WAV checkpoint failure");
    });
    try{feed(recording,pending,8);recording.finish();}catch(const std::exception&){failed=true;}
    try{recording.wait();}catch(const std::exception&){failed=true;}
    try{recording.preserve_prefix();}catch(const std::exception&){failed=true;}
  }
  const auto tip=json(directory.path/L"single-wav.json");
  const auto expected=std::string_view(fault)=="close"?128000:64000;
  require(failed&&tip["frames"]==expected&&tip["producerClosed"]==false,"failed WAV write advanced or closed unverified audio");
  require(fs::exists(directory.path/L"single-wav-owner.json")&&fs::exists(directory.path/L"recording.wav")
    &&fs::exists(directory.path/L"wav-00000001.json"),"WAV failure deleted recovery evidence");
}

static void fractional_lane_boundaries_keep_every_sample(){
  Directory directory;auto pending=std::make_shared<product::StoragePending>();
  auto recording=std::make_shared<product::LiveWav>(directory.path,recording_owner,pending);
  product::Lane lane(directory.path/L"microphone.wav",1,false);
  lane.wav=std::make_unique<product::WavInput>(lane.path,0,1,recording);
  std::vector<float> packet(479,0.125f);constexpr std::uint64_t origin=10000000;
  for(std::uint64_t index=0;index<500;++index){
    lane.append(packet.data(),479,origin+index*75000,index*160,0,origin+(index+1)*100000);
    const auto through=lane.wav->normalized_through();
    recording->advance(0,through);recording->advance(1,through);recording->drain(through);
  }
  lane.close();const auto end=std::int64_t((origin+500ULL*479*10000000/48000)*100);
  recording->advance(0,end);recording->advance(1,end);recording->drain(end);recording->finish();
  require(lane.wav->frames==79834&&recording->status()["frames"]==79834,"fractional lane lost normalized PCM");
  require(recording->origin()==std::int64_t(origin*100),"fractional lane moved capture origin");
  const auto saved=bytes(directory.path/L"recording.wav");
  require(saved.size()==80+79834*2,"fractional WAV checkpoint/tail geometry changed");
  for(std::size_t i=80;i<saved.size();i+=2){std::int16_t sample;std::memcpy(&sample,saved.data()+i,2);require(sample==4096,"fractional lane reordered or changed PCM");}
  const auto proof=json(directory.path/L"wav-00000002.json");
  require(proof["frameStart"]==64000&&proof["frames"]==15834&&proof["coverageFrames"][0]==15834,"fractional durable coverage lost tail");
  Directory clipped_directory;product::Lane clipped(clipped_directory.path/L"microphone.wav",1,true);
  clipped.wav=std::make_unique<product::WavInput>(clipped.path,0,1);
  clipped.window_start=origin+100;clipped.window_end=origin+49999999;
  for(std::uint64_t index=0;index<502;++index)
    clipped.append(packet.data(),479,origin+index*75000,index*160,0,origin+(index+1)*100000);
  clipped.close();
  require(clipped.first==origin+99791/479,"fractional test window moved the first accepted sample");
  require(clipped.clock.parts.front().start>=clipped.window_start&&clipped.clock.parts.back().last_end<=clipped.window_end,"fractional test window escaped measured packet bounds");
}

static void unproven_stop_commits_the_accepted_tail(){
  Directory directory;auto pending=std::make_shared<product::StoragePending>();
  product::LiveWav recording(directory.path,recording_owner,pending);
  feed(recording,pending,1);recording.preserve_prefix();recording.preserve_prefix();
  const auto tip=json(directory.path/L"single-wav.json");
  require(tip["frames"]==16000&&tip["checkpoints"]==1&&tip["producerClosed"]==false,"partial stop lost the short accepted tail or invented closure");
  const auto data=bytes(directory.path/L"recording.wav");
  require(data.size()==80+32000,"partial stop changed accepted WAV geometry");
  require(fs::exists(directory.path/L"wav-00000001.json"),"partial stop did not publish a durable receipt");
}

static void auxiliary_clock_capacity_does_not_reject_shared_pcm(){
  Directory directory;auto pending=std::make_shared<product::StoragePending>();
  auto recording=std::make_shared<product::LiveWav>(directory.path,recording_owner,pending);
  product::WavInput input(directory.path/L"microphone.wav",0,1,recording,1);
  std::vector<float> pcm(4800,0.25f);
  input.append(pcm.data(),4800,10000000,11000000,false,0);
  input.append(pcm.data(),4800,12000000,13000000,true,0);
  input.finish();recording->finish();
  require(input.frames==3200&&!input.clock_error().empty(),"auxiliary map capacity discarded valid normalized PCM");
  require(recording->status()["frames"]==4800&&recording->status()["producerClosed"]==true,"auxiliary map failure prevented shared WAV completion");
}

class ManualWavReader final : public product::CaptureReader {
 public: void stop()override{}
};
struct ReconnectWav {
  static constexpr std::uint64_t origin=100000000,packet_ticks=100000;
  Directory directory;
  std::shared_ptr<product::StoragePending> pending=std::make_shared<product::StoragePending>();
  std::shared_ptr<product::LiveWav> recording=std::make_shared<product::LiveWav>(directory.path,recording_owner,pending);
  product::Lane mic{directory.path/L"microphone.wav",1},pc{directory.path/L"system.wav",2};
  std::array<std::uint64_t,2> position{},packets{};
  std::vector<unsigned char> expected;
  ReconnectWav(){
    mic.wav=std::make_unique<product::WavInput>(mic.path,0,1,recording);
    pc.wav=std::make_unique<product::WavInput>(pc.path,1,2,recording);
    bind(mic);bind(pc);
  }
  void bind(product::Lane& lane){
    product::PreparedInput prepared;prepared.matching_position_rate=true;prepared.reader=std::make_unique<ManualWavReader>();
    lane.adopt(std::move(prepared));position[lane.channels==1?0:1]=0;
  }
  void feed(product::Lane& lane,std::uint64_t raw,std::uint64_t arrival,std::uint64_t mapped,DWORD extra_flags=0,bool reset_position=false){
    const unsigned index=lane.channels==1?0:1;
    if(reset_position)position[index]=0;
    product::CapturePacket packet;packet.frames=480;packet.time=raw;packet.arrival=arrival;packet.position=position[index];
    packet.flags=extra_flags|(index?AUDCLNT_BUFFERFLAGS_SILENT:0);
    if(!index)packet.samples.assign(480,0.25f);
    lane.reader->queue.push(std::move(packet));lane.pump();position[index]+=480;++packets[index];
    lane.reader->queue.observe(mapped+packet_ticks);
    require(lane.recent.back()["mappedStart100ns"]==mapped&&lane.recent.back()["mappedEnd100ns"]==mapped+packet_ticks,"reconnect changed an expected full-packet map");
    const auto first=(mapped-origin)/625,last=first+160;
    require(mapped>=origin&&(mapped-origin)%625==0,"test PCM does not lie on normalized sample grid");
    if(expected.size()<last)expected.resize(static_cast<std::size_t>(last));
    for(auto frame=first;frame<last;++frame){require(!(expected[frame]&(1u<<index)),"source PCM overlapped itself");expected[frame]|=1u<<index;}
  }
  void flush(std::uint64_t now){
    recording->advance(0,mic.wav->normalized_through());recording->advance(1,pc.wav->normalized_through());recording->drain(static_cast<std::int64_t>(now*100));
  }
  void idle(product::Lane& lane,std::uint64_t through){
    if(auto frontier=lane.idle_frontier(through)){
      lane.wav->drain_converter();recording->advance(lane.channels==1?0:1,std::max(lane.wav->normalized_through(),static_cast<std::int64_t>(*frontier*100)));
    }
  }
  void retire(product::Lane& lane){
    lane.acquisition_failed("device_disconnected");
    const auto deadline=GetTickCount64()+5000;
    while(lane.retirement&&GetTickCount64()<deadline){lane.poll_retirement();Sleep(1);}
    require(!lane.retirement&&!lane.reader,"native retirement did not release its source");
  }
  void finish(){
    mic.close();pc.close();recording->finish();
    require(mic.wav->frames==packets[0]*160&&pc.wav->frames==packets[1]*160,"Bluetooth cycles lost accepted normalized source samples");
    require(recording->status()["frames"]==expected.size()&&recording->status()["producerClosed"]==true,"Bluetooth cycles changed the finalized WAV length or closure");
    const auto saved=bytes(directory.path/L"recording.wav");require(saved.size()==80+expected.size()*2,"Bluetooth WAV geometry lost PCM");
    std::array<std::uint64_t,2> wanted_coverage{},saved_coverage{};
    for(std::size_t frame=0;frame<expected.size();++frame){
      std::int16_t value;std::memcpy(&value,saved.data()+80+frame*2,2);
      const auto wanted=expected[frame]==1?8192:expected[frame]==3?4096:0;
      require(value==wanted,"Bluetooth cycles changed, dropped or reordered exact mixed PCM");
      for(unsigned lane=0;lane<2;++lane)wanted_coverage[lane]+=(expected[frame]&(1u<<lane))!=0;
    }
    const auto checkpoints=recording->status()["checkpoints"].get<unsigned>();
    for(unsigned checkpoint=1;checkpoint<=checkpoints;++checkpoint){
      std::ostringstream name;name<<"wav-"<<std::setw(8)<<std::setfill('0')<<checkpoint<<".json";
      const auto receipt=json(directory.path/product::wide(name.str()));
      for(unsigned lane=0;lane<2;++lane)saved_coverage[lane]+=receipt["coverageFrames"][lane].get<std::uint64_t>();
    }
    require(saved_coverage==wanted_coverage,"Bluetooth WAV dropped or fabricated silent source coverage");
    const auto left=pending->snapshot();require(!left.frames[0]&&!left.frames[1]&&!left.bytes&&!left.jobs,"Bluetooth WAV recovery leaked storage reservations");
  }
};

static void repeated_microphone_gaps_and_reconnects_preserve_live_wav(){
  ReconnectWav test;std::uint64_t pc_time=ReconnectWav::origin;
  auto pc_tick=[&]{test.feed(test.pc,pc_time,pc_time+100000,pc_time);pc_time+=100000;test.flush(pc_time);};
  for(unsigned cycle=0;cycle<12;++cycle){
    if(cycle)test.bind(test.mic);
    const auto first=pc_time;test.feed(test.mic,first,first+100000,first);pc_tick();
    // A successful but empty microphone for 880 ms must not authorize a timer
    // barrier. The same native stream then returns its delayed next packet.
    for(unsigned packet=0;packet<88;++packet)pc_tick();
    test.mic.reader->queue.observe(pc_time-2500000);
    require(!test.mic.idle_frontier(pc_time-2500000),"empty live microphone advanced over delayed PCM");
    test.idle(test.mic,pc_time-2500000);test.flush(pc_time);
    test.feed(test.mic,first+100000,pc_time,first+100000);test.flush(pc_time);
    test.retire(test.mic);
    // Only actual native closure permits the absent input to advance while
    // healthy PC audio continues. Reopen with a stale but increasing QPC.
    for(unsigned packet=0;packet<50;++packet){pc_tick();test.idle(test.mic,pc_time-2500000);test.flush(pc_time);}
    const auto floor=pc_time-2500000;
    require(test.recording->mixed_through()==static_cast<std::int64_t>(floor*100),"absent-input test failed to emit its expected frontier");
    test.bind(test.mic);
    test.feed(test.mic,first+200000,floor+100000,floor);
    require(test.mic.recent.back()["reason"]=="stream_restart_behind_wav_frontier"&&test.mic.recent.back()["rawQpc100ns"]==first+200000,"new microphone generation lost its original stale clock diagnostic");
    test.feed(test.mic,first+300000,floor+200000,floor+100000);test.flush(pc_time);
    require(test.pc.binding.state=="connected"&&test.mic.clock.estimated_packets>0,"microphone reacquisition interrupted healthy PC audio or hid estimated timing");
    test.retire(test.mic);
  }
  test.finish();
}

static void repeated_same_stream_system_gaps_preserve_live_wav(){
  ReconnectWav test;std::uint64_t mic_time=ReconnectWav::origin;
  auto mic_tick=[&]{test.feed(test.mic,mic_time,mic_time+100000,mic_time);mic_time+=100000;test.flush(mic_time);};
  test.feed(test.pc,mic_time,mic_time+100000,mic_time);mic_tick();
  test.feed(test.pc,mic_time,mic_time+100000,mic_time,AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR);mic_tick();
  auto previous_end=mic_time;
  for(unsigned cycle=0;cycle<12;++cycle){
    for(unsigned packet=0;packet<88;++packet)mic_tick();
    const auto floor=mic_time-2500000;
    test.pc.reader->queue.observe(floor);test.idle(test.pc,floor);test.flush(mic_time);
    require(test.recording->mixed_through()==static_cast<std::int64_t>(floor*100),"system-gap test failed to emit its expected frontier");
    const auto raw=mic_time-100000;
    test.feed(test.pc,raw,mic_time,floor,AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY,true);
    const auto& diagnostic=test.pc.recent.back();
    require(diagnostic["streamEpoch"]==1&&diagnostic["reason"]=="device_position_uncorrelated"&&diagnostic["source"]=="sample_count","system recovery hid its same-generation clock fault");
    require(diagnostic["correctionReason"]=="wav_frontier_rebased"&&diagnostic["beforeFrontierStart100ns"]==previous_end&&diagnostic["frontierAdjustment100ns"]==floor-previous_end,"system recovery lost its exact frontier adjustment");
    require(test.mic.binding.state=="connected"&&test.pc.clock.frontier_rebases==cycle+1,"system delivery gap restarted the healthy microphone or repeated its correction");
    previous_end=floor+100000;test.flush(mic_time);
  }
  test.finish();
}

static void valid_qpc_frontier_correction_remains_in_diagnostics(){
  ReconnectWav test;std::uint64_t mic_time=ReconnectWav::origin;
  auto mic_tick=[&]{test.feed(test.mic,mic_time,mic_time+100000,mic_time);mic_time+=100000;test.flush(mic_time);};
  test.feed(test.pc,mic_time,mic_time+100000,mic_time);mic_tick();
  for(unsigned packet=0;packet<88;++packet)mic_tick();
  const auto floor=mic_time-2500000,raw=ReconnectWav::origin+100000;
  test.pc.reader->queue.observe(floor);test.idle(test.pc,floor);test.flush(mic_time);
  test.feed(test.pc,raw,mic_time,floor);
  const auto corrected=test.pc.recent.back();
  require(corrected["streamEpoch"]==1&&corrected["reason"]=="none"&&corrected["source"]=="wasapi_qpc","valid delayed QPC was misreported as a native clock fault");
  require(corrected["rawQpc100ns"]==raw&&corrected["beforeFrontierStart100ns"]==raw&&corrected["mappedStart100ns"]==floor&&corrected["frontierAdjustment100ns"]==floor-raw&&corrected["correctionReason"]=="wav_frontier_rebased","valid-QPC correction lost its original and corrected timing evidence");
  for(unsigned packet=1;packet<=35;++packet){
    const auto next=floor+packet*100000;
    test.feed(test.pc,next,next+100000,next);test.flush(next+100000);
    require(test.pc.recent.back()["reason"]=="none"&&test.pc.recent.back()["frontierAdjustment100ns"]==0,"normal packets reapplied a frontier correction");
  }
  const auto diagnostic=test.pc.diagnostics();
  require(diagnostic["recentPackets"].size()==32&&diagnostic["recentPackets"][0]["packet"].get<unsigned>()>corrected["packet"].get<unsigned>(),"diagnostic regression did not age the corrected packet out of recent history");
  require(diagnostic["firstAnomalies"].size()==1&&diagnostic["firstAnomalies"][0]==corrected,"valid-QPC frontier correction disappeared after the recent packet window");
  require(diagnostic["wavFrontierRebases"]==1&&diagnostic["timingEstimated"]==true,"valid-QPC correction lost its persistent estimate summary");
  test.finish();
}

static void stop_waits_for_cancelled_preparation_cleanup(){
  Directory directory;product::Lane lane(directory.path/L"microphone.wav",1);
  struct Counts {std::atomic<unsigned> disposed{0};std::atomic<bool> entered{false};};
  struct Reader final:product::CaptureReader{
    std::shared_ptr<Counts> counts;explicit Reader(std::shared_ptr<Counts> value):counts(std::move(value)){}
    void stop()override{}~Reader(){++counts->disposed;}
  };
  auto counts=std::make_shared<Counts>();
  lane.preparation_schedule.begin(GetTickCount64());
  lane.preparation=std::make_unique<product::NativePreparation>([counts](auto cancelled){
    counts->entered=true;
    const auto deadline=GetTickCount64()+5000;
    while(!cancelled()&&GetTickCount64()<deadline)Sleep(1);
    require(cancelled(),"stop did not cancel the outstanding native preparation");
    Sleep(40);product::PreparedInput ready;ready.reader=std::make_unique<Reader>(counts);return ready;
  });
  const auto deadline=GetTickCount64()+5000;while(!counts->entered&&GetTickCount64()<deadline)Sleep(1);
  require(counts->entered,"native preparation did not enter controlled wait");
  lane.close();
  require(counts->disposed==1&&lane.acquisition_closed()&&!lane.reader&&!lane.started,"stop returned before cancelled native disposal or adopted a late reader");
}

static void explicit_stop_bounds_bluetooth_cleanup_and_preserves_tail(){
  struct Gate {
    std::mutex mutex;std::condition_variable changed;bool entered=false,open=false;
    std::atomic<unsigned> disposed{0};
    void wait(){std::unique_lock lock(mutex);entered=true;changed.notify_all();changed.wait(lock,[&]{return open;});}
    void release(){{std::lock_guard lock(mutex);open=true;}changed.notify_all();}
    bool blocked(){std::lock_guard lock(mutex);return entered&&!open;}
  };
  struct Reader final:product::CaptureReader{
    std::shared_ptr<Gate> gate;explicit Reader(std::shared_ptr<Gate> value):gate(std::move(value)){}
    void stop()override{gate->wait();}~Reader(){++gate->disposed;}
  };
  auto gate=std::make_shared<Gate>();
  std::jthread watchdog([gate]{std::unique_lock lock(gate->mutex);gate->changed.wait_for(lock,std::chrono::seconds(5),[&]{return gate->open;});lock.unlock();gate->release();});
  struct Release {std::shared_ptr<Gate> gate;~Release(){gate->release();}} release{gate};
  ReconnectWav test;test.retire(test.mic);
  product::PreparedInput ready;ready.reader=std::make_unique<Reader>(gate);test.mic.adopt(std::move(ready));
  test.feed(test.mic,ReconnectWav::origin,ReconnectWav::origin+100000,ReconnectWav::origin);
  test.feed(test.pc,ReconnectWav::origin,ReconnectWav::origin+100000,ReconnectWav::origin);
  const auto history=test.mic.binding.history();
  // Session shutdown initiates both owners before waiting on either one.
  test.mic.stop_acquisition();test.pc.stop_acquisition();
  const auto entered_by=GetTickCount64()+3000;while(!gate->blocked()&&GetTickCount64()<entered_by)Sleep(1);
  require(gate->blocked()&&!test.mic.reader&&!test.pc.reader,"explicit stop did not start both owned retirements");
  const auto started=GetTickCount64();bool unproven=false;
  try{test.mic.close(40);}catch(const std::runtime_error&){unproven=true;}
  require(unproven&&GetTickCount64()-started<2000&&!test.mic.acquisition_closed(),"Bluetooth Stop bypassed the bounded closure wait");
  test.pc.close();
  require(test.pc.acquisition_closed()&&gate->blocked(),"blocked Bluetooth Stop prevented the other lane from closing");
  require(test.mic.binding.history()==history,"explicit user stop fabricated an input-disconnection event");
  test.recording->preserve_prefix();
  require(test.recording->status()["frames"]==160&&test.recording->status()["producerClosed"]==false&&test.mic.wav->frames==160,"unproven stop dropped accepted converter tail or advertised closure");
  gate->release();watchdog.join();
  const auto closed_by=GetTickCount64()+5000;
  while(test.mic.retirement&&GetTickCount64()<closed_by){test.mic.poll_retirement();Sleep(1);}
  require(!test.mic.retirement&&gate->disposed==1,"completed Stop retained its reader or disposed it before native cleanup");
  test.finish();

  // A timed-out owner may be destroyed while native Stop is still pending.
  // Its worker holds only the source, and releases it after real completion.
  auto late=std::make_shared<Gate>();
  {
    product::Lane lane(test.directory.path/L"not-opened.wav",1);
    product::PreparedInput input;input.reader=std::make_unique<Reader>(late);lane.adopt(std::move(input));
    lane.stop_acquisition();bool pending=false;
    try{lane.close(10);}catch(const std::runtime_error&){pending=true;}
    require(pending&&!lane.acquisition_closed()&&!late->disposed,"late Stop incorrectly proved native disposal");
  }
  require(!late->disposed,"destroying Lane disposed a source still inside Stop");
  late->release();
  const auto disposed_by=GetTickCount64()+5000;while(!late->disposed&&GetTickCount64()<disposed_by)Sleep(1);
  require(late->disposed==1,"owned Stop worker outlived Lane without safely releasing its source");
}

int main(){try{
  using namespace product;constexpr auto owner="735027d4-a03e-43d3-830a-21e767dfaf88";
  Directory directory;auto pending=std::make_shared<StoragePending>();
  auto recording=std::make_shared<LiveWav>(directory.path,owner,pending);
  auto mic=std::make_unique<WavInput>(directory.path/L"microphone.wav",0,1,recording);
  auto pc=std::make_unique<WavInput>(directory.path/L"system.wav",1,2,recording);
  std::vector<float> microphone(480,0.25f),system(960,0.75f);
  constexpr std::uint64_t origin=100000000ULL;
  for(unsigned packet=0;packet<800;++packet){
    const auto time=origin+packet*100000ULL;
    mic->append(microphone.data(),480,time,time+100000,false,0);pc->append(system.data(),480,time,time+100000,false,0);
    const auto through=std::min(mic->normalized_through(),pc->normalized_through());
    recording->advance(0,through);recording->advance(1,through);recording->drain(through);
  }
  mic->finish();pc->finish();
  const auto close=static_cast<std::int64_t>((origin+800*100000ULL)*100);
  recording->advance(0,close);recording->advance(1,close);recording->drain(close);recording->finish();
  require(recording->status()["producerClosed"]==true,"WAV closure was not durable");
  require(mic->frames==128000&&pc->frames==128000,"normalized source tail missing");
  const auto budget=pending->snapshot();
  require(!budget.frames[0]&&!budget.frames[1]&&!budget.bytes&&!budget.jobs,"durable WAV left source reservations");
  mic.reset();pc.reset();recording.reset();
  const auto saved=bytes(directory.path/L"recording.wav"),header=VoicePCM::wave_header(128000);
  require(saved.size()==80+128000*2&&std::equal(header.begin(),header.end(),saved.begin()),"single WAV geometry invalid");
  for(std::size_t i=80;i<saved.size();i+=2){std::int16_t sample;std::memcpy(&sample,saved.data()+i,2);require(sample==16384,"live average changed PCM");}
  const auto tip=json(directory.path/L"single-wav.json");
  require(tip["frames"]==128000&&tip["checkpoints"]==2&&tip["producerClosed"]==true,"checkpoint prefix inconsistent");
  std::string previous;
  for(unsigned sequence=1;sequence<=2;++sequence){
    std::ostringstream name;name<<"wav-"<<std::setw(8)<<std::setfill('0')<<sequence<<".json";
    const auto encoded=bytes(directory.path/wide(name.str()));const auto proof=Json::parse(encoded.begin(),encoded.end());
    require(proof["sequence"]==sequence&&proof["frameStart"]==(sequence-1)*64000&&proof["frames"]==64000,"checkpoint order changed");
    require(proof["pcmSHA256"]==sha256_bytes(saved.data()+80+(sequence-1)*128000,128000),"checkpoint PCM hash mismatch");
    require(sequence==1?proof["previousEntrySHA256"].is_null():proof["previousEntrySHA256"]==previous,"checkpoint chain changed");
    previous=sha256_bytes(encoded.data(),encoded.size());
  }
  require(tip["lastEntrySHA256"]==previous,"durable tip did not reference exact receipt bytes");
  const auto reservation=json(directory.path/L"single-wav-owner.json");require(reservation["ownerId"]==owner&&reservation["fileName"]=="recording.wav","WAV owner reservation missing");
  for(const auto& item:fs::directory_iterator(directory.path))require(item.path().extension()!=L".caf"&&item.path().extension()!=L".flac","normal capture produced intermediate audio");
  Directory test_directory;
  WavInput test(test_directory.path/L"microphone.wav",0,1);test.append(microphone.data(),480,origin,origin+100000,false,0);test.finish();
  const auto test_audio=bytes(test_directory.path/L"microphone.wav");require(test_audio.size()==80+160*2&&test_audio[0]=='R',"five-second lane did not write compact WAV");
  slow_storage_keeps_dsp_independent();
  unproven_stop_commits_the_accepted_tail();
  auxiliary_clock_capacity_does_not_reject_shared_pcm();
  fractional_lane_boundaries_keep_every_sample();
  repeated_microphone_gaps_and_reconnects_preserve_live_wav();
  repeated_same_stream_system_gaps_preserve_live_wav();
  valid_qpc_frontier_correction_remains_in_diagnostics();
  stop_waits_for_cancelled_preparation_cleanup();
  explicit_stop_bounds_bluetooth_cleanup_and_preserves_tail();
  for(const auto* stage:{"pcm_write","pcm_sync","receipt","tip","close"})failed_checkpoint_preserves_prior_prefix(stage);
  std::cout<<"live WAV native: exact PCM, single file, reservations, checkpoint failures, 100-second gated storage, closure and test WAV passed\n";
  return 0;
}catch(const std::exception& error){std::cerr<<error.what()<<'\n';return 1;}}
