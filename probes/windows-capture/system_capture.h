#pragma once
// Included inside namespace product after CaptureApartment and CaptureReader.
// Teams desktop render streams can be silent in process loopback, including
// EXCLUDE_TARGET_PROCESS_TREE. System scope must read the render endpoints.
// Application scope continues to use process loopback, without broadening it.
class SystemCapture final : public CaptureReader {
  struct Stream {
    ComPtr<IAudioClient> client;
    ComPtr<IAudioCaptureClient> capture;
    PacketClock clock;
    std::optional<std::uint64_t> next_frame;
    bool started=false;
    ~Stream(){if(started)client->Stop();}
  };
  std::atomic<bool> stopping_{false};
  std::thread thread_;
  std::mutex startup_mutex_;
  std::condition_variable startup_changed_;
  bool startup_finished_=false;
  std::exception_ptr startup_error_;
  void startup(std::exception_ptr error=nullptr){std::lock_guard lock(startup_mutex_);if(!startup_finished_){startup_error_=error;startup_finished_=true;startup_changed_.notify_all();}}
  static std::map<std::wstring,ComPtr<IMMDevice>> endpoints(IMMDeviceEnumerator* e){
    ComPtr<IMMDeviceCollection> devices;acquisition(e->EnumAudioEndpoints(eRender,DEVICE_STATE_ACTIVE,&devices),"enumerate PC audio outputs");
    UINT count=0;acquisition(devices->GetCount(&count),"count PC audio outputs");
    if(count>128)throw std::runtime_error("too many PC audio output devices");
    std::map<std::wstring,ComPtr<IMMDevice>> result;
    for(UINT i=0;i<count;++i){ComPtr<IMMDevice> device;acquisition(devices->Item(i,&device),"read PC audio output");result.emplace(endpoint_id(device.Get()),std::move(device));}
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
  static void drain(Stream& stream,EndpointMixer& mixer){
    UINT32 available=0;acquisition(stream.capture->GetNextPacketSize(&available),"poll PC audio output packets");
    for(unsigned drained=0;available&&drained<256;++drained){
      BYTE* data=nullptr;UINT32 frames=0;DWORD flags=0;UINT64 position=0,time=0;
      acquisition(stream.capture->GetBuffer(&data,&frames,&flags,&position,&time),"read PC audio output packet");
      try{
        if(frames){
          auto arrival=qpc();const auto mapped=stream.clock.observe(frames,time,position,flags,arrival);
          auto first=EndpointMixer::frame_at(mapped.start);
          // QPC rounding jitter must not double a boundary sample from the
          // same endpoint. Real gaps larger than one frame remain gaps.
          if(stream.next_frame&&(first>*stream.next_frame?first-*stream.next_frame:*stream.next_frame-first)<=1)first=*stream.next_frame;
          stream.next_frame=first+frames;
          if(mapped.boundary)flags|=AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY;
          if(stream.clock.estimated_packets)flags|=AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR;
          mixer.add(first,frames,reinterpret_cast<const float*>(data),flags);
        }
      }catch(...){stream.capture->ReleaseBuffer(frames);throw;}
      acquisition(stream.capture->ReleaseBuffer(frames),"release PC audio output packet");
      acquisition(stream.capture->GetNextPacketSize(&available),"poll next PC audio output packet");
    }
  }
 public:
  SystemCapture(){
    thread_=std::thread([this]{
      try{
        CaptureApartment apartment;
        auto e=enumerator();auto devices=endpoints(e.Get());
        if(devices.empty())throw InputFailure(AUDCLNT_E_DEVICE_INVALIDATED,"PC audio output unavailable");
        std::map<std::wstring,std::unique_ptr<Stream>> streams;
        for(auto& [id,device]:devices)streams.emplace(id,open(device.Get()));
        for(auto& [id,stream]:streams){acquisition(stream->client->Start(),"start PC audio output loopback");stream->started=true;}
        std::cerr<<"mon_capture_system: source=render_endpoints count="<<streams.size()<<std::endl;
        DWORD index=0;auto priority_handle=AvSetMmThreadCharacteristicsW(L"Audio",&index);
        // AVRT handles require AvRevertMmThreadCharacteristics, not CloseHandle.
        struct Priority {HANDLE value;~Priority(){if(value)AvRevertMmThreadCharacteristics(value);}} priority_owner{priority_handle};
        if(priority_handle)AvSetMmThreadPriority(priority_handle,AVRT_PRIORITY_HIGH);
        EndpointMixer mixer;startup();auto next_check=GetTickCount64()+1000;
        auto emit=[&](CapturePacket packet){queue.push(std::move(packet));};
        try{
          while(!stopping_.load(std::memory_order_acquire)){
            for(auto& [id,stream]:streams)drain(*stream,mixer);
            const auto now=qpc(),frame=EndpointMixer::frame_at(now);
            mixer.flush(frame>EndpointMixer::latency_frames?frame-EndpointMixer::latency_frames:0,now,emit);
            if(GetTickCount64()>=next_check){
              next_check=GetTickCount64()+1000;auto current=endpoints(e.Get());
              bool changed=current.size()!=streams.size();
              for(auto& [id,device]:current)changed|=!streams.count(id);
              // Reopen the owned set through the existing lane lifecycle so
              // output-device changes remain visible as capture interruptions.
              if(changed)throw InputFailure(AUDCLNT_E_DEVICE_INVALIDATED,"PC audio output devices changed");
            }
            Sleep(1);
          }
          for(auto& [id,stream]:streams){acquisition(stream->client->Stop(),"stop PC audio output loopback");stream->started=false;drain(*stream,mixer);}
        }catch(...){
          auto error=std::current_exception();
          for(auto& [id,stream]:streams){
            if(stream->started){stream->client->Stop();stream->started=false;}
            try{drain(*stream,mixer);}catch(const InputFailure&){}
          }
          mixer.flush(std::numeric_limits<std::uint64_t>::max(),qpc(),emit);std::rethrow_exception(error);
        }
        mixer.flush(std::numeric_limits<std::uint64_t>::max(),qpc(),emit);
      }catch(...){auto error=std::current_exception();queue.fail(error);startup(error);}
    });
    std::unique_lock lock(startup_mutex_);startup_changed_.wait(lock,[&]{return startup_finished_;});
    if(startup_error_){auto error=startup_error_;lock.unlock();if(thread_.joinable())thread_.join();std::rethrow_exception(error);}
  }
  void stop() override{stopping_.store(true,std::memory_order_release);if(thread_.joinable())thread_.join();}
  ~SystemCapture() override{stop();}
};
