#pragma once
#include "json.hpp"
#include "packet_clock.h"
#include "voice_pcm.h"
#include "packet_queue.h"
#include "input_preparation.h"
#include "endpoint_mixer.h"
#include "synthetic_driver.h"
#include <avrt.h>
#include <functiondiscoverykeys_devpkey.h>
#include <tlhelp32.h>
#include <audiopolicy.h>
#include <endpointvolume.h>
#include <cstring>
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
#include <limits>

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
DWORD capture_parent_pid(){
  Handle snapshot(CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS,0));
  if(snapshot.value==INVALID_HANDLE_VALUE)throw std::runtime_error("capture parent metadata unavailable");
  PROCESSENTRY32W item{};item.dwSize=sizeof(item);
  if(Process32FirstW(snapshot.value,&item)){do{if(item.th32ProcessID==GetCurrentProcessId())return item.th32ParentProcessID;}while(Process32NextW(snapshot.value,&item));}
  throw std::runtime_error("capture parent metadata unavailable");
}
Json normalized_catalog(const Json& rows,bool applications){
  const auto encoded=rows.dump();std::uint32_t indices[128];
  const auto count=mon_capture_catalog_order(reinterpret_cast<const std::uint8_t*>(encoded.data()),encoded.size(),applications,
    GetCurrentProcessId(),applications?capture_parent_pid():0,indices,128);
  if(count<0)throw std::runtime_error("capture catalog exceeds capacity or has invalid identities");
  Json ordered=Json::array();for(std::ptrdiff_t i=0;i<count;++i)ordered.push_back(rows.at(indices[i]));return ordered;
}
Json applications(){
  const auto self=GetCurrentProcessId(),parent=capture_parent_pid();
  std::set<DWORD> pids;EnumWindows([](HWND window,LPARAM context)->BOOL{if(IsWindowVisible(window)&&GetWindowTextLengthW(window)>0){DWORD pid;GetWindowThreadProcessId(window,&pid);reinterpret_cast<std::set<DWORD>*>(context)->insert(pid);}return TRUE;},reinterpret_cast<LPARAM>(&pids));
  Json list=Json::array();for(auto pid:pids){if(pid==self||pid==parent)continue;try{auto app=process_identity(pid);app["name"]=utf8(fs::path(wide(app["bundleIdentifier"].get<std::string>())).stem().wstring());list.push_back(app);}catch(const std::exception&){}}
  return Json{{"schemaVersion",1},{"systemPermission","authorized"},{"applications",normalized_catalog(list,true)}};
}
std::wstring endpoint_id(IMMDevice* device){LPWSTR id=nullptr;check(device->GetId(&id),"get endpoint identity");std::wstring value(id);CoTaskMemFree(id);return value;}
ComPtr<IMMDeviceEnumerator> enumerator(){ComPtr<IMMDeviceEnumerator> e;check(CoCreateInstance(__uuidof(MMDeviceEnumerator),nullptr,CLSCTX_ALL,IID_PPV_ARGS(e.GetAddressOf())),"enumerate audio endpoints");return e;}
#include "audio_inspection.h"
Json microphones(){
  auto e=enumerator();ComPtr<IMMDevice> selected;std::wstring current;if(SUCCEEDED(e->GetDefaultAudioEndpoint(eCapture,eCommunications,&selected)))current=endpoint_id(selected.Get());
  ComPtr<IMMDeviceCollection> collection;check(e->EnumAudioEndpoints(eCapture,DEVICE_STATE_ACTIVE,&collection),"list microphones");UINT count=0;check(collection->GetCount(&count),"count microphones");Json devices=Json::array();
  if(count>128)throw std::runtime_error("too many input devices");
  for(UINT i=0;i<count;++i){ComPtr<IMMDevice> device;check(collection->Item(i,&device),"read microphone");auto id=endpoint_id(device.Get());std::string name=utf8(id);ComPtr<IPropertyStore> props;if(SUCCEEDED(device->OpenPropertyStore(STGM_READ,&props))){PROPVARIANT value;PropVariantInit(&value);if(SUCCEEDED(props->GetValue(PKEY_Device_FriendlyName,&value))&&value.vt==VT_LPWSTR)name=utf8(value.pwszVal);PropVariantClear(&value);}devices.push_back(Json{{"uid",utf8(id)},{"name",name},{"isDefault",id==current}});}
  return Json{{"schemaVersion",1},{"microphonePermission","unknown"},{"devices",normalized_catalog(devices,false)}};
}
#include "input_binding.h"
#include "endpoint_changes.h"
#include "raw_segments.h"
#include "live_wav.h"
#include "async_capture.h"
struct CaptureApartment {
  CaptureApartment(){acquisition(CoInitializeEx(nullptr,COINIT_MULTITHREADED),"initialize WASAPI preparation thread");}
  ~CaptureApartment(){CoUninitialize();}
};
#include "system_capture.h"
struct PreparedInput {
  ComPtr<IAudioClient> client;
  std::unique_ptr<CaptureReader> reader;
  MicrophoneChoice choice;
  bool matching_position_rate=false;
  PreparedInput()=default;
  PreparedInput(PreparedInput&& other)noexcept{swap(other);}
  PreparedInput& operator=(PreparedInput&& other)noexcept{swap(other);return *this;}
  void swap(PreparedInput& other)noexcept{client.Swap(other.client);reader.swap(other.reader);std::swap(choice,other.choice);std::swap(matching_position_rate,other.matching_position_rate);}
  ~PreparedInput(){if(reader)reader->stop();}
};
using NativePreparation=InputPreparation<PreparedInput,CaptureApartment>;
using NativeRetirement=InputPreparation<std::shared_ptr<CaptureReader>,CaptureApartment>;
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
  std::shared_ptr<EndpointLifetime> lifetime;
  auto current=[&]{
    if(cancelled())throw InputFailure(E_ABORT,"obsolete WASAPI preparation");
    if(lifetime)lifetime->require_current();
    if(!microphone&&selected_pid&&process_identity(selected_pid)!=identity)throw InputFailure(E_FAIL,"selected application identity changed");
  };
  current();
  if(!microphone&&!selected_pid){
    result.reader=std::make_unique<SystemCapture>();result.matching_position_rate=true;
    current();return result;
  }
  if(microphone){
    result.choice=current_microphone_choice(uid,selected_pid,identity);
    current();auto e=enumerator();ComPtr<IMMDevice> device;
    lifetime=std::make_shared<EndpointLifetime>(e,wide(result.choice.uid));
    current();
    acquisition(e->GetDevice(wide(result.choice.uid).c_str(),&device),"get selected microphone");
    acquisition(device->Activate(__uuidof(IAudioClient),CLSCTX_ALL,nullptr,reinterpret_cast<void**>(result.client.GetAddressOf())),"activate WASAPI microphone");
  }else{
    if(selected_pid&&process_identity(selected_pid)!=identity)throw InputFailure(E_FAIL,"selected application identity changed");
    // Selected-app capture never falls back to global endpoint loopback.
    // The worker retains an unfinished native activation even after UI timeout.
    result.client=activate_process_loopback(selected_pid,false,INFINITE);
  }
  current();
  WAVEFORMATEX* mix=nullptr;
  result.matching_position_rate=SUCCEEDED(result.client->GetMixFormat(&mix))&&mix&&mix->nSamplesPerSec==48000;
  if(microphone)std::cerr<<"mon_capture_input: "+Json{{"event","open"},{"uid",result.choice.uid},{"source",result.choice.source},{"nativeRate",mix?Json(mix->nSamplesPerSec):Json(nullptr)},{"nativeChannels",mix?Json(mix->nChannels):Json(nullptr)}}.dump()+"\n";
  if(mix)CoTaskMemFree(mix);
  current();
  const UINT32 channels=microphone?1:2;
  WAVEFORMATEX format={};format.wFormatTag=WAVE_FORMAT_IEEE_FLOAT;format.nChannels=WORD(channels);format.nSamplesPerSec=48000;format.wBitsPerSample=32;format.nBlockAlign=WORD(channels*4);format.nAvgBytesPerSec=48000*format.nBlockAlign;
  acquisition(result.client->Initialize(AUDCLNT_SHAREMODE_SHARED,AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM|AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY|(microphone?0:AUDCLNT_STREAMFLAGS_LOOPBACK),5000000,0,&format,nullptr),"initialize 48 kHz floating-point WASAPI");
  current();result.reader=std::make_unique<AsyncCapture>(std::move(result.client),channels,microphone,result.choice.uid,lifetime);
  current();
  return result;
}
class Lane {
 public:
  fs::path path;UINT32 channels;std::uint64_t frames=0,packets=0,first=0;double rms=0,peak=0;UINT32 window_frames=0;std::uint64_t measured=0;PacketClock clock;
  Handle file;
  std::uint64_t converted_epoch=0;
  std::uint64_t admitted_sequence=0;
  bool started=false,holding_for_window=false;std::uint64_t window_start=0,window_end=0;
  Binding binding;
  std::unique_ptr<RawSegments> raw;
  std::unique_ptr<WavInput> wav;
  std::unique_ptr<CaptureReader> reader;
  std::shared_ptr<CaptureReader> retiring_reader;
  std::unique_ptr<NativeRetirement> retirement;
  bool retirement_queue_ended=false,retirement_fault_reported=false,retirement_wait_expired=false;
  std::unique_ptr<NativePreparation> preparation;
  PreparationSchedule preparation_schedule;
  MonCaptureInputRetry input_retry{};
  unsigned input_fault_reports=0;
  void acquisition_failed(const char* reason,bool unavailable=false,std::uint64_t now_ms=GetTickCount64()){
    // A Bluetooth stream can deliver a few valid (even silent) packets and
    // then stall. Reopening it every second repeatedly changes its profile.
    // Only sustained PCM reception in append(), not Start/adopt, clears retry.
    mon_capture_input_fault(&input_retry,now_ms*1000000ULL);
    detach(reason,unavailable);
  }
  void input_fault(){
    mon_capture_input_fault(&input_retry,GetTickCount64()*1000000ULL);
    if(input_fault_reports<16){++input_fault_reports;std::cerr<<"mon_capture_input_fault: lane="<<(channels==1?"microphone":"system")<<" attempt="<<input_retry.failures<<std::endl;}
    detach("invalid_audio_data",true);
  }
  std::uint64_t peak_queued_frames=0,discontinuities=0;
  Json recent=Json::array(),anomalies=Json::array();std::uint64_t observed=0;
  AlignmentDiagnostics previous_alignment;
  Json alignment_diagnostics(){
    auto value=previous_alignment;if(reader)value.merge(reader->alignment_diagnostics());if(retiring_reader)value.merge(retiring_reader->alignment_diagnostics());
    Json losses=Json::array();for(const auto& entry:value.losses)losses.push_back(Json{{"startQpc100ns",EndpointMixer::ticks_at(entry.first)},{"frames",entry.frames},{"reason",entry.reason},{"mappedStartQpc100ns",EndpointMixer::ticks_at(entry.mapped_first)}});
    return Json{{"latePackets",value.late_packets},{"lateFrames",value.late_frames},{"retimedPackets",value.retimed_packets},{"retimedFrames",value.retimed_frames},{"clockAdjustmentFrames",value.clock_adjustment_frames},{"pressureFlushes",value.pressure_flushes},{"discardedFrames",0},{"capacityFrames",value.capacity_frames},{"resyncFrames",value.reset_frames},{"ranges",losses},{"rangesTruncated",value.truncated}};
  }
  Lane(fs::path output,UINT32 count,bool bounded=false):path(std::move(output)),channels(count),clock(bounded?PacketClock::bounded_capacity:PacketClock::recording_capacity){}
  ~Lane(){if(reader)reader->stop();}
  void cancel_preparation(){if(preparation){preparation->cancel();preparation_schedule.cancel();}}
  bool is_recording()const{return raw||(wav&&wav->pending());}
  void adopt(PreparedInput input){if(reader||retirement||retiring_reader)throw std::logic_error("input handoff overlaps owned retirement");if(raw)input.reader->queue.attach_storage(raw->writer->pending(),raw->lane_index);else if(wav&&wav->pending())input.reader->queue.attach_storage(wav->pending(),wav->lane_index());clock.begin_stream(input.matching_position_rate);admitted_sequence=0;reader=std::move(input.reader);started=true;binding.connected();preparation_schedule.accept();}
  void detach(const char* reason,bool unavailable=false){
    cancel_preparation();
    if(started)std::cerr<<"mon_capture_reconnect: lane="<<(channels==1?"microphone":"system")<<" reason="<<reason<<std::endl;
    started=false;window_frames=0;binding.lost(reason,unavailable);
    if(reader){
      if(retirement||retiring_reader)throw std::logic_error("overlapping input retirement");
      retiring_reader=std::shared_ptr<CaptureReader>(std::move(reader));
      retirement_queue_ended=false;retirement_wait_expired=false;
      retirement_fault_reported=std::string_view(reason)=="invalid_audio_data";
      // Stop/COM cleanup may wait for a Bluetooth profile. The control thread
      // keeps pumping accepted PCM and publishing both inputs while this one
      // owned worker waits. It never opens a replacement before real closure.
      retirement=std::make_unique<NativeRetirement>([source=retiring_reader](auto){source->stop();return source;});
    }
  }
  void poll_retirement(){
    if(!retirement)return;
    const auto drain=[&](std::uint64_t budget){
      try{pump(budget);}
      catch(const InputFailure&){retirement_queue_ended=true;}
      catch(const InputDataFault&){
        retirement_queue_ended=true;if(!is_recording())throw;
        if(!retirement_fault_reported){
          retirement_fault_reported=true;mon_capture_input_fault(&input_retry,GetTickCount64()*1000000ULL);
          binding.lost("invalid_audio_data",true);
        }
      }
    };
    if(!retirement_queue_ended)drain(4800);
    // Taking the quiesced reader releases the worker's acceptance handshake;
    // only finished() also proves disposal of its native operation context.
    (void)retirement->take();
    if(!retirement->finished())return;
    if(auto error=retirement->error())std::rethrow_exception(error);
    // Keep each control turn bounded even after Stop completed. A full native
    // ingress queue must not starve the other lane or its 250 ms heartbeat.
    if(retiring_reader->queue.pending_frames())return;
    if(!retirement_queue_ended)drain(1);
    peak_queued_frames=std::max(peak_queued_frames,retiring_reader->queue.peak());
    previous_alignment.merge(retiring_reader->alignment_diagnostics());
    retiring_reader.reset();retirement.reset();
    if(raw&&input_retry.failures)raw->drain();
    if(wav&&input_retry.failures)wav->drain_converter();
  }
  void begin_file(){
    file.value=CreateFileW(path.c_str(),GENERIC_WRITE,FILE_SHARE_READ,nullptr,CREATE_NEW,FILE_ATTRIBUTE_NORMAL,nullptr);if(file.value==INVALID_HANDLE_VALUE)throw std::runtime_error("capture destination already exists");
    std::vector<BYTE> header={'c','a','f','f',0,1,0,0,'d','e','s','c'};
    auto be=[&](std::uint64_t value,int width){for(int i=width-1;i>=0;--i)header.push_back(BYTE(value>>(8*i)));};
    be(32,8);double rate=48000;std::uint64_t bits;std::memcpy(&bits,&rate,8);be(bits,8);header.insert(header.end(),{'l','p','c','m'});for(auto value:{3u,channels*4,1u,channels,32u})be(value,4);header.insert(header.end(),{'d','a','t','a'});be(~std::uint64_t(0),8);be(0,4);write_all(file.value,header.data(),header.size());
  }
  void append(const float* samples,UINT32 count,std::uint64_t raw,std::uint64_t position,DWORD flags,std::uint64_t arrival,std::uint64_t storage_end=0){
    if(holding_for_window)return;
    const bool silent=(flags&AUDCLNT_BUFFERFLAGS_SILENT)!=0;
    require_input_pcm(samples,count,channels,silent);
    discontinuities+=(flags&AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY)!=0;
    auto previous=recent.empty()?Json(nullptr):recent.back()["rawQpc100ns"];
    auto previous_position=recent.empty()?Json(nullptr):recent.back()["devicePositionFrames"];
    auto previous_end=recent.empty()?Json(nullptr):recent.back()["mappedEnd100ns"];
    const auto packet=clock.observe(count,raw,position,flags,arrival);
    auto time=packet.start;const auto original_count=count;UINT32 skipped=0;
    if(window_end){
      if(wav){
        if(packet.start>=window_end||packet.end<=window_start)count=0;
        else{
          const auto width=packet.end-packet.start;
          const auto skip=packet.start<window_start?((window_start-packet.start)*original_count+width-1)/width:0;
          const auto after=packet.end>window_end?(window_end-packet.start)*original_count/width:original_count;
          skipped=UINT32(skip);count=after>skip?UINT32(after-skip):0;
          time=packet.start+width*skip/original_count;
          if(!silent)samples+=skip*channels;
        }
      }
      else if(time>=window_end)count=0;
      else {
        if(time<window_start){auto skip=std::min<std::uint64_t>(count,((window_start-time)*48000+9999999)/10000000);time+=skip*10000000/48000;count-=UINT32(skip);skipped=UINT32(skip);if(!silent)samples+=skip*channels;}
        count=UINT32(std::min<std::uint64_t>(count,(window_end-time)*48000/10000000));
      }
    }
    Json entry={{"packet",++observed},{"streamEpoch",clock.stream_epoch},{"rawQpc100ns",raw},{"previousRawQpc100ns",previous},{"devicePositionFrames",position},{"previousDevicePositionFrames",previous_position},{"previousMappedEnd100ns",previous_end},{"flags",flags},{"arrivalQpc100ns",arrival},{"packetFrames",original_count},{"mappedStart100ns",packet.start},{"mappedEnd100ns",packet.end},{"source",packet.source},{"reason",packet.reason},{"selectedFrames",count},{"skippedFrames",skipped}};
    if(recent.size()==32)recent.erase(recent.begin());recent.push_back(entry);
    if(std::string_view(packet.reason)!="none"&&anomalies.size()<16)anomalies.push_back(entry);
    if(!count)return;if(!first)first=time;if(!this->raw&&!wav&&file.value==INVALID_HANDLE_VALUE)begin_file();
    std::vector<float> zeros;if(silent){zeros.assign(std::size_t(count)*channels,0);samples=zeros.data();}
    double sum=0,maximum=0;for(std::size_t i=0;i<std::size_t(count)*channels;++i){if(!std::isfinite(samples[i])||std::abs(samples[i])>16)throw std::runtime_error("invalid captured PCM amplitude");sum+=double(samples[i])*samples[i];maximum=std::max(maximum,double(std::abs(samples[i])));}
    const auto previous_maps=clock.parts.size();
    const auto selected_end=packet.start+(packet.end-packet.start)*(std::uint64_t(skipped)+count)/original_count;
    clock.append(frames,count,time,packet.boundary,wav?selected_end:0);
    if(this->raw)this->raw->append(samples,count,clock.parts.back().last_end-std::uint64_t(count)*10000000/48000,packet.boundary||clock.parts.size()!=previous_maps||clock.stream_epoch!=converted_epoch,storage_end);
    else if(wav)wav->append(samples,count,time,selected_end,packet.boundary||clock.parts.size()!=previous_maps||clock.stream_epoch!=converted_epoch,storage_end);
    converted_epoch=clock.stream_epoch;
    if(!this->raw&&!wav)write_all(file.value,samples,std::size_t(count)*channels*4);frames+=count;++packets;rms=std::sqrt(sum/(double(count)*channels));peak=maximum;window_frames=count;measured=qpc();
    if(started)mon_capture_input_valid(&input_retry,GetTickCount64()*1000000ULL);
  }
  void pump(std::uint64_t budget=4800){auto* source=reader?reader.get():retiring_reader.get();if(!source)return;std::uint64_t frames_written=0;while(frames_written<budget){auto packet=source->queue.pop();if(!packet)break;if(packet->sequence!=admitted_sequence+1)throw InputDataFault("capture ingress order changed");admitted_sequence=packet->sequence;append(packet->samples.data(),packet->frames,packet->time,packet->position,packet->flags,packet->arrival,packet->storage_end);frames_written+=packet->frames;}}
  bool queued(){return (reader&&reader->queue.pending_frames()!=0)||(retiring_reader&&retiring_reader->queue.pending_frames()!=0);}
  std::optional<std::uint64_t> idle_frontier(std::uint64_t no_source_through){
    auto* source=reader?reader.get():retiring_reader.get();
    if(source)return source->queue.observed_through(admitted_sequence);
    // An uncancelled preparation may already hold PCM which will be admitted
    // on adoption. A canceled result is never adopted; no producer remains.
    if(preparation&&!preparation_schedule.cancelled)return std::nullopt;
    return no_source_through;
  }
  Json diagnostics(){return Json{{"observedPackets",observed},{"savedFrames",frames},{"discontinuityPackets",discontinuities},{"acquisitionQueuePeakFrames",std::max(peak_queued_frames,reader?reader->queue.peak():0ULL)},{"acquisitionQueueLimitFrames",PacketQueue::max_frames},{"equalTimestampPackets",clock.equal_timestamps},{"reversedTimestampPackets",clock.reversed_timestamps},{"timestampErrorPackets",clock.timestamp_errors},{"devicePositionFallbackPackets",clock.device_fallbacks},{"sampleCountFallbackPackets",clock.frame_fallbacks},{"estimatedPackets",clock.estimated_packets},{"timingEstimated",clock.estimated_packets!=0},{"firstAnomalies",anomalies},{"recentPackets",recent}};}
  void close(){cancel_preparation();
    const auto deadline=GetTickCount64()+PreparationSchedule::timeout_ms;
    while(retirement){
      poll_retirement();if(!retirement)break;
      if(retirement_wait_expired||GetTickCount64()>=deadline){
        retirement_wait_expired=true;
        try{while(retiring_reader->queue.pending_frames())pump(std::numeric_limits<std::uint64_t>::max());}
        catch(const InputFailure&){}catch(const InputDataFault&){if(!is_recording())throw;}
        throw std::runtime_error("input retirement is still pending; recording closure is unproven");
      }
      Sleep(1);
    }
    if(reader){reader->stop();peak_queued_frames=std::max(peak_queued_frames,reader->queue.peak());}if(started){try{pump(std::numeric_limits<std::uint64_t>::max());}catch(const InputFailure&){}catch(const InputDataFault&){if(!is_recording())throw;}started=false;}if(raw)raw->drain();if(wav)wav->finish();if(file.value!=INVALID_HANDLE_VALUE){auto length=frames*channels*4+4;BYTE size[8];for(int i=0;i<8;++i)size[i]=BYTE(length>>(56-i*8));LARGE_INTEGER offset;offset.QuadPart=56;if(!SetFilePointerEx(file.value,offset,nullptr,FILE_BEGIN))throw std::runtime_error("CAF finalization seek failed");write_all(file.value,size,8);if(!capture_flush(file.value,"caf_finalize_sync"))throw std::runtime_error("CAF finalization flush failed");CloseHandle(file.value);file.value=INVALID_HANDLE_VALUE;}}
  bool acquisition_closed() const{return (!preparation||preparation->finished())&&(!retirement||retirement->finished());}
  Json track(std::uint64_t origin){Json segments=Json::array();for(auto& p:clock.parts)segments.push_back(Json{{"sourceFrameStart",p.first},{"sourceFrameEnd",p.first+p.frames},{"timelineStartUs",(p.start-origin)/10},{"timelineEndUs",(p.last_end-origin)/10},{"mappingJson","{\"schema_version\":1,\"method\":\"linear\"}"}});if(raw)segments=raw->timeline(origin);if(wav)segments=wav->timeline(origin);const auto stored=raw?raw->stored_frames():wav?wav->frames:frames;return Json{{"frames",stored},{"sampleRate",stored?Json(raw||wav?16000:48000):Json(nullptr)},{"channels",stored?Json(raw||wav?1:channels):Json(nullptr)},{"appendFailures",0},{"callbackCount",packets},{"clockSegments",segments},{"clockBoundaryAdjustments",clock.boundary_adjustments},{"maxClockBoundaryAdjustmentNs",clock.max_boundary_adjustment_ticks*100},{"auxiliaryClockFailure",wav&&!wav->clock_error().empty()?Json(wav->clock_error()):Json(nullptr)}};}
  Json level(){if(!window_frames)return Json{{"state","waiting"},{"windowFrames",nullptr},{"ageMs",nullptr},{"rms",nullptr},{"peak",nullptr}};return Json{{"state","measured"},{"windowFrames",window_frames},{"ageMs",(qpc()-measured)/10000},{"rms",rms},{"peak",peak}};}
};
#include "capture_options.h"
#include "capture_session.h"
int run(int argc,wchar_t** argv){
  check(CoInitializeEx(nullptr,COINIT_MULTITHREADED),"initialize product capture COM");
  if(argc==2&&std::wstring(argv[1])==L"--list-input-devices"){std::cout<<microphones().dump()<<std::endl;return 0;}
  if(argc==2&&std::wstring(argv[1])==L"--inspect-audio-state"){std::cout<<audio_inspection::snapshot().dump()<<std::endl;return 0;}
  if(argc==2&&std::wstring(argv[1])==L"--list-capture-applications"){std::cout<<applications().dump()<<std::endl;return 0;}
  return CaptureSession(parse(argc,argv)).run();
}
} // namespace product
