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
  for(const auto* stage:{"pcm_write","pcm_sync","pcm_verify","receipt","tip","close"})failed_checkpoint_preserves_prior_prefix(stage);
  std::cout<<"live WAV native: exact PCM, single file, reservations, checkpoint failures, 100-second gated storage, closure and test WAV passed\n";
  return 0;
}catch(const std::exception& error){std::cerr<<error.what()<<'\n';return 1;}}
