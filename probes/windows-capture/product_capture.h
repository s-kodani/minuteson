#pragma once
#include "json.hpp"
#include "packet_clock.h"
#include "packet_queue.h"
#include "input_preparation.h"
#include "loopback_scope.h"
#include <avrt.h>
#include <functiondiscoverykeys_devpkey.h>
#include <tlhelp32.h>
#include <audiopolicy.h>
#include <bcrypt.h>
#include <algorithm>
#include <cmath>
#include <map>
#include <set>
#include <memory>
#include <condition_variable>
#include <mutex>
#include <optional>
#include <thread>

namespace product {
using Json=nlohmann::json;
constexpr std::uint64_t EPOCH=116444736000000000ULL;
struct Handle {
  HANDLE value=INVALID_HANDLE_VALUE;
  explicit Handle(HANDLE h=INVALID_HANDLE_VALUE):value(h){}
  Handle(const Handle&)=delete; Handle& operator=(const Handle&)=delete;
  ~Handle(){if(value && value!=INVALID_HANDLE_VALUE)CloseHandle(value);}
};
std::uint64_t ticks(FILETIME v){return (std::uint64_t(v.dwHighDateTime)<<32)|v.dwLowDateTime;}
std::uint64_t qpc(){LARGE_INTEGER t,f;QueryPerformanceCounter(&t);QueryPerformanceFrequency(&f);return std::uint64_t(t.QuadPart/f.QuadPart)*10000000ULL+std::uint64_t(t.QuadPart%f.QuadPart)*10000000ULL/std::uint64_t(f.QuadPart);}
std::wstring wide(const std::string& s){int n=MultiByteToWideChar(CP_UTF8,MB_ERR_INVALID_CHARS,s.data(),int(s.size()),nullptr,0);if(!n)throw std::runtime_error("invalid UTF-8");std::wstring w(n,0);MultiByteToWideChar(CP_UTF8,MB_ERR_INVALID_CHARS,s.data(),int(s.size()),w.data(),n);return w;}
void write_all(HANDLE file,const void* data,std::size_t bytes){auto started=GetTickCount64();auto p=static_cast<const BYTE*>(data);while(bytes){DWORD n=0;DWORD part=DWORD(std::min<std::size_t>(bytes,1<<20));if(!WriteFile(file,p,part,&n,nullptr)||n!=part){auto code=GetLastError();if(code==ERROR_DISK_FULL||code==ERROR_HANDLE_DISK_FULL)throw std::runtime_error("空き容量が不足し、音声を保存できません。確定済み原音声は保持しています");throw std::runtime_error("音声・履歴の保存に失敗しました。保存先の書込み権限と共有ロックを確認してください（Windows error "+std::to_string(code)+"）");}p+=n;bytes-=n;}const auto elapsed=GetTickCount64()-started;if(elapsed>=250)std::cerr<<"mon_capture_io: stage=write elapsedMs="<<elapsed<<std::endl;}
bool capture_flush(HANDLE file,const char* stage){auto started=GetTickCount64();const bool ok=FlushFileBuffers(file)!=0;const auto elapsed=GetTickCount64()-started;if(elapsed>=250)std::cerr<<"mon_capture_io: stage="<<stage<<" elapsedMs="<<elapsed<<std::endl;return ok;}
void atomic_json(const fs::path& path,const Json& value,bool durable=true){
  auto started=GetTickCount64();
  auto temporary=path;temporary+=L".pending";
  struct OwnedTemporary {fs::path path;bool owned=false;~OwnedTemporary(){if(owned)DeleteFileW(path.c_str());}} cleanup{temporary};
  {Handle file(CreateFileW(temporary.c_str(),GENERIC_WRITE,FILE_SHARE_READ,nullptr,CREATE_NEW,FILE_ATTRIBUTE_NORMAL|(durable?FILE_FLAG_WRITE_THROUGH:0),nullptr));if(file.value==INVALID_HANDLE_VALUE)throw std::runtime_error("metadata temporary file already exists or cannot be created");cleanup.owned=true;auto data=value.dump();write_all(file.value,data.data(),data.size());if(durable&&!capture_flush(file.value,"metadata_sync"))throw std::runtime_error("metadata flush failed");}
  // A scanner or a short-lived reader can temporarily deny delete sharing.
  // Keep one flushed staging file and retry only that same atomic publication.
  const auto replacing=GetTickCount64();auto deadline=replacing+2000;
  while(!MoveFileExW(temporary.c_str(),path.c_str(),MOVEFILE_REPLACE_EXISTING|(durable?MOVEFILE_WRITE_THROUGH:0))) {
    DWORD error=GetLastError();
    if((error!=ERROR_SHARING_VIOLATION&&error!=ERROR_LOCK_VIOLATION&&error!=ERROR_ACCESS_DENIED)||GetTickCount64()>=deadline)
      throw std::runtime_error("metadata publication failed ("+utf8(path.filename().wstring())+", Windows error "+std::to_string(error)+")");
    Sleep(10);
  }
  const auto replacement_ms=GetTickCount64()-replacing;if(replacement_ms>=250)std::cerr<<"mon_capture_io: stage=replace_wait elapsedMs="<<replacement_ms<<std::endl;
  const auto elapsed=GetTickCount64()-started;if(elapsed>=250)std::cerr<<"mon_capture_io: file="<<utf8(path.filename().wstring())<<" durable="<<durable<<" elapsedMs="<<elapsed<<std::endl;
}
#include "metadata_worker.h"
Json process_identity(DWORD pid){
  Handle process(OpenProcess(SYNCHRONIZE|PROCESS_QUERY_LIMITED_INFORMATION,FALSE,pid));if(!process.value||WaitForSingleObject(process.value,0)!=WAIT_TIMEOUT)throw std::runtime_error("application is no longer available");
  FILETIME created,ended,kernel,user;if(!GetProcessTimes(process.value,&created,&ended,&kernel,&user))throw std::runtime_error("application creation time unavailable");
  std::wstring path(32768,0);DWORD n=DWORD(path.size());if(!QueryFullProcessImageNameW(process.value,0,path.data(),&n))throw std::runtime_error("application identity unavailable");path.resize(n);
  if(WaitForSingleObject(process.value,0)!=WAIT_TIMEOUT)throw std::runtime_error("application is no longer available");
  return Json{{"pid",pid},{"bundleIdentifier",utf8(path)},{"launchTimeUs",(ticks(created)-EPOCH)/10}};
}
Json applications(){
  std::set<DWORD> pids;EnumWindows([](HWND window,LPARAM context)->BOOL{if(IsWindowVisible(window)&&GetWindowTextLengthW(window)>0){DWORD pid;GetWindowThreadProcessId(window,&pid);reinterpret_cast<std::set<DWORD>*>(context)->insert(pid);}return TRUE;},reinterpret_cast<LPARAM>(&pids));
  Json list=Json::array();for(auto pid:pids){if(list.size()>=128)break;try{auto app=process_identity(pid);app["name"]=utf8(fs::path(wide(app["bundleIdentifier"].get<std::string>())).stem().wstring());list.push_back(app);}catch(const std::exception&){}}
  return Json{{"schemaVersion",1},{"systemPermission","authorized"},{"applications",list}};
}
std::wstring endpoint_id(IMMDevice* device){LPWSTR id=nullptr;check(device->GetId(&id),"get endpoint identity");std::wstring value(id);CoTaskMemFree(id);return value;}
ComPtr<IMMDeviceEnumerator> enumerator(){ComPtr<IMMDeviceEnumerator> e;check(CoCreateInstance(__uuidof(MMDeviceEnumerator),nullptr,CLSCTX_ALL,IID_PPV_ARGS(e.GetAddressOf())),"enumerate audio endpoints");return e;}
Json microphones(){
  auto e=enumerator();ComPtr<IMMDevice> selected;std::wstring current;if(SUCCEEDED(e->GetDefaultAudioEndpoint(eCapture,eCommunications,&selected)))current=endpoint_id(selected.Get());
  ComPtr<IMMDeviceCollection> collection;check(e->EnumAudioEndpoints(eCapture,DEVICE_STATE_ACTIVE,&collection),"list microphones");UINT count=0;check(collection->GetCount(&count),"count microphones");Json devices=Json::array();
  for(UINT i=0;i<count&&i<128;++i){ComPtr<IMMDevice> device;check(collection->Item(i,&device),"read microphone");auto id=endpoint_id(device.Get());std::string name=utf8(id);ComPtr<IPropertyStore> props;if(SUCCEEDED(device->OpenPropertyStore(STGM_READ,&props))){PROPVARIANT value;PropVariantInit(&value);if(SUCCEEDED(props->GetValue(PKEY_Device_FriendlyName,&value))&&value.vt==VT_LPWSTR)name=utf8(value.pwszVal);PropVariantClear(&value);}devices.push_back(Json{{"uid",utf8(id)},{"name",name},{"isDefault",id==current}});}
  return Json{{"schemaVersion",1},{"microphonePermission","unknown"},{"devices",devices}};
}
#include "input_binding.h"
#include "raw_segments.h"
#include "async_capture.h"
struct CaptureApartment {
  CaptureApartment(){acquisition(CoInitializeEx(nullptr,COINIT_MULTITHREADED),"initialize WASAPI preparation thread");}
  ~CaptureApartment(){CoUninitialize();}
};
struct PreparedInput {
  ComPtr<IAudioClient> client;
  std::unique_ptr<AsyncCapture> reader;
  MicrophoneChoice choice;
  bool matching_position_rate=false;
  PreparedInput()=default;
  PreparedInput(PreparedInput&& other)noexcept{swap(other);}
  PreparedInput& operator=(PreparedInput&& other)noexcept{swap(other);return *this;}
  void swap(PreparedInput& other)noexcept{client.Swap(other.client);reader.swap(other.reader);std::swap(choice,other.choice);std::swap(matching_position_rate,other.matching_position_rate);}
  ~PreparedInput(){if(reader)reader->stop();}
};
using NativePreparation=InputPreparation<PreparedInput,CaptureApartment>;
MicrophoneChoice current_microphone_choice(const std::string& uid,DWORD selected_pid,const Json& identity){
  bool live=true;try{if(selected_pid)live=process_identity(selected_pid)==identity;}catch(...){live=false;}
  auto choice=choose_microphone(uid,live?selected_pid:0);
  if(selected_pid&&uid.empty()&&live){try{live=process_identity(selected_pid)==identity;}catch(...){live=false;}if(!live)choice=choose_microphone("",0);}
  if(selected_pid&&uid.empty()&&!live)choice.fallback="application_input_unavailable";
  return choice;
}
Json microphone_selection_status(const std::string& owner,std::uint64_t revision,std::uint64_t request_revision,const std::string& requested,const std::string& selected,const MicrophoneChoice& choice,bool pending){
  const auto& uid=pending&&!requested.empty()?requested:selected;
  if(uid.empty())return nullptr;
  return Json{{"schemaVersion",1},{"ownerId",owner},{"revision",revision},{"requestRevision",request_revision},{"mode",requested.empty()?"automatic":"fixed"},{"deviceUID",uid},{"source",pending?"pending":choice.source},{"fallbackReason",pending||choice.fallback.empty()?Json(nullptr):Json(choice.fallback)}};
}
PreparedInput prepare_input(bool microphone,const std::string& uid,DWORD selected_pid,const Json& identity,NativePreparation::Cancellation cancelled){
  PreparedInput result;
  auto current=[&]{
    if(cancelled())throw InputFailure(E_ABORT,"obsolete WASAPI preparation");
    if(!microphone&&selected_pid&&process_identity(selected_pid)!=identity)throw InputFailure(E_FAIL,"selected application identity changed");
  };
  current();
  if(microphone){
    result.choice=current_microphone_choice(uid,selected_pid,identity);
    current();auto e=enumerator();ComPtr<IMMDevice> device;
    acquisition(e->GetDevice(wide(result.choice.uid).c_str(),&device),"get selected microphone");
    acquisition(device->Activate(__uuidof(IAudioClient),CLSCTX_ALL,nullptr,reinterpret_cast<void**>(result.client.GetAddressOf())),"activate WASAPI microphone");
  }else{
    if(selected_pid&&process_identity(selected_pid)!=identity)throw InputFailure(E_FAIL,"selected application identity changed");
    const auto target=loopback_target(selected_pid,GetCurrentProcessId());
    // Process loopback includes every render endpoint. The global mode excludes
    // this helper's tree only; selected-app capture never falls back to global.
    // The worker retains an unfinished native activation even after UI timeout.
    result.client=activate_process_loopback(target.pid,target.exclude_tree,INFINITE);
  }
  current();
  WAVEFORMATEX* mix=nullptr;
  result.matching_position_rate=SUCCEEDED(result.client->GetMixFormat(&mix))&&mix&&mix->nSamplesPerSec==48000;
  if(mix)CoTaskMemFree(mix);
  current();
  const UINT32 channels=microphone?1:2;
  WAVEFORMATEX format={};format.wFormatTag=WAVE_FORMAT_IEEE_FLOAT;format.nChannels=WORD(channels);format.nSamplesPerSec=48000;format.wBitsPerSample=32;format.nBlockAlign=WORD(channels*4);format.nAvgBytesPerSec=48000*format.nBlockAlign;
  acquisition(result.client->Initialize(AUDCLNT_SHAREMODE_SHARED,AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM|AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY|(microphone?0:AUDCLNT_STREAMFLAGS_LOOPBACK),5000000,0,&format,nullptr),"initialize 48 kHz floating-point WASAPI");
  current();result.reader=std::make_unique<AsyncCapture>(std::move(result.client),channels);
  current();
  return result;
}
class Lane {
 public:
  fs::path path;UINT32 channels;std::uint64_t frames=0,packets=0,first=0;double rms=0,peak=0;UINT32 window_frames=0;std::uint64_t measured=0;PacketClock clock;
  Handle file;
  bool started=false,holding_for_window=false;std::uint64_t window_start=0,window_end=0;
  Binding binding;
  std::unique_ptr<RawSegments> raw;
  std::unique_ptr<AsyncCapture> reader;
  std::unique_ptr<NativePreparation> preparation;
  PreparationSchedule preparation_schedule;
  std::uint64_t peak_queued_frames=0,discontinuities=0;
  Json recent=Json::array(),anomalies=Json::array();std::uint64_t observed=0;
  Lane(fs::path output,UINT32 count,bool bounded=false):path(std::move(output)),channels(count),clock(bounded?PacketClock::bounded_capacity:PacketClock::recording_capacity){}
  ~Lane(){if(reader)reader->stop();}
  void cancel_preparation(){if(preparation){preparation->cancel();preparation_schedule.cancel();}}
  void adopt(PreparedInput input){clock.begin_stream(input.matching_position_rate);reader=std::move(input.reader);started=true;binding.connected();preparation_schedule.accept();}
  void detach(const char* reason,bool unavailable=false){cancel_preparation();if(started)std::cerr<<"mon_capture_reconnect: lane="<<(channels==1?"microphone":"system")<<" reason="<<reason<<std::endl;if(reader){reader->stop();peak_queued_frames=std::max(peak_queued_frames,reader->queue.peak());try{pump(std::numeric_limits<std::uint64_t>::max());}catch(const InputFailure&){}reader.reset();}started=false;window_frames=0;binding.lost(reason,unavailable);}
  void begin_file(){
    file.value=CreateFileW(path.c_str(),GENERIC_WRITE,FILE_SHARE_READ,nullptr,CREATE_NEW,FILE_ATTRIBUTE_NORMAL,nullptr);if(file.value==INVALID_HANDLE_VALUE)throw std::runtime_error("capture destination already exists");
    std::vector<BYTE> header={'c','a','f','f',0,1,0,0,'d','e','s','c'};
    auto be=[&](std::uint64_t value,int width){for(int i=width-1;i>=0;--i)header.push_back(BYTE(value>>(8*i)));};
    be(32,8);double rate=48000;std::uint64_t bits;std::memcpy(&bits,&rate,8);be(bits,8);header.insert(header.end(),{'l','p','c','m'});for(auto value:{3u,channels*4,1u,channels,32u})be(value,4);header.insert(header.end(),{'d','a','t','a'});be(~std::uint64_t(0),8);be(0,4);write_all(file.value,header.data(),header.size());
  }
  void append(const float* samples,UINT32 count,std::uint64_t raw,std::uint64_t position,DWORD flags,std::uint64_t arrival){
    if(holding_for_window)return;
    const bool silent=(flags&AUDCLNT_BUFFERFLAGS_SILENT)!=0;
    discontinuities+=(flags&AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY)!=0;
    auto previous=recent.empty()?Json(nullptr):recent.back()["rawQpc100ns"];
    auto previous_position=recent.empty()?Json(nullptr):recent.back()["devicePositionFrames"];
    auto previous_end=recent.empty()?Json(nullptr):recent.back()["mappedEnd100ns"];
    const auto packet=clock.observe(count,raw,position,flags,arrival);
    auto time=packet.start;const auto original_count=count;UINT32 skipped=0;
    if(window_end){
      if(time>=window_end)count=0;
      else {
        if(time<window_start){auto skip=std::min<std::uint64_t>(count,((window_start-time)*48000+9999999)/10000000);time+=skip*10000000/48000;count-=UINT32(skip);skipped=UINT32(skip);if(!silent)samples+=skip*channels;}
        count=UINT32(std::min<std::uint64_t>(count,(window_end-time)*48000/10000000));
      }
    }
    Json entry={{"packet",++observed},{"streamEpoch",clock.stream_epoch},{"rawQpc100ns",raw},{"previousRawQpc100ns",previous},{"devicePositionFrames",position},{"previousDevicePositionFrames",previous_position},{"previousMappedEnd100ns",previous_end},{"flags",flags},{"arrivalQpc100ns",arrival},{"packetFrames",original_count},{"mappedStart100ns",packet.start},{"mappedEnd100ns",packet.end},{"source",packet.source},{"reason",packet.reason},{"selectedFrames",count},{"skippedFrames",skipped}};
    if(recent.size()==32)recent.erase(recent.begin());recent.push_back(entry);
    if(std::string_view(packet.reason)!="none"&&anomalies.size()<16)anomalies.push_back(entry);
    if(!count)return;if(!first)first=time;if(file.value==INVALID_HANDLE_VALUE)begin_file();
    std::vector<float> zeros;if(silent){zeros.assign(std::size_t(count)*channels,0);samples=zeros.data();}
    double sum=0,maximum=0;for(std::size_t i=0;i<std::size_t(count)*channels;++i){if(!std::isfinite(samples[i])||std::abs(samples[i])>16)throw std::runtime_error("invalid captured PCM amplitude");sum+=double(samples[i])*samples[i];maximum=std::max(maximum,double(std::abs(samples[i])));}
    clock.append(frames,count,time,packet.boundary);
    if(this->raw)this->raw->append(samples,count,clock.parts.back().last_end-std::uint64_t(count)*10000000/48000,packet.boundary);
    write_all(file.value,samples,std::size_t(count)*channels*4);frames+=count;++packets;rms=std::sqrt(sum/(double(count)*channels));peak=maximum;window_frames=count;measured=qpc();
  }
  void pump(std::uint64_t budget=4800){if(!reader)return;std::uint64_t frames_written=0;while(frames_written<budget){auto packet=reader->queue.pop();if(!packet)break;append(packet->samples.data(),packet->frames,packet->time,packet->position,packet->flags,packet->arrival);frames_written+=packet->frames;}}
  bool queued(){return reader&&reader->queue.pending_frames()!=0;}
  Json diagnostics(){return Json{{"observedPackets",observed},{"savedFrames",frames},{"discontinuityPackets",discontinuities},{"acquisitionQueuePeakFrames",std::max(peak_queued_frames,reader?reader->queue.peak():0ULL)},{"acquisitionQueueLimitFrames",PacketQueue::max_frames},{"equalTimestampPackets",clock.equal_timestamps},{"reversedTimestampPackets",clock.reversed_timestamps},{"timestampErrorPackets",clock.timestamp_errors},{"devicePositionFallbackPackets",clock.device_fallbacks},{"sampleCountFallbackPackets",clock.frame_fallbacks},{"estimatedPackets",clock.estimated_packets},{"timingEstimated",clock.estimated_packets!=0},{"firstAnomalies",anomalies},{"recentPackets",recent}};}
  void close(){cancel_preparation();if(reader){reader->stop();peak_queued_frames=std::max(peak_queued_frames,reader->queue.peak());}if(started){try{pump(std::numeric_limits<std::uint64_t>::max());}catch(const InputFailure&){}started=false;}if(file.value!=INVALID_HANDLE_VALUE){auto length=frames*channels*4+4;BYTE size[8];for(int i=0;i<8;++i)size[i]=BYTE(length>>(56-i*8));LARGE_INTEGER offset;offset.QuadPart=56;if(!SetFilePointerEx(file.value,offset,nullptr,FILE_BEGIN))throw std::runtime_error("CAF finalization seek failed");write_all(file.value,size,8);if(!capture_flush(file.value,"caf_finalize_sync"))throw std::runtime_error("CAF finalization flush failed");CloseHandle(file.value);file.value=INVALID_HANDLE_VALUE;}}
  bool acquisition_closed() const{return !preparation||preparation->finished();}
  Json track(std::uint64_t origin){Json segments=Json::array();for(auto& p:clock.parts)segments.push_back(Json{{"sourceFrameStart",p.first},{"sourceFrameEnd",p.first+p.frames},{"timelineStartUs",(p.start-origin)/10},{"timelineEndUs",(p.last_end-origin)/10},{"mappingJson","{\"schema_version\":1,\"method\":\"linear\"}"}});if(raw)segments=raw->timeline(origin);return Json{{"frames",frames},{"sampleRate",frames?Json(48000):Json(nullptr)},{"channels",frames?Json(channels):Json(nullptr)},{"appendFailures",0},{"callbackCount",packets},{"clockSegments",segments},{"clockBoundaryAdjustments",clock.boundary_adjustments},{"maxClockBoundaryAdjustmentNs",clock.max_boundary_adjustment_ticks*100}};}
  Json level(){if(!window_frames)return Json{{"state","waiting"},{"windowFrames",nullptr},{"ageMs",nullptr},{"rms",nullptr},{"peak",nullptr}};return Json{{"state","measured"},{"windowFrames",window_frames},{"ageMs",(qpc()-measured)/10000},{"rms",rms},{"peak",peak}};}
};
struct Settings {fs::path output;std::string owner,mode="system",uid,scenario,identity;DWORD pid=0,parent=0,synthetic_startup_delay_ms=0;bool test=false,synthetic=false,synthetic_clock_overlap=false,synthetic_clock_cases=false,synthetic_clock_resampling=false,synthetic_clock_drift=false,lease_stdin=false;};
Settings parse(int argc,wchar_t** argv) {
  Settings s;
  for(int i=1;i<argc;++i) {
    std::wstring flag=argv[i];
    if(flag==L"--stop-on-file"||flag==L"--until-stopped")continue;
    if(flag==L"--synthetic"){s.synthetic=true;continue;}
    if(flag==L"--synthetic-clock-overlap"){s.synthetic_clock_overlap=true;continue;}
    if(flag==L"--synthetic-clock-cases"){s.synthetic_clock_cases=true;continue;}
    if(flag==L"--synthetic-clock-resampling"){s.synthetic_clock_resampling=true;continue;}
    if(flag==L"--synthetic-clock-drift"){s.synthetic_clock_drift=true;continue;}
    if(i+1>=argc)throw std::runtime_error("missing product capture argument");
    auto value=std::wstring(argv[++i]);
    if(flag==L"--output")s.output=value;
    else if(flag==L"--mode")s.mode=utf8(value);
    else if(flag==L"--microphone-uid")s.uid=utf8(value);
    else if(flag==L"--input-level-owner")s.owner=utf8(value);
    else if(flag==L"--five-second-test"){s.test=true;s.owner=utf8(value);}
    else if(flag==L"--pid")s.pid=DWORD(std::stoul(value));
    else if(flag==L"--parent-pid")s.parent=DWORD(std::stoul(value));
    else if(flag==L"--application-identity")s.identity=utf8(value);
    else if(flag==L"--scenario")s.scenario=utf8(value);
    else if(flag==L"--synthetic-startup-delay-ms")s.synthetic_startup_delay_ms=DWORD(std::stoul(value));
    else if(flag==L"--capture-test-lease-stdin"){if(value!=L"1")throw std::runtime_error("invalid test lease");s.lease_stdin=true;}
    else if(flag==L"--backend"){if(value!=L"wasapi")throw std::runtime_error("unsupported capture backend");}
    else throw std::runtime_error("unknown product capture argument");
  }
  if(!s.output.is_absolute()||s.owner.size()!=36||s.owner.find_first_not_of("0123456789abcdef-")!=std::string::npos||!s.parent||(s.mode!="system"&&s.mode!="application")||(s.mode=="application")!=bool(s.pid)
    ||s.synthetic_startup_delay_ms>5000||(!s.synthetic&&(s.synthetic_startup_delay_ms||s.synthetic_clock_overlap||s.synthetic_clock_cases||s.synthetic_clock_resampling||s.synthetic_clock_drift)))
    throw std::runtime_error("invalid product capture ownership or scope");
  return s;
}
int run(int argc,wchar_t** argv){
  check(CoInitializeEx(nullptr,COINIT_MULTITHREADED),"initialize product capture COM");
  if(argc==2&&std::wstring(argv[1])==L"--list-input-devices"){std::cout<<microphones().dump()<<std::endl;return 0;}
  if(argc==2&&std::wstring(argv[1])==L"--list-capture-applications"){std::cout<<applications().dump()<<std::endl;return 0;}
  Settings s=parse(argc,argv);Json manifest,journal_persistence=nullptr;std::unique_ptr<MetadataWorker> metadata;std::unique_ptr<Lane> mic_lane,system_lane;
  IdleSleepGuard awake;
  auto diagnostics=[&](const char* state,const Json& error){
    if(mic_lane&&system_lane){auto diagnostic=Json{{"schemaVersion",1},{"ownerId",s.owner},{"state",state},{"error",error},{"qpcUnit","100ns"},{"sampleRate",48000},{"microphone",mic_lane->diagnostics()},{"system",system_lane->diagnostics()}};if(metadata)metadata->observe(L"capture-clock.json",std::move(diagnostic));else atomic_json(s.output/L"capture-clock.json",diagnostic,false);}
  };
  try {
    if(s.lease_stdin){
      auto handle=GetStdHandle(STD_INPUT_HANDLE);BY_HANDLE_FILE_INFORMATION info{};wchar_t path[32768];
      auto size=GetFinalPathNameByHandleW(handle,path,32768,0);
      if(!s.test||GetFileType(handle)!=FILE_TYPE_DISK||!GetFileInformationByHandle(handle,&info))throw std::runtime_error("録音前テストの永続所有ハンドルを取得できません");
      if(info.nNumberOfLinks!=1||(info.dwFileAttributes&(FILE_ATTRIBUTE_REPARSE_POINT|FILE_ATTRIBUTE_DIRECTORY))||info.nFileSizeHigh||info.nFileSizeLow||!size||size>=32768||fs::path(path).filename()!=wide("lease-"+s.owner))throw std::runtime_error("録音前テストの永続所有ファイルが一致しません");
      // GetFinalPathNameByHandle returns an extended path (\\?\), whereas
      // filesystem canonicalization may remove that prefix. Compare the held
      // parent directory identities instead of their textual spellings.
      auto parent_info=[](const fs::path& parent){
        Handle directory(CreateFileW(parent.c_str(),FILE_READ_ATTRIBUTES,FILE_SHARE_READ|FILE_SHARE_WRITE|FILE_SHARE_DELETE,nullptr,OPEN_EXISTING,FILE_FLAG_BACKUP_SEMANTICS|FILE_FLAG_OPEN_REPARSE_POINT,nullptr));
        BY_HANDLE_FILE_INFORMATION metadata{};
        if(directory.value==INVALID_HANDLE_VALUE||!GetFileInformationByHandle(directory.value,&metadata)||!(metadata.dwFileAttributes&FILE_ATTRIBUTE_DIRECTORY)||(metadata.dwFileAttributes&FILE_ATTRIBUTE_REPARSE_POINT))throw std::runtime_error("録音前テストの所有ディレクトリを確認できません");
        return metadata;
      };
      const auto actual=parent_info(fs::path(path).parent_path()),expected=parent_info(s.output.parent_path());
      if(actual.dwVolumeSerialNumber!=expected.dwVolumeSerialNumber||actual.nFileIndexHigh!=expected.nFileIndexHigh||actual.nFileIndexLow!=expected.nFileIndexLow||ticks(actual.ftCreationTime)!=ticks(expected.ftCreationTime))throw std::runtime_error("録音前テストの永続所有ディレクトリが一致しません");
    }
    auto attributes=GetFileAttributesW(s.output.c_str());if(attributes==INVALID_FILE_ATTRIBUTES||!(attributes&FILE_ATTRIBUTE_DIRECTORY)||(attributes&FILE_ATTRIBUTE_REPARSE_POINT)||!fs::is_empty(s.output))throw std::runtime_error("capture destination must be an empty ordinary directory");
    mic_lane=std::make_unique<Lane>(s.output/L"microphone.caf",1,s.test);system_lane=std::make_unique<Lane>(s.output/L"system.caf",2,s.test);auto& mic=*mic_lane;auto& system=*system_lane;
    mic.holding_for_window=system.holding_for_window=s.test;
    if(!s.test){mic.raw=std::make_unique<RawSegments>(s.output,s.owner,"microphone",1);system.raw=std::make_unique<RawSegments>(s.output,s.owner,"system",2);}
    manifest=Json{{"schemaVersion",4},{"captureMode",s.mode},{"backend","wasapi"},{"targetPIDs",s.pid?Json::array({s.pid}):Json::array()},{"targetApplicationIdentity",nullptr},{"microphoneDeviceUID",s.uid.empty()?Json(nullptr):Json(s.uid)},{"recordingOwnerId",s.owner},{"inputDiagnosticsState","reported"},{"state","starting"},{"errors",Json::array()}, {"microphone",mic.track(0)}, {"system",system.track(0)}};
    manifest["consoleWindowPresent"]=GetConsoleWindow()!=nullptr;
    manifest["clockDiagnosticsFile"]="capture-clock.json";
    // Every publication, including initialization failures, must be decodable
    // by the same strict reader used while the helper is still running.
    if(s.test)manifest["captureTest"]=Json{{"ownerId",s.owner},{"durationSeconds",5},{"windowStartNs",nullptr},{"windowEndNs",nullptr},{"firstSavedCaptureNs",nullptr},{"completed",false}};
    atomic_json(s.output/L"manifest.json",manifest);
    diagnostics("starting",nullptr);
    if(s.synthetic_startup_delay_ms)Sleep(s.synthetic_startup_delay_ms);
    Handle parent(OpenProcess(SYNCHRONIZE|PROCESS_QUERY_LIMITED_INFORMATION,FALSE,s.parent));if(!parent.value)throw std::runtime_error("capture parent unavailable");
    Handle application_process(s.pid?OpenProcess(SYNCHRONIZE|PROCESS_QUERY_LIMITED_INFORMATION,FALSE,s.pid):INVALID_HANDLE_VALUE);
    Json identity=nullptr;if(s.pid){if(!application_process.value||application_process.value==INVALID_HANDLE_VALUE||WaitForSingleObject(application_process.value,0)!=WAIT_TIMEOUT)throw std::runtime_error("capture application is unavailable");identity=process_identity(s.pid);if(Json::parse(s.identity)!=identity)throw std::runtime_error("capture application identity changed");}
    manifest["targetApplicationIdentity"]=identity;
    FILETIME wall;GetSystemTimePreciseAsFileTime(&wall);auto utc=(ticks(wall)-EPOCH)/10000;auto before=qpc();
    MicrophoneChoice choice{s.uid,"fixed",""};
    std::string selected=s.uid;
    auto reconnect=[&](Lane& lane,bool microphone){
      const auto now=GetTickCount64();
      if(lane.preparation){
        // Expiry wins over a result first observed after the deadline. Such a
        // result is never allowed to restart a timed-out or stopped input.
        if(lane.preparation_schedule.timed_out(now)){
          lane.cancel_preparation();lane.preparation_schedule.initial_resolved=true;lane.binding.lost("restart_failed");
        }
        if(!lane.preparation_schedule.cancelled){
          if(!microphone&&s.pid){
            try{if(process_identity(s.pid)!=identity)throw InputFailure(E_FAIL,"selected application identity changed");}
            catch(...){lane.cancel_preparation();lane.binding.lost("application_unavailable",true);return;}
          }
          if(auto ready=lane.preparation->take()){
            if(microphone){
              choice=ready->choice;selected=choice.uid;
              // The manifest records the initial input, matching the stored
              // start configuration. Later switches belong to selection
              // status/history, not a rewrite of that initial identity.
              if(manifest["microphoneDeviceUID"].is_null()){
                manifest["microphoneDeviceUID"]=selected;atomic_json(s.output/L"manifest.json",manifest);
              }
            }
            lane.adopt(std::move(*ready));lane.preparation.reset();return;
          }
        }
        if(lane.preparation->finished()){
          auto error=lane.preparation->error();lane.preparation.reset();lane.preparation_schedule.finish(now);
          if(error&&!lane.preparation_schedule.cancelled){
            try{std::rethrow_exception(error);}
            catch(const InputFailure& failure){lane.binding.lost(failure.result==E_ACCESSDENIED?"permission_unavailable":microphone?"device_disconnected":s.pid?"application_unavailable":"restart_failed",failure.result==E_ACCESSDENIED||(!microphone&&s.pid));}
            catch(...){lane.binding.lost(microphone?"device_disconnected":s.pid?"application_unavailable":"restart_failed",!microphone&&s.pid);}
          }
          // Includes cleanup of a timed-out stream; never overlap activations.
          lane.binding.next_attempt=lane.preparation_schedule.retry_at;
        }
        return;
      }
      if(lane.started||now<lane.binding.next_attempt||!lane.preparation_schedule.may_start(now))return;
      lane.preparation_schedule.begin(now);
      // Capture values only: a cancelled preparation can outlive this run.
      lane.preparation=std::make_unique<NativePreparation>([microphone,uid=s.uid,pid=s.pid,identity](NativePreparation::Cancellation cancelled){return prepare_input(microphone,uid,pid,identity,cancelled);});
    };
    if(s.synthetic){if(selected.empty())selected="synthetic-microphone";choice={selected,s.uid.empty()?"systemDefault":"fixed",s.uid.empty()?"mon_direct_recording":""};mic.binding.connected();system.binding.connected();mic.preparation_schedule.initial_resolved=system.preparation_schedule.initial_resolved=true;}
    else{reconnect(mic,true);reconnect(system,false);}
    std::uint64_t revision=0,selection_revision=0,synthetic_packet=0,last_diagnostics=0,reported_estimates=0;std::uint64_t start=s.test&&!s.synthetic?0:qpc();
    if(s.test&&start){mic.holding_for_window=system.holding_for_window=false;mic.window_start=system.window_start=start;mic.window_end=system.window_end=start+50000000ULL;}
    manifest["state"]=s.test&&!start?"starting":"recording";manifest["microphoneDeviceUID"]=selected.empty()?Json(nullptr):Json(selected);
    if(s.test&&start)manifest["captureTest"]=Json{{"ownerId",s.owner},{"durationSeconds",5},{"windowStartNs",start*100},{"windowEndNs",start*100+5000000000ULL},{"firstSavedCaptureNs",nullptr},{"completed",false}};
    atomic_json(s.output/L"manifest.json",manifest);
    metadata=std::make_unique<MetadataWorker>(s.output,s.owner);
    auto selection=[&]{auto value=microphone_selection_status(s.owner,revision,selection_revision,s.uid,selected,choice,!s.synthetic&&!mic.started);if(!value.is_null())metadata->observe(L"microphone-selection.status.json",std::move(value));};
    auto publish_status=[&](bool sealed){
      Json report={{"schemaVersion",3},{"ownerId",s.owner},{"revision",++revision},{"microphone",mic.binding.status()},{"system",system.binding.status()},{"microphoneHistory",mic.binding.history()},{"systemHistory",system.binding.history()}};
      metadata->publish(std::move(report),sealed);
    };
    using SelectionPreparation=InputPreparation<MicrophoneChoice,CaptureApartment>;
    std::unique_ptr<SelectionPreparation> selection_preparation;
    PreparationSchedule selection_schedule;
    std::uint64_t next_status=0;
    while(true){
      if(WaitForSingleObject(parent.value,0)!=WAIT_TIMEOUT)throw std::runtime_error("capture parent exited");
      if(fs::exists(s.output/L"stop.request"))break;
      // Writer latency is not a clock change or a WASAPI device failure.
      if(!s.synthetic&&s.pid&&system.started){try{if(process_identity(s.pid)!=identity)system.detach("application_unavailable",true);}catch(...){system.detach("application_unavailable",true);}}
      if(s.synthetic){
        if(synthetic_packet==0&&s.scenario=="system-only")mic.binding.lost("device_disconnected",true);
        if(s.scenario=="input-recovery"){
          if(synthetic_packet==25)mic.binding.lost("device_disconnected");
          if(synthetic_packet==60)mic.binding.connected();
          if(synthetic_packet==80)system.binding.lost("application_unavailable",true);
          if(synthetic_packet==120)system.binding.connected();
        }
        // Deterministic driver-shaped timestamps for hardware-free regressions.
        auto time=start+synthetic_packet*100000-(s.synthetic_clock_overlap&&synthetic_packet?10:0);
        if(s.synthetic_clock_drift)time+=std::min(synthetic_packet,399ULL)*10;
        auto position=synthetic_packet*480;UINT32 count=480;DWORD flags=s.synthetic_clock_overlap&&synthetic_packet==1?1:0;
        if(s.synthetic_clock_resampling){time=start+synthetic_packet*75000;position=synthetic_packet*160;count=479;}
        if(s.synthetic_clock_cases){
          if(synthetic_packet==1)time=start;
          if(synthetic_packet==2)time=start-100;
          if(synthetic_packet==3){time=0;flags|=4;}
          if(synthetic_packet>=4)position=(synthetic_packet-4)*480;
          if(synthetic_packet==4)flags|=1;
          if(synthetic_packet>=7)position+=960;
        }
        // Synthetic timestamps advance independently of the scheduler and may
        // lead the host clock; use their simulated arrival, not real hardware.
        auto arrival=std::max(qpc(),start+(synthetic_packet+1)*100000);
        std::vector<float> tone(count,0.125f);if(mic.binding.state=="connected")mic.append(tone.data(),count,time,position,flags,arrival);
        std::vector<float> silence(count*2,0);if(system.binding.state=="connected")system.append(silence.data(),count,time,position,flags,arrival);++synthetic_packet;
      }
      else{
        // Endpoint/session enumeration can block too. Poll it independently of
        // PCM writes and stop requests, with one outstanding query and no Lane
        // references in its worker. An obsolete query cannot select a device.
        const auto selection_now=GetTickCount64();
        if(selection_preparation){
          if(!s.uid.empty()||!mic.started||selection_schedule.timed_out(selection_now)){selection_preparation->cancel();selection_schedule.cancel();}
          if(!selection_schedule.cancelled){
            if(auto next=selection_preparation->take()){
              selection_preparation.reset();selection_schedule.accept();selection_schedule.retry_at=selection_now+PreparationSchedule::retry_ms;
              if(next->uid!=selected)mic.detach("device_changed");choice=std::move(*next);
            }
          }
          if(selection_preparation&&selection_preparation->finished()){
            auto error=selection_preparation->error();selection_preparation.reset();selection_schedule.finish(selection_now);
            if(error&&!selection_schedule.cancelled&&mic.started)mic.detach("device_disconnected");
          }
        }
        if(s.uid.empty()&&mic.started&&!selection_preparation&&selection_schedule.may_start(selection_now)){
          selection_schedule.begin(selection_now);
          selection_preparation=std::make_unique<SelectionPreparation>([uid=s.uid,pid=s.pid,identity](auto){return current_microphone_choice(uid,pid,identity);});
        }
        for(auto lane:{&mic,&system})if(lane->started){try{lane->pump();}catch(const InputFailure& error){lane->detach(error.result==E_ACCESSDENIED?"permission_unavailable":lane==&mic?"device_disconnected":"stream_stopped");}}
        reconnect(mic,true);reconnect(system,false);
        if(s.test&&!start&&mic.preparation_schedule.initial_resolved&&system.preparation_schedule.initial_resolved){
          start=qpc();mic.holding_for_window=system.holding_for_window=false;
          mic.window_start=system.window_start=start;mic.window_end=system.window_end=start+50000000ULL;
          manifest["state"]="recording";manifest["microphoneDeviceUID"]=selected.empty()?Json(nullptr):Json(selected);
          manifest["captureTest"]=Json{{"ownerId",s.owner},{"durationSeconds",5},{"windowStartNs",start*100},{"windowEndNs",start*100+5000000000ULL},{"firstSavedCaptureNs",nullptr},{"completed",false}};
          atomic_json(s.output/L"manifest.json",manifest);
        }
      }
      const auto estimates=mic.clock.estimated_packets+system.clock.estimated_packets;
      if(!last_diagnostics||qpc()-last_diagnostics>=10000000||(estimates&&!reported_estimates)){
        ULARGE_INTEGER available{};if(!GetDiskFreeSpaceExW(s.output.c_str(),&available,nullptr,nullptr))throw std::runtime_error("保存先の空き容量を確認できません。確定済みの音声は保持しています");if(available.QuadPart<16ULL*1024*1024)throw std::runtime_error("保存先の空き容量が不足しています。確定済みの原音声は保持しています");
        diagnostics("recording",nullptr);last_diagnostics=qpc();reported_estimates=estimates;
      }
      if(GetTickCount64()>=next_status){
        next_status=GetTickCount64()+250;++revision;
        metadata->observe(L"input-levels.json",Json{{"schemaVersion",1},{"ownerId",s.owner},{"revision",revision},{"microphone",mic.level()},{"system",system.level()}});
        publish_status(false);selection();
      }
      auto request=s.output/L"microphone-selection.request";
      if(fs::exists(request)){
        if(fs::file_size(request)>4096)throw std::runtime_error("microphone selection request exceeds limit");std::ifstream input(request,std::ios::binary);Json next=Json::parse(input);input.close();
        if(next.at("schemaVersion")!=1||next.at("ownerId")!=s.owner)throw std::runtime_error("microphone selection ownership differs");
        auto next_revision=next.at("revision").get<std::uint64_t>();if(next_revision>selection_revision){
          std::string uid=next.at("uid").is_null()?"":next.at("uid").get<std::string>();
          // A running lane keeps its file and timeline. Restart only its input.
          if(selection_preparation){selection_preparation->cancel();selection_schedule.cancel();}
          mic.detach("device_changed");s.uid=uid;selection_revision=next_revision;
          if(s.synthetic){selected=uid.empty()?"synthetic-microphone":uid;choice={selected,uid.empty()?"systemDefault":"fixed",uid.empty()?"mon_direct_recording":""};mic.binding.connected();}
          else{mic.binding.next_attempt=0;reconnect(mic,true);}publish_status(false);selection();
        }
      }
      if(fs::exists(s.output/L"stop.request")||(s.synthetic?synthetic_packet>=500:s.test&&start&&qpc()-start>=50000000ULL))break;
      if(s.synthetic||(!mic.queued()&&!system.queued()))Sleep(10);
    }
    if(selection_preparation)selection_preparation->cancel();
    mic.close();auto mic_closed=std::max(qpc(),mic.clock.parts.empty()?0ULL:mic.clock.parts.back().last_end);system.close();auto closed=std::max(qpc(),system.clock.parts.empty()?0ULL:system.clock.parts.back().last_end);closed=std::max(closed,mic_closed);
    // Preserve the buffered tail even when an unreturned native preparation
    // prevents producer-closure proof. Only finish() may publish closed=true.
    if(mic.raw){mic.raw->seal();system.raw->seal();}
    if(!mic.acquisition_closed()||!system.acquisition_closed())throw std::runtime_error("音声入力の準備処理の終了を確認できません。確定済み原音声を保持しています。録音を回復してください");
    if(mic.raw){mic.raw->finish();system.raw->finish();}mic.binding.stopped();system.binding.stopped();publish_status(true);metadata->finish();journal_persistence=metadata->persistence();if(!metadata->error().empty()){manifest["inputDiagnosticsState"]="unconfirmed";manifest["inputEventJournalError"]=metadata->error();}metadata.reset();
    if(!mic.frames&&!system.frames)throw std::runtime_error("マイク・PC音声を取得できませんでした。Windowsのマイク権限と入力機器を確認してください");
    auto origin=mic.frames?(system.frames?std::min(mic.first,system.first):mic.first):system.first;
    manifest["microphone"]=mic.track(origin);manifest["system"]=system.track(origin);manifest["systemAudioUnreceived"]=system.frames==0;
    manifest["microphoneAudioUnreceived"]=mic.frames==0;
    if(mic.raw){manifest["microphone"]["rawSegments"]=mic.raw->status();manifest["system"]["rawSegments"]=system.raw->status();manifest["clockMappingSource"]="sealed_raw_v1";}
    auto uncertainty=std::max(1000000ULL,std::max(mic.clock.max_boundary_adjustment_ticks,system.clock.max_boundary_adjustment_ticks)*100);
    manifest["timingEstimated"]=(mic.clock.estimated_packets+system.clock.estimated_packets)!=0;
    // Estimated clocks cannot claim the normal 1 ms precision. Retain a
    // conservative whole-recording envelope and explicitly label its origin.
    manifest["captureClock"]=Json{{"domain","query_performance_counter"},{"originUtcMs",static_cast<std::int64_t>(utc)+(static_cast<std::int64_t>(origin)-static_cast<std::int64_t>(before))/10000},{"uncertaintyNs",manifest["timingEstimated"]==true?std::max(uncertainty,(closed-origin)*100+1000000):uncertainty}};
    manifest["captureEnd"]=Json{{"method","acquisition_closed"},{"microphoneClosedUs",(mic_closed-origin)/10},{"systemClosedUs",(closed-origin)/10},{"endUs",(closed-origin)/10}};
    manifest["state"]="saved";if(s.test){manifest["captureTest"]["completed"]=true;manifest["captureTest"]["firstSavedCaptureNs"]=origin*100;}
    if(!journal_persistence.is_null())manifest["inputEventJournalPersistence"]=journal_persistence;
    metadata.reset();diagnostics("saved",nullptr);atomic_json(s.output/L"manifest.json",manifest);return 0;
  }catch(const std::exception& error){
    try{diagnostics("failed",error.what());}catch(...){}
    if(!manifest.is_null()){
      manifest["inputDiagnosticsState"]="unconfirmed";manifest["inputEventJournalError"]="録音異常終了のため履歴の停止時封印を確認できません";
      if(mic_lane&&system_lane){auto origin=mic_lane->frames?mic_lane->first:system_lane->first;if(system_lane->frames)origin=std::min(origin,system_lane->first);manifest["microphone"]=mic_lane->track(origin);manifest["system"]=system_lane->track(origin);if(mic_lane->raw){manifest["microphone"]["rawSegments"]=mic_lane->raw->status();manifest["system"]["rawSegments"]=system_lane->raw->status();}}
      manifest["state"]="failed";manifest["errors"]=Json::array({error.what()});try{atomic_json(s.output/L"manifest.json",manifest);}catch(...){}
    }std::cerr<<error.what()<<std::endl;return 1;
  }
}
} // namespace product
