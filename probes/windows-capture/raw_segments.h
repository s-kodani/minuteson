#pragma once
// Included inside namespace product. Four-second, bounded raw PCM commits use
// exactly the portable v2 receipt contract consumed by recording_segments.rs.
std::string sha256_bytes(const BYTE* bytes,std::size_t size){
  BCRYPT_ALG_HANDLE algorithm=nullptr;BCRYPT_HASH_HANDLE hash=nullptr;
  if(BCryptOpenAlgorithmProvider(&algorithm,BCRYPT_SHA256_ALGORITHM,nullptr,0)<0)throw std::runtime_error("SHA256 provider unavailable");
  struct Cleanup{BCRYPT_ALG_HANDLE a;BCRYPT_HASH_HANDLE* h;~Cleanup(){if(*h)BCryptDestroyHash(*h);BCryptCloseAlgorithmProvider(a,0);}} cleanup{algorithm,&hash};
  if(BCryptCreateHash(algorithm,&hash,nullptr,0,nullptr,0,0)<0)throw std::runtime_error("SHA256 creation failed");
  while(size){auto count=ULONG(std::min<std::size_t>(size,1<<20));if(BCryptHashData(hash,const_cast<BYTE*>(bytes),count,0)<0)throw std::runtime_error("SHA256 failed");bytes+=count;size-=count;}
  BYTE digest[32];if(BCryptFinishHash(hash,digest,32,0)<0)throw std::runtime_error("SHA256 finalization failed");
  constexpr char hex[]="0123456789abcdef";std::string result;for(auto byte:digest){result+=hex[byte>>4];result+=hex[byte&15];}return result;
}
struct RawSegments {
  fs::path directory;std::string owner,lane;UINT32 channels;
  std::uint64_t frames=0,total=0;std::uint32_t sequence=0;Json previous=nullptr;bool closed=false;
  PacketClock clock;std::vector<BYTE> pcm;
  RawSegments(fs::path path,std::string id,std::string input,UINT32 count):directory(std::move(path)),owner(std::move(id)),lane(std::move(input)),channels(count){pcm.reserve(192000*channels*4);tip();}
  std::string name(std::uint32_t number,const char* extension)const{std::ostringstream result;result<<"raw-"<<lane<<"-"<<std::setw(8)<<std::setfill('0')<<number<<'.'<<extension;return result.str();}
  Json status()const{return Json{{"schemaVersion",2},{"ownerId",owner},{"lane",lane},{"sealedSegments",sequence},{"sealedFrames",total},{"lastEntrySHA256",previous},{"producerClosed",closed},{"failure",nullptr}};}
  void tip(){auto value=status();value.erase("failure");atomic_json(directory/wide("raw-"+lane+".json"),value);}
  void seal(){
    if(!frames)return;
    auto audio=name(sequence+1,"caf");std::vector<BYTE> body={'c','a','f','f',0,1,0,0,'d','e','s','c'};
    auto be=[&](std::uint64_t value,int width){for(int i=width-1;i>=0;--i)body.push_back(BYTE(value>>(8*i)));};
    be(32,8);double rate=48000;std::uint64_t bits;std::memcpy(&bits,&rate,8);be(bits,8);body.insert(body.end(),{'l','p','c','m'});for(auto value:{3u,channels*4,1u,channels,32u})be(value,4);body.insert(body.end(),{'d','a','t','a'});be(pcm.size()+4,8);be(0,4);body.insert(body.end(),pcm.begin(),pcm.end());
    {Handle file(CreateFileW((directory/wide(audio)).c_str(),GENERIC_WRITE,FILE_SHARE_READ,nullptr,CREATE_NEW,FILE_FLAG_WRITE_THROUGH,nullptr));if(file.value==INVALID_HANDLE_VALUE)throw std::runtime_error("原PCMの確定先を作成できません");write_all(file.value,body.data(),body.size());if(!capture_flush(file.value,"raw_pcm_sync"))throw std::runtime_error("原PCMを同期できません");}
    const auto origin=clock.parts.front().start;Json maps=Json::array(),endpoints=Json::array();
    for(auto& part:clock.parts){maps.push_back(Json{{"sourceFrameStart",part.first},{"sourceFrameEnd",part.first+part.frames},{"timelineStartUs",(part.start-origin)/10},{"timelineEndUs",(part.last_end-origin)/10},{"mappingJson","{\"schema_version\":1,\"method\":\"linear\"}"}});endpoints.push_back(Json{{"startNanoseconds",part.start*100},{"endNanoseconds",part.last_end*100}});}
    Json entry={{"schemaVersion",2},{"ownerId",owner},{"lane",lane},{"sequence",sequence+1},{"audioName",audio},{"sourceFrameStart",total},{"frames",frames},{"sampleRate",48000},{"channels",channels},{"captureOriginNanoseconds",origin*100},{"clockSegments",maps},{"clockNanoseconds",endpoints},{"audioBytes",body.size()},{"audioSHA256",sha256_bytes(body.data(),body.size())},{"previousEntrySHA256",previous}};
    const auto encoded=entry.dump();atomic_json(directory/wide(name(sequence+1,"json")),entry);
    previous=sha256_bytes(reinterpret_cast<const BYTE*>(encoded.data()),encoded.size());++sequence;total+=frames;frames=0;pcm.clear();clock=PacketClock();tip();
  }
  void append(const float* samples,UINT32 count,std::uint64_t time,bool boundary){
    while(count){if(clock.parts.size()>=126)seal();auto n=UINT32(std::min<std::uint64_t>(count,192000-frames));
      clock.append(frames,n,time,boundary);const auto bytes=reinterpret_cast<const BYTE*>(samples);pcm.insert(pcm.end(),bytes,bytes+std::size_t(n)*channels*4);frames+=n;count-=n;samples+=std::size_t(n)*channels;time+=std::uint64_t(n)*10000000/48000;boundary=false;if(frames==192000)seal();
    }
  }
  void finish(){seal();closed=true;tip();}
};
