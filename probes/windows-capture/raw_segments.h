#pragma once
// Included inside namespace product. Four-second, bounded raw PCM commits use
// exactly the portable v3 PCM16 receipt contract consumed by recording_segments.rs.
std::string sha256_bytes(const BYTE* bytes,std::size_t size){
  BCRYPT_ALG_HANDLE algorithm=nullptr;BCRYPT_HASH_HANDLE hash=nullptr;
  if(BCryptOpenAlgorithmProvider(&algorithm,BCRYPT_SHA256_ALGORITHM,nullptr,0)<0)throw std::runtime_error("SHA256 provider unavailable");
  struct Cleanup{BCRYPT_ALG_HANDLE a;BCRYPT_HASH_HANDLE* h;~Cleanup(){if(*h)BCryptDestroyHash(*h);BCryptCloseAlgorithmProvider(a,0);}} cleanup{algorithm,&hash};
  if(BCryptCreateHash(algorithm,&hash,nullptr,0,nullptr,0,0)<0)throw std::runtime_error("SHA256 creation failed");
  while(size){auto count=ULONG(std::min<std::size_t>(size,1<<20));if(BCryptHashData(hash,const_cast<BYTE*>(bytes),count,0)<0)throw std::runtime_error("SHA256 failed");bytes+=count;size-=count;}
  BYTE digest[32];if(BCryptFinishHash(hash,digest,32,0)<0)throw std::runtime_error("SHA256 finalization failed");
  constexpr char hex[]="0123456789abcdef";std::string result;for(auto byte:digest){result+=hex[byte>>4];result+=hex[byte&15];}return result;
}
void verify_raw_pcm(HANDLE file,const std::vector<BYTE>& expected){
  BY_HANDLE_FILE_INFORMATION before{},after{};LARGE_INTEGER size{},zero{};
  if(!GetFileInformationByHandle(file,&before)||before.nNumberOfLinks!=1
    ||(before.dwFileAttributes&(FILE_ATTRIBUTE_DIRECTORY|FILE_ATTRIBUTE_REPARSE_POINT))
    ||!GetFileSizeEx(file,&size)||size.QuadPart<0||static_cast<std::uint64_t>(size.QuadPart)!=expected.size()
    ||!SetFilePointerEx(file,zero,nullptr,FILE_BEGIN))throw std::runtime_error("raw segment readback identity is unavailable");
  std::vector<BYTE> block(65536);std::size_t offset=0;
  while(offset<expected.size()){
    const auto requested=static_cast<DWORD>(std::min(block.size(),expected.size()-offset));DWORD count=0;
    if(!ReadFile(file,block.data(),requested,&count,nullptr)||!count||count>requested
      ||!std::equal(block.begin(),block.begin()+count,expected.begin()+offset))throw std::runtime_error("raw segment stored PCM differs from accepted frames");
    offset+=count;
  }
  if(!GetFileInformationByHandle(file,&after)||before.dwVolumeSerialNumber!=after.dwVolumeSerialNumber
    ||before.nFileIndexHigh!=after.nFileIndexHigh||before.nFileIndexLow!=after.nFileIndexLow
    ||before.nFileSizeHigh!=after.nFileSizeHigh||before.nFileSizeLow!=after.nFileSizeLow
    ||ticks(before.ftLastWriteTime)!=ticks(after.ftLastWriteTime))throw std::runtime_error("raw segment changed during readback");
}
#include "raw_commit_worker.h"
struct RawSegments {
  fs::path directory;std::string owner,lane;UINT32 channels;unsigned lane_index;
  std::uint64_t frames=0,saved_frames=0;
  MonRawPrefix prefix=mon_capture_prefix_empty(); // Planned, never advertised.
  PacketClock clock{PacketClock::bounded_capacity,16000};std::vector<BYTE> pcm;
  std::unique_ptr<VoicePCM> converter;bool output_boundary=false,active=true,closing=false;
  std::uint64_t converter_source_base=0,converter_output_frames=0,submitted_source_end=0,source_end=0;
  std::shared_ptr<RawCommitWorker> writer;
  std::shared_ptr<RawCommitState> committed=std::make_shared<RawCommitState>();
  RawSegments(fs::path path,std::string id,std::string input,UINT32 count,std::shared_ptr<RawCommitWorker> disk)
    :directory(std::move(path)),owner(std::move(id)),lane(std::move(input)),channels(count),
     lane_index(lane=="microphone"?0:1),writer(std::move(disk)){
    writer->pending()->active();pcm.reserve(64000*2+68);pcm.resize(68);
    // The session constructs both initial tips before native acquisition starts.
    atomic_json(directory/wide("raw-"+lane+".json"),prefix_tip(prefix,false));
  }
  ~RawSegments(){if(active)writer->pending()->abandon_active();}
  std::string name(std::uint32_t number,const char* extension)const{std::ostringstream result;result<<"raw-"<<lane<<"-"<<std::setw(8)<<std::setfill('0')<<number<<'.'<<extension;return result.str();}
  Json prefix_tip(const MonRawPrefix& value,bool final)const{
    std::uint8_t bytes[1024];const auto count=mon_capture_prefix_tip(&value,
      reinterpret_cast<const std::uint8_t*>(owner.data()),owner.size(),
      reinterpret_cast<const std::uint8_t*>(lane.data()),lane.size(),final,bytes,sizeof(bytes));
    if(count<0)throw std::runtime_error("raw segment prefix identity is invalid");
    return Json::parse(bytes,bytes+count);
  }
  Json previous_hash()const{std::uint8_t bytes[64];if(!mon_capture_prefix_hash(&prefix,bytes,64))return nullptr;return std::string(reinterpret_cast<char*>(bytes),64);}
  Json status()const{std::lock_guard lock(committed->mutex);auto value=prefix_tip(committed->prefix,committed->closed);value["failure"]=committed->failure.empty()?Json(nullptr):Json(committed->failure);return value;}
  std::uint64_t stored_frames()const{std::lock_guard lock(committed->mutex);return committed->prefix.frames;}
  void seal(){
    if(!frames)return;
    if(prefix.segments>=mon_capture_limit(1))throw std::runtime_error("raw segment sequence capacity exceeded");
    if(prefix.clock_maps+clock.parts.size()>PacketClock::recording_capacity)throw std::runtime_error("too many committed recording clock segments");
    if(mon_voice_caf_header(frames,pcm.data(),68)!=68)throw std::runtime_error("invalid raw CAF geometry");
    const auto audio=name(prefix.segments+1,"caf");
    const auto origin=clock.parts.front().start;Json maps=Json::array(),endpoints=Json::array();
    for(auto& part:clock.parts){maps.push_back(Json{{"sourceFrameStart",part.first},{"sourceFrameEnd",part.first+part.frames},{"timelineStartUs",(part.start-origin)/10},{"timelineEndUs",(part.last_end-origin)/10},{"mappingJson","{\"schema_version\":1,\"method\":\"linear\"}"}});endpoints.push_back(Json{{"startNanoseconds",part.start*100},{"endNanoseconds",part.last_end*100}});}
    Json entry={{"schemaVersion",3},{"sampleEncoding","pcm16"},{"ownerId",owner},{"lane",lane},{"sequence",prefix.segments+1},{"audioName",audio},{"sourceFrameStart",prefix.frames},{"frames",frames},{"sampleRate",16000},{"channels",1},{"captureOriginNanoseconds",origin*100},{"clockSegments",maps},{"clockNanoseconds",endpoints},{"audioBytes",pcm.size()},{"audioSHA256",sha256_bytes(pcm.data(),pcm.size())},{"previousEntrySHA256",previous_hash()}};
    const auto encoded=entry.dump();
    MonRawPrefix next{};std::uint8_t next_tip[1024];
    const auto tip_bytes=mon_capture_prepare_commit(&prefix,reinterpret_cast<const std::uint8_t*>(encoded.data()),encoded.size(),&next,next_tip,sizeof(next_tip));
    if(tip_bytes<0)throw std::runtime_error("raw segment commit does not extend the accepted prefix");
    RawCommitJob job;job.directory=directory;job.lane=lane;job.lane_index=lane_index;job.audio_name=audio;
    job.receipt_name=name(prefix.segments+1,"json");job.receipt=encoded;
    job.tip.assign(reinterpret_cast<char*>(next_tip),tip_bytes);job.before=prefix;job.after=next;
    job.source_end=source_end;job.state=committed;job.clocks=clock.parts;
    // Keep the assembler intact if admission fails. Body ownership moves only
    // after all preflight checks, with no full recording copy or file access.
    job.body=pcm;
    writer->enqueue(std::move(job),!closing);
    prefix=next;frames=0;pcm.clear();clock=PacketClock(PacketClock::bounded_capacity,16000);
    if(closing)active=false;else pcm.resize(68);
  }
  void converted(const std::int16_t* samples,std::size_t count,std::int64_t start_ns,std::int64_t end_ns){
    const auto total=count;std::size_t offset=0;
    while(count){
      if(clock.parts.size()>=126||frames==64000)seal();
      const auto n=std::min<std::size_t>(count,64000-frames);
      const auto start=std::uint64_t(start_ns+(end_ns-start_ns)*offset/total)/100;
      const auto end=std::uint64_t(start_ns+(end_ns-start_ns)*(offset+n)/total)/100;
      clock.append(frames,static_cast<std::uint32_t>(n),start,output_boundary,end);output_boundary=false;
      const auto bytes=reinterpret_cast<const BYTE*>(samples);pcm.insert(pcm.end(),bytes,bytes+n*2);
      frames+=n;saved_frames+=n;offset+=n;count-=n;samples+=n;
      converter_output_frames+=n;
      source_end=std::max(source_end,std::min(submitted_source_end,converter_source_base+converter_output_frames*3));
      if(frames==64000)seal();
    }
  }
  void drain(){
    if(converter){converter->finish([&](const auto* pcm,auto count,auto start,auto end){converted(pcm,count,start,end);});converter.reset();}
  }
  void append(const float* samples,UINT32 count,std::uint64_t time,bool boundary,std::uint64_t reservation_end=0){
    writer->check();
    if(!active||closing)throw std::logic_error("recording assembler already closed");
    if(boundary){drain();output_boundary=true;}
    if(!reservation_end)reservation_end=writer->pending()->acquire(lane_index,count);
    if(!converter){converter=std::make_unique<VoicePCM>(48000,channels);converter_source_base=reservation_end-count;converter_output_frames=0;}
    submitted_source_end=reservation_end;
    converter->push(samples,std::size_t(count)*channels,static_cast<std::int64_t>(time*100),
      static_cast<std::int64_t>(time*100+std::uint64_t(count)*1000000000/48000),
      [&](const auto* pcm,auto count,auto start,auto end){converted(pcm,count,start,end);});
  }
  void finish(){
    // Drain with a live assembler: converter.finish can cross a four-second
    // boundary and still need a second job for its final filtered samples.
    drain();closing=true;seal();
    if(!active){writer->pending()->active();active=true;}
    RawCommitJob job;job.directory=directory;job.lane=lane;job.lane_index=lane_index;
    job.before=job.after=prefix;job.source_end=source_end;job.state=committed;
    job.final=true;job.tip=prefix_tip(prefix,true).dump();
    writer->enqueue(std::move(job),false);active=false;
  }
  Json timeline(std::uint64_t origin)const{
    Json result=Json::array();std::lock_guard lock(committed->mutex);
    for(const auto& part:committed->clocks)result.push_back(Json{{"sourceFrameStart",part.first},{"sourceFrameEnd",part.first+part.frames},{"timelineStartUs",(part.start-origin)/10},{"timelineEndUs",(part.last_end-origin)/10},{"mappingJson","{\"schema_version\":1,\"method\":\"linear\"}"}});
    return result;
  }
};
