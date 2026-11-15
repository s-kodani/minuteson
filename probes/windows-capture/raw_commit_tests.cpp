// The actual product assembler, ingress and disk owner, with deterministic PCM
// and an injected filesystem gate/fault. No audio hardware or UI is involved.
#define wmain unused_capture_entry
#include "main.cpp"
#undef wmain
#include <fstream>

void require(bool ok,const char* message){if(!ok)throw std::runtime_error(message);}
struct Temporary {
  fs::path path=fs::temp_directory_path()/(L"mon-raw-commit-"+std::to_wstring(GetCurrentProcessId())+L"-"+std::to_wstring(product::qpc()));
  Temporary(){if(!path.is_absolute()||!fs::create_directory(path))throw std::runtime_error("test directory already exists");}
  ~Temporary(){std::error_code ignored;fs::remove_all(path,ignored);}
};
struct Reader final:product::CaptureReader {unsigned stops=0;void stop()override{++stops;}};
std::vector<BYTE> bytes(const fs::path& path){std::ifstream file(path,std::ios::binary);return {std::istreambuf_iterator<char>(file),{}};}
product::Json json(const fs::path& path){auto value=bytes(path);return product::Json::parse(value.begin(),value.end());}
constexpr const char* owner="735027d4-a03e-43d3-830a-21e767dfaf88";

void gated_capture(unsigned seconds){
  using namespace product;
  Temporary directory;std::mutex gate_mutex;std::condition_variable gate_changed;bool entered=false,released=false;
  auto pending=std::make_shared<StoragePending>();std::vector<unsigned> commit_order;
  auto writer=std::make_shared<RawCommitWorker>(pending,[&](const char* stage,const RawCommitJob& job){
    if(std::string_view(stage)!="raw_create")return;
    commit_order.push_back(job.lane_index);
    std::unique_lock lock(gate_mutex);if(released)return;entered=true;gate_changed.notify_all();
    if(!gate_changed.wait_for(lock,std::chrono::seconds(30),[&]{return released;}))throw std::runtime_error("test disk gate timed out");
  });
  Lane microphone(directory.path/L"microphone.caf",1),system(directory.path/L"system.caf",2);
  microphone.raw=std::make_unique<RawSegments>(directory.path,owner,"microphone",1,writer);
  system.raw=std::make_unique<RawSegments>(directory.path,owner,"system",2,writer);
  auto adopt=[](Lane& lane){PreparedInput input;input.reader=std::make_unique<Reader>();input.matching_position_rate=true;lane.adopt(std::move(input));};
  adopt(microphone);adopt(system);
  std::vector<std::int16_t> expected;VoicePCM reference(48000,1);
  std::uint64_t source=0;
  for(unsigned packet=0;packet<seconds*100;++packet){
    const UINT32 count=packet%2?481:479;
    std::vector<float> mono(count),stereo(count*2);
    for(unsigned i=0;i<count;++i){mono[i]=float(std::sin(double(source+i)*0.017)*0.5);stereo[i*2]=stereo[i*2+1]=mono[i];}
    const auto start=100000000ULL+source*10000000/48000,end=100000000ULL+(source+count)*10000000/48000;
    reference.push(mono.data(),mono.size(),start*100,end*100,[&](const auto* pcm,auto n,auto,auto){expected.insert(expected.end(),pcm,pcm+n);});
    for(auto* lane:{&microphone,&system}){
      CapturePacket input;input.frames=count;input.time=start;input.arrival=end;input.position=source;
      input.samples=lane==&microphone?mono:stereo;lane->reader->queue.push(std::move(input));lane->pump();
    }
    source+=count;
  }
  microphone.raw->drain();system.raw->drain();
  {
    std::unique_lock lock(gate_mutex);require(gate_changed.wait_for(lock,std::chrono::seconds(5),[&]{return entered;}),"disk worker never entered gate");
  }
  const auto snapshot=pending->snapshot();
  require(snapshot.frames[0]==source&&snapshot.frames[1]==source,"in-flight disk pop released unpersisted source audio");
  require(snapshot.bytes<mon_capture_limit(11)&&snapshot.jobs==seconds/4*2+2,"compact jobs or active buffers escaped their memory budget");
  require(microphone.reader->queue.peak()<=481&&system.reader->queue.peak()<=481,"slow disk stopped draining native ingress");
  require(microphone.level()["state"]=="measured"&&system.level()["state"]=="measured","slow disk blocked input observations");
  require(microphone.raw->stored_frames()==0&&system.raw->stored_frames()==0,"planned prefix advertised as durable");
  const auto before=GetTickCount64();microphone.reader->stop();system.reader->stop();microphone.close();system.close();
  microphone.raw->finish();system.raw->finish();
  require(GetTickCount64()-before<1000,"disk gate blocked native stop or converter tail drain");
  require(microphone.raw->status()["producerClosed"]==false,"producer closed before queued PCM persisted");
  {std::lock_guard lock(gate_mutex);released=true;gate_changed.notify_all();}
  writer->wait();
  require(commit_order.size()==seconds/4*2,"unexpected number of four-second PCM commits");
  for(std::size_t i=0;i<commit_order.size();++i)require(commit_order[i]==i%2,"one input monopolized the shared disk owner");
  reference.finish([&](const auto* pcm,auto n,auto,auto){expected.insert(expected.end(),pcm,pcm+n);});
  const auto done=pending->snapshot();require(done.frames[0]==0&&done.frames[1]==0&&done.bytes==0&&done.jobs==0,"durable commit failed to release all reservations");
  for(auto* lane:{&microphone,&system}){
    require(lane->raw->status()["producerClosed"]==true,"final durable prefix missing closure");
    require(lane->raw->stored_frames()==expected.size(),"converter output frame count changed");
    std::vector<BYTE> saved;
    for(unsigned sequence=1;sequence<=lane->raw->prefix.segments;++sequence){
      const auto audio=bytes(directory.path/wide(lane->raw->name(sequence,"caf")));
      require(audio.size()>=68&&audio.size()<=128068,"raw PCM batching changed segment geometry");
      saved.insert(saved.end(),audio.begin()+68,audio.end());
    }
    const auto expected_bytes=reinterpret_cast<const BYTE*>(expected.data());
    require(saved.size()==expected.size()*2&&std::equal(saved.begin(),saved.end(),expected_bytes),"batched storage changed accepted PCM");
  }
  require(!fs::exists(directory.path/L"microphone.caf")&&!fs::exists(directory.path/L"system.caf"),"product recording still writes duplicate continuous CAF");
}

