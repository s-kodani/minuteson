#pragma once
// Included inside namespace product after CaptureApartment and CaptureReader.
// Teams desktop render streams can be silent in process loopback, including
// EXCLUDE_TARGET_PROCESS_TREE. System scope must read the render endpoints.
// Application scope continues to use process loopback, without broadening it.
#include "render_endpoints.h"
class RenderChangeSignal final : public Microsoft::WRL::RuntimeClass<
  Microsoft::WRL::RuntimeClassFlags<Microsoft::WRL::ClassicCom>,IMMNotificationClient,Microsoft::WRL::FtmBase> {
 public:
  std::atomic<bool> changed{false};
  HRESULT STDMETHODCALLTYPE OnDefaultDeviceChanged(EDataFlow flow,ERole,LPCWSTR) override{if(flow==eRender)changed.store(true);return S_OK;}
  HRESULT STDMETHODCALLTYPE OnDeviceAdded(LPCWSTR) override{changed.store(true);return S_OK;}
  HRESULT STDMETHODCALLTYPE OnDeviceRemoved(LPCWSTR) override{changed.store(true);return S_OK;}
  HRESULT STDMETHODCALLTYPE OnDeviceStateChanged(LPCWSTR,DWORD) override{changed.store(true);return S_OK;}
  HRESULT STDMETHODCALLTYPE OnPropertyValueChanged(LPCWSTR,const PROPERTYKEY) override{changed.store(true);return S_OK;}
};
class SystemCapture final : public CaptureReader {
  struct Stream {
    ComPtr<IAudioClient> client;
    ComPtr<IAudioCaptureClient> capture;
    PacketClock clock;
    EndpointCursor cursor;
    bool started=false;
    ~Stream(){if(started)client->Stop();}
  };
  std::atomic<bool> stopping_{false};
  std::thread thread_;
  std::mutex startup_mutex_;
  std::condition_variable startup_changed_;
  bool startup_finished_=false;
  std::exception_ptr startup_error_;
  std::mutex alignment_mutex_;
  AlignmentDiagnostics alignment_;
  void observe_alignment(const EndpointMixer& mixer){std::lock_guard lock(alignment_mutex_);alignment_=mixer.diagnostics();}
  void startup(std::exception_ptr error=nullptr){std::lock_guard lock(startup_mutex_);if(!startup_finished_){startup_error_=error;startup_finished_=true;startup_changed_.notify_all();}}
  static std::optional<bool> playing(IMMDevice* device){
    ComPtr<IAudioSessionManager2> manager;ComPtr<IAudioSessionEnumerator> sessions;
    if(FAILED(device->Activate(__uuidof(IAudioSessionManager2),CLSCTX_ALL,nullptr,reinterpret_cast<void**>(manager.GetAddressOf())))||FAILED(manager->GetSessionEnumerator(&sessions)))return std::nullopt;
    int count=0;if(FAILED(sessions->GetCount(&count))||count<0||count>4096)return std::nullopt;
    bool unknown=false;
    for(int i=0;i<count;++i){
      ComPtr<IAudioSessionControl> session;ComPtr<IAudioSessionControl2> control;AudioSessionState state;DWORD pid=0;
      if(FAILED(sessions->GetSession(i,&session))||FAILED(session->GetState(&state))){unknown=true;continue;}
      if(state!=AudioSessionStateActive)continue;
      if(FAILED(session.As(&control))){unknown=true;continue;}
      const auto identity=control->GetProcessId(&pid);
      if(FAILED(identity)){unknown=true;continue;}
      if(render_session_uses_output(true,pid==GetCurrentProcessId(),identity!=S_OK))return true;
    }
    return unknown?std::nullopt:std::optional<bool>(false);
  }
  static std::map<std::wstring,ComPtr<IMMDevice>> endpoints(IMMDeviceEnumerator* e,const std::map<std::wstring,std::unique_ptr<Stream>>& streams){
    ComPtr<IMMDeviceCollection> devices;acquisition(e->EnumAudioEndpoints(eRender,DEVICE_STATE_ACTIVE,&devices),"enumerate PC audio outputs");
    UINT count=0;acquisition(devices->GetCount(&count),"count PC audio outputs");
    if(count>128)throw std::runtime_error("too many PC audio output devices");
    std::set<std::wstring> defaults;
    for(auto role:{eConsole,eMultimedia}){ComPtr<IMMDevice> device;if(SUCCEEDED(e->GetDefaultAudioEndpoint(eRender,role,&device)))defaults.insert(endpoint_id(device.Get()));}
    std::map<std::wstring,ComPtr<IMMDevice>> result;
    for(UINT i=0;i<count;++i){
      ComPtr<IMMDevice> device;acquisition(devices->Item(i,&device),"read PC audio output");const auto id=endpoint_id(device.Get());
      if(needs_render_loopback(defaults.count(id)!=0,playing(device.Get()),streams.count(id)!=0))result.emplace(id,std::move(device));
    }
    return result;
  }
  static std::unique_ptr<Stream> open(IMMDevice* device){
    auto stream=std::make_unique<Stream>();
    acquisition(device->Activate(__uuidof(IAudioClient),CLSCTX_ALL,nullptr,reinterpret_cast<void**>(stream->client.GetAddressOf())),"activate PC audio output");
    WAVEFORMATEX* mix=nullptr;
    const bool matching=SUCCEEDED(stream->client->GetMixFormat(&mix))&&mix&&mix->nSamplesPerSec==48000;
    if(mix)CoTaskMemFree(mix);stream->clock.begin_stream(matching);
    WAVEFORMATEX format{};format.wFormatTag=WAVE_FORMAT_IEEE_FLOAT;format.nChannels=2;format.nSamplesPerSec=48000;
    format.wBitsPerSample=32;format.nBlockAlign=8;format.nAvgBytesPerSec=384000;
    acquisition(stream->client->Initialize(AUDCLNT_SHAREMODE_SHARED,AUDCLNT_STREAMFLAGS_LOOPBACK|AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM|AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,5000000,0,&format,nullptr),"initialize PC audio output loopback");
    acquisition(stream->client->GetService(IID_PPV_ARGS(stream->capture.GetAddressOf())),"get PC audio output capture service");
    return stream;
  }
  bool drain(Stream& stream,EndpointMixer& mixer,unsigned budget=1){
    UINT32 available=0;acquisition(stream.capture->GetNextPacketSize(&available),"poll PC audio output packets");
    for(unsigned drained=0;available&&drained<budget;++drained){
      BYTE* data=nullptr;UINT32 frames=0;DWORD flags=0;UINT64 position=0,time=0;
      acquisition(stream.capture->GetBuffer(&data,&frames,&flags,&position,&time),"read PC audio output packet");
      try{
        if(frames){
          require_input_pcm(reinterpret_cast<const float*>(data),frames,2,(flags&AUDCLNT_BUFFERFLAGS_SILENT)!=0);
          auto arrival=qpc();const auto mapped=stream.clock.observe(frames,time,position,flags,arrival);
          const auto first=EndpointMixer::frame_at(mapped.start);
          if(mapped.boundary)flags|=AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY;
          if(stream.clock.estimated_packets)flags|=AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR;
          const auto result=mixer.add(stream.cursor,first,frames,reinterpret_cast<const float*>(data),flags,arrival,
            [&](CapturePacket packet){queue.push(std::move(packet));});
          if(result.adjusted||result.pressure)observe_alignment(mixer);
        }
      }catch(...){stream.capture->ReleaseBuffer(frames);throw;}
      acquisition(stream.capture->ReleaseBuffer(frames),"release PC audio output packet");
      acquisition(stream.capture->GetNextPacketSize(&available),"poll next PC audio output packet");
    }
    return available!=0;
  }
 public:
  SystemCapture(){
    thread_=std::thread([this]{
      try{
        CaptureApartment apartment;
        auto e=enumerator();
        auto changes=Microsoft::WRL::Make<RenderChangeSignal>();
        const bool registered=SUCCEEDED(e->RegisterEndpointNotificationCallback(changes.Get()));
        struct Notifications {IMMDeviceEnumerator* e;RenderChangeSignal* signal;bool registered;~Notifications(){if(registered)e->UnregisterEndpointNotificationCallback(signal);}} notifications{e.Get(),changes.Get(),registered};
        std::map<std::wstring,std::unique_ptr<Stream>> streams;
        auto devices=endpoints(e.Get(),streams);
        if(devices.empty())throw InputFailure(AUDCLNT_E_DEVICE_INVALIDATED,"PC audio output unavailable");
        for(auto& [id,device]:devices)streams.emplace(id,open(device.Get()));
        for(auto& [id,stream]:streams){acquisition(stream->client->Start(),"start PC audio output loopback");stream->started=true;}
        std::cerr<<"mon_capture_system: source=render_endpoints count="<<streams.size()<<std::endl;
        DWORD index=0;auto priority_handle=AvSetMmThreadCharacteristicsW(L"Audio",&index);
        // AVRT handles require AvRevertMmThreadCharacteristics, not CloseHandle.
        struct Priority {HANDLE value;~Priority(){if(value)AvRevertMmThreadCharacteristics(value);}} priority_owner{priority_handle};
        if(priority_handle)AvSetMmThreadPriority(priority_handle,AVRT_PRIORITY_HIGH);
        EndpointMixer mixer;startup();auto next_check=GetTickCount64()+100;
        auto emit=[&](CapturePacket packet){queue.push(std::move(packet));};
        try{
          while(!stopping_.load(std::memory_order_acquire)){
            if(changes->changed.exchange(false)||GetTickCount64()>=next_check){
              next_check=GetTickCount64()+100;auto current=endpoints(e.Get(),streams);
              if(current.empty())throw InputFailure(AUDCLNT_E_DEVICE_INVALIDATED,"PC audio output unavailable");
              reconcile_render_streams(streams,current,[&](Stream& stream){
                if(stream.started){stream.client->Stop();stream.started=false;}
                try{drain(stream,mixer,256);}catch(const InputFailure&){}
              },[&](const ComPtr<IMMDevice>& device){auto stream=open(device.Get());acquisition(stream->client->Start(),"start changed PC audio output loopback");stream->started=true;return stream;});
            }
            bool backlog=false;
            for(auto& [id,stream]:streams)backlog|=drain(*stream,mixer);
            const auto now=qpc(),frame=EndpointMixer::frame_at(now);
            auto through=frame>EndpointMixer::latency_frames?frame-EndpointMixer::latency_frames:0;
            for(auto& [id,stream]:streams)through=stream->cursor.safe_through(now,through);
            mixer.flush(through,now,emit);
            // Round-robin packets keep endpoints fair, but sleeping with a
            // backlog can make a 10 ms stream fall behind the Windows timer.
            if(!backlog)Sleep(1);
          }
          for(auto& [id,stream]:streams){acquisition(stream->client->Stop(),"stop PC audio output loopback");stream->started=false;drain(*stream,mixer,256);}
        }catch(...){
          auto error=std::current_exception();
          for(auto& [id,stream]:streams){
            if(stream->started){stream->client->Stop();stream->started=false;}
            try{drain(*stream,mixer,256);}catch(const InputFailure&){}catch(const InputDataFault&){}catch(...){error=std::current_exception();}
          }
          try{
            mixer.flush(std::numeric_limits<std::uint64_t>::max(),qpc(),emit);
          }catch(const InputFailure&){}catch(...){error=std::current_exception();}
          // A device resync must never hide invalid PCM or storage failures.
          std::rethrow_exception(error);
        }
        mixer.flush(std::numeric_limits<std::uint64_t>::max(),qpc(),emit);
      }catch(...){auto error=std::current_exception();queue.fail(error);startup(error);}
    });
    std::unique_lock lock(startup_mutex_);startup_changed_.wait(lock,[&]{return startup_finished_;});
    if(startup_error_){auto error=startup_error_;lock.unlock();if(thread_.joinable())thread_.join();std::rethrow_exception(error);}
  }
  void stop() override{stopping_.store(true,std::memory_order_release);if(thread_.joinable())thread_.join();}
  AlignmentDiagnostics alignment_diagnostics() override{std::lock_guard lock(alignment_mutex_);return alignment_;}
  ~SystemCapture() override{stop();}
};
