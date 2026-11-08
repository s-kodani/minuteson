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
struct RawSegments {
  fs::path directory;std::string owner,lane;UINT32 channels;
  std::uint64_t frames=0;MonRawPrefix prefix=mon_capture_prefix_empty();bool closed=false;
  PacketClock clock{PacketClock::bounded_capacity,16000};std::vector<BYTE> pcm;
  std::unique_ptr<VoicePCM> converter;Handle continuous;std::uint64_t saved_frames=0;bool output_boundary=false;
  std::vector<ClockPart> committed_clock;
  RawSegments(fs::path path,std::string id,std::string input,UINT32 count):directory(std::move(path)),owner(std::move(id)),lane(std::move(input)),channels(count){pcm.reserve(64000*2);tip();}
  std::string name(std::uint32_t number,const char* extension)const{std::ostringstream result;result<<"raw-"<<lane<<"-"<<std::setw(8)<<std::setfill('0')<<number<<'.'<<extension;return result.str();}
  Json prefix_tip(bool final)const{
    std::uint8_t bytes[1024];const auto count=mon_capture_prefix_tip(&prefix,
      reinterpret_cast<const std::uint8_t*>(owner.data()),owner.size(),
      reinterpret_cast<const std::uint8_t*>(lane.data()),lane.size(),final,bytes,sizeof(bytes));
    if(count<0)throw std::runtime_error("raw segment prefix identity is invalid");
    return Json::parse(bytes,bytes+count);
  }
  Json previous_hash()const{std::uint8_t bytes[64];if(!mon_capture_prefix_hash(&prefix,bytes,64))return nullptr;return std::string(reinterpret_cast<char*>(bytes),64);}
  Json status()const{auto value=prefix_tip(closed);value["failure"]=nullptr;return value;}
  void tip(){atomic_json(directory/wide("raw-"+lane+".json"),prefix_tip(closed));}
  void seal(){
    if(!frames)return;
    if(prefix.segments>=mon_capture_limit(1))throw std::runtime_error("raw segment sequence capacity exceeded");
    if(committed_clock.size()+clock.parts.size()>PacketClock::recording_capacity)throw std::runtime_error("too many committed recording clock segments");
    auto audio=name(prefix.segments+1,"caf");std::vector<BYTE> body(68);
    if(mon_voice_caf_header(frames,body.data(),body.size())!=68)throw std::runtime_error("invalid raw CAF geometry");
    body.insert(body.end(),pcm.begin(),pcm.end());
    const auto origin=clock.parts.front().start;Json maps=Json::array(),endpoints=Json::array();
    for(auto& part:clock.parts){maps.push_back(Json{{"sourceFrameStart",part.first},{"sourceFrameEnd",part.first+part.frames},{"timelineStartUs",(part.start-origin)/10},{"timelineEndUs",(part.last_end-origin)/10},{"mappingJson","{\"schema_version\":1,\"method\":\"linear\"}"}});endpoints.push_back(Json{{"startNanoseconds",part.start*100},{"endNanoseconds",part.last_end*100}});}
    Json entry={{"schemaVersion",3},{"sampleEncoding","pcm16"},{"ownerId",owner},{"lane",lane},{"sequence",prefix.segments+1},{"audioName",audio},{"sourceFrameStart",prefix.frames},{"frames",frames},{"sampleRate",16000},{"channels",1},{"captureOriginNanoseconds",origin*100},{"clockSegments",maps},{"clockNanoseconds",endpoints},{"audioBytes",body.size()},{"audioSHA256",sha256_bytes(body.data(),body.size())},{"previousEntrySHA256",previous_hash()}};
    const auto encoded=entry.dump();
    // Check the common reader contract BEFORE writing either member. Never
    // advertise a prefix which finalization or interrupted recovery rejects.
    MonRawPrefix next{};std::uint8_t next_tip[1024];
    const auto tip_bytes=mon_capture_prepare_commit(&prefix,reinterpret_cast<const std::uint8_t*>(encoded.data()),encoded.size(),&next,next_tip,sizeof(next_tip));
    if(tip_bytes<0)throw std::runtime_error("raw segment commit does not extend the accepted prefix");
    // Keep this no-write/no-delete-sharing handle until the receipt and tip
    // commit, and verify actual stored bytes rather than hashing memory only.
    Handle file(CreateFileW((directory/wide(audio)).c_str(),GENERIC_READ|GENERIC_WRITE,FILE_SHARE_READ,nullptr,CREATE_NEW,FILE_FLAG_WRITE_THROUGH,nullptr));
    if(file.value==INVALID_HANDLE_VALUE)throw std::runtime_error("原PCMの確定先を作成できません");
    write_all(file.value,body.data(),body.size());if(!capture_flush(file.value,"raw_pcm_sync"))throw std::runtime_error("原PCMを同期できません");
    verify_raw_pcm(file.value,body);
    atomic_json(directory/wide(name(prefix.segments+1,"json")),entry);
    // Export exactly the maps durably bound to these samples. A separate
    // whole-recording affine fit can disagree with four-second raw fits.
    atomic_json(directory/wide("raw-"+lane+".json"),Json::parse(next_tip,next_tip+tip_bytes));
    for(auto part:clock.parts){part.first+=prefix.frames;committed_clock.push_back(part);}
    prefix=next;frames=0;pcm.clear();clock=PacketClock(PacketClock::bounded_capacity,16000);
  }
  void converted(const std::int16_t* samples,std::size_t count,std::int64_t start_ns,std::int64_t end_ns){
    if(continuous.value==INVALID_HANDLE_VALUE){
      const auto leaf=lane=="microphone"?L"microphone.caf":L"system.caf";
      continuous.value=CreateFileW((directory/leaf).c_str(),GENERIC_WRITE,FILE_SHARE_READ,nullptr,CREATE_NEW,FILE_ATTRIBUTE_NORMAL,nullptr);
      if(continuous.value==INVALID_HANDLE_VALUE)throw std::runtime_error("compact capture destination already exists");
      auto header=VoicePCM::header(0);std::fill(header.begin()+56,header.begin()+64,255);write_all(continuous.value,header.data(),header.size());
    }
    const auto total=count;std::size_t offset=0;
    while(count){
      if(clock.parts.size()>=126||frames==64000)seal();
      const auto n=std::min<std::size_t>(count,64000-frames);
      const auto start=std::uint64_t(start_ns+(end_ns-start_ns)*offset/total)/100;
      const auto end=std::uint64_t(start_ns+(end_ns-start_ns)*(offset+n)/total)/100;
      clock.append(frames,static_cast<std::uint32_t>(n),start,output_boundary,end);output_boundary=false;
      const auto bytes=reinterpret_cast<const BYTE*>(samples);
      pcm.insert(pcm.end(),bytes,bytes+n*2);write_all(continuous.value,bytes,n*2);
      frames+=n;saved_frames+=n;offset+=n;count-=n;samples+=n;
      if(frames==64000)seal();
    }
  }
  void drain(){
    if(converter){converter->finish([&](const auto* pcm,auto count,auto start,auto end){converted(pcm,count,start,end);});converter.reset();}
  }
  void append(const float* samples,UINT32 count,std::uint64_t time,bool boundary){
    if(boundary){drain();output_boundary=true;}
    if(!converter)converter=std::make_unique<VoicePCM>(48000,channels);
    converter->push(samples,std::size_t(count)*channels,static_cast<std::int64_t>(time*100),
      static_cast<std::int64_t>(time*100+std::uint64_t(count)*1000000000/48000),
      [&](const auto* pcm,auto count,auto start,auto end){converted(pcm,count,start,end);});
  }
  void finish(){
    drain();seal();
    if(continuous.value!=INVALID_HANDLE_VALUE){
      LARGE_INTEGER zero{};if(!SetFilePointerEx(continuous.value,zero,nullptr,FILE_BEGIN))throw std::runtime_error("compact CAF finalization seek failed");
      auto header=VoicePCM::header(saved_frames);write_all(continuous.value,header.data(),header.size());
      if(!capture_flush(continuous.value,"caf_finalize_sync"))throw std::runtime_error("compact CAF finalization sync failed");
      CloseHandle(continuous.value);continuous.value=INVALID_HANDLE_VALUE;
    }
    atomic_json(directory/wide("raw-"+lane+".json"),prefix_tip(true));closed=true;
  }
  Json timeline(std::uint64_t origin)const{
    Json result=Json::array();
    for(const auto& part:committed_clock)result.push_back(Json{{"sourceFrameStart",part.first},{"sourceFrameEnd",part.first+part.frames},{"timelineStartUs",(part.start-origin)/10},{"timelineEndUs",(part.last_end-origin)/10},{"mappingJson","{\"schema_version\":1,\"method\":\"linear\"}"}});
    return result;
  }
};