void commit_failure(const char* fault){
  using namespace product;
  Temporary directory;auto pending=std::make_shared<StoragePending>();
  auto writer=std::make_shared<RawCommitWorker>(pending,[&](const char* stage,const RawCommitJob& job){
    if(std::string_view(stage)==fault&&job.after.segments==2)throw std::runtime_error(std::string("injected ")+fault);
  });
  RawSegments raw(directory.path,owner,"microphone",1,writer);
  std::vector<float> samples(480,0.25f);
  bool failed=false;
  try{
    for(unsigned packet=0;packet<800;++packet)raw.append(samples.data(),480,100000000ULL+packet*100000,false);
    raw.finish();writer->wait();
  }catch(const std::exception&){failed=true;}
  try{writer->wait();}catch(const std::exception&){failed=true;}
  require(failed,"injected storage failure was hidden");
  const auto tip=json(directory.path/L"raw-microphone.json");
  const auto expected=std::string_view(fault)=="closed_tip"?2:1;
  require(tip["sealedSegments"]==expected&&tip["producerClosed"]==false,"failure advanced or closed an uncommitted raw prefix");
  require(raw.status()["sealedSegments"]==expected&&raw.stored_frames()==expected*64000,"failure snapshot includes queued/planned audio");
  require(fs::exists(directory.path/L"raw-microphone-00000001.caf")&&fs::exists(directory.path/L"raw-microphone-00000001.json"),"failure removed recoverable durable audio");
}

void bounded_pending(){
  using namespace product;
  auto pending=std::make_shared<StoragePending>();
  PacketQueue queue;CapturePacket preparation;preparation.frames=480;queue.push(std::move(preparation));queue.attach_storage(pending,0);
  require(queue.pop()->storage_end==480,"adoption did not charge preparation ingress");
  pending->acquire(0,48000*120-480);bool failed=false;
  try{CapturePacket extra;extra.frames=1;queue.push(std::move(extra));}catch(const std::exception&){failed=true;}
  require(failed&&queue.pending_frames()==0&&pending->snapshot().frames[0]==48000*120,"120-second guard mutated accepted ingress");
  StoragePending bytes_budget;for(unsigned i=0;i<64;++i)bytes_budget.active();failed=false;
  try{bytes_budget.active();}catch(const std::exception&){failed=true;}
  require(failed&&bytes_budget.snapshot().bytes==mon_capture_limit(11),"compact active-buffer byte cap failed");
  StoragePending jobs_budget;jobs_budget.active();
  for(unsigned i=1;i<4096;++i)jobs_budget.replace_active(1,true);
  failed=false;try{jobs_budget.replace_active(1,true);}catch(const std::exception&){failed=true;}
  require(failed&&jobs_budget.snapshot().jobs==4096,"compact job cap mutated queued/in-flight reservations");
}

void planned_prefix_limit(){
  using namespace product;
  Temporary directory;auto pending=std::make_shared<StoragePending>();auto writer=std::make_shared<RawCommitWorker>(pending);
  RawSegments raw(directory.path,owner,"microphone",1,writer);
  raw.prefix.segments=static_cast<std::uint32_t>(mon_capture_limit(1));
  std::vector<float> samples(480,0.25f);bool failed=false;
  try{for(unsigned packet=0;packet<401;++packet)raw.append(samples.data(),480,100000000ULL+packet*100000,false);}catch(const std::exception&){failed=true;}
  writer->wait();
  require(failed&&raw.stored_frames()==0&&json(directory.path/L"raw-microphone.json")["sealedSegments"]==0,"planned capacity failure advanced durable prefix");
  unsigned files=0;for(const auto& file:fs::directory_iterator(directory.path)){(void)file;++files;}
  require(files==1,"prefix preflight wrote audio before rejecting capacity");
}

int main(){try{
  bounded_pending();planned_prefix_limit();for(unsigned seconds:{4,20,100})gated_capture(seconds);
  for(const auto* stage:{"raw_create","raw_write","raw_sync","raw_verify","receipt","tip","closed_tip"})commit_failure(stage);
  std::cout<<"raw commit: 4/20/100-second gated disk, independent ingress, exact PCM, no duplicates, durable failure prefixes, stop tails and bounded reservations passed\n";
  return 0;
}catch(const std::exception& error){std::cerr<<error.what()<<'\n';return 1;}}
