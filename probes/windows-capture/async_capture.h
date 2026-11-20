#pragma once
// Included inside namespace product after InputFailure. Acquisition owns no
// filesystem work. GetBuffer and ReleaseBuffer always run on this one thread.
// A microphone continuously produces packets even for silence. A successful
// Start alone is not proof that a Bluetooth capture engine is delivering data.
struct MicrophoneFlow {
  static constexpr std::uint64_t timeout_ms=3000;
  std::uint64_t last_packet_at=0;
  bool received=false;
  explicit MicrophoneFlow(std::uint64_t now):last_packet_at(now){}
  void packet(std::uint64_t now){last_packet_at=now;received=true;}
  bool expired(std::uint64_t now)const{return now-last_packet_at>=timeout_ms;}
};
class AsyncCapture final : public CaptureReader {
  UINT32 channels_;
  std::atomic<bool> stopping_{false};
  std::thread thread_;
  std::mutex startup_mutex_;
  std::condition_variable startup_changed_;
  bool startup_finished_=false;
  std::exception_ptr startup_error_;
  void startup(std::exception_ptr error=nullptr){std::lock_guard lock(startup_mutex_);if(!startup_finished_){startup_error_=error;startup_finished_=true;startup_changed_.notify_all();}}
  bool drain(IAudioCaptureClient* capture,EndpointLifetime* lifetime,bool closing=false) {
    if(lifetime)lifetime->require_current();
    bool received=false;
    UINT32 available=0;
    acquisition(capture->GetNextPacketSize(&available),"poll WASAPI packets");
    for(UINT32 drained=0;available&&(closing||drained<256);++drained) {
      BYTE* data=nullptr;UINT32 count=0;DWORD flags=0;UINT64 position=0,time=0;
      acquisition(capture->GetBuffer(&data,&count,&flags,&position,&time),"read WASAPI packet");
      try {
        if(lifetime)lifetime->require_current();
        if(count) {
          if(count>PacketQueue::max_frames)throw std::runtime_error("Windows音声パケットが保存待ち上限を超えています");
          CapturePacket packet;packet.frames=count;packet.flags=flags;packet.time=time;packet.position=position;packet.arrival=qpc();
          if(!(flags&AUDCLNT_BUFFERFLAGS_SILENT)) {
            require_input_pcm(reinterpret_cast<const float*>(data),count,channels_);
            const auto samples=reinterpret_cast<const float*>(data);
            packet.samples.assign(samples,samples+std::size_t(count)*channels_);
          }
          else require_input_pcm(nullptr,count,channels_,true);
          queue.push(std::move(packet));
          received=true;
        }
      }catch(...){capture->ReleaseBuffer(count);throw;}
      acquisition(capture->ReleaseBuffer(count),"release WASAPI packet");
      acquisition(capture->GetNextPacketSize(&available),"poll next WASAPI packet");
    }
    if(!available){
      const auto now=qpc(),latency=EndpointMixer::latency_frames*10000000/48000;
      queue.observe(now>latency?now-latency:0);
    }
    return received;
  }
 public:
  AsyncCapture(ComPtr<IAudioClient> client,UINT32 channels,bool microphone=false,std::string uid={},std::shared_ptr<EndpointLifetime> lifetime={}):channels_(channels) {
    thread_=std::thread([this,client=std::move(client),microphone,uid=std::move(uid),lifetime=std::move(lifetime)]() mutable {
      const auto initialized=CoInitializeEx(nullptr,COINIT_MULTITHREADED);
      DWORD index=0;HANDLE priority=nullptr;
      {
        // WASAPI requires releasing IAudioCaptureClient on the GetService
        // thread. Keep its sole service reference, start/stop and all buffer
        // operations inside this scope, before leaving the COM apartment.
        ComPtr<IAudioCaptureClient> capture;bool started=false;
        try {
          acquisition(initialized,"initialize WASAPI acquisition thread");
          if(lifetime)lifetime->require_current();
          acquisition(client->GetService(IID_PPV_ARGS(capture.GetAddressOf())),"get WASAPI capture service");
          acquisition(client->Start(),"音声入力を開始できません");started=true;
          priority=AvSetMmThreadCharacteristicsW(L"Audio",&index);
          if(priority)AvSetMmThreadPriority(priority,AVRT_PRIORITY_HIGH);
          MicrophoneFlow flow(GetTickCount64());
          if(!microphone)startup(); // loopback can legitimately have no playback
          while(!stopping_.load(std::memory_order_acquire)){
            if(drain(capture.Get(),lifetime.get())){flow.packet(GetTickCount64());if(microphone)startup();}
            if(microphone&&flow.expired(GetTickCount64())){
              std::cerr<<"mon_capture_input: "+Json{{"event","no_packets"},{"uid",uid},{"receivedBefore",flow.received},{"elapsedMs",GetTickCount64()-flow.last_packet_at}}.dump()+"\n";
              throw InputFailure(HRESULT_FROM_WIN32(ERROR_TIMEOUT),flow.received?"microphone packet delivery stopped":"microphone Start succeeded without delivering packets");
            }
            Sleep(1);
          }
          acquisition(client->Stop(),"stop WASAPI input");started=false;drain(capture.Get(),lifetime.get(),true);
        }catch(...){auto error=std::current_exception();queue.fail(error);if(started)client->Stop();startup(error);}
      }
      client.Reset();
      lifetime.reset();
      if(priority)AvRevertMmThreadCharacteristics(priority);
      if(SUCCEEDED(initialized))CoUninitialize();
    });
    // This wait runs only on the independently owned preparation worker in
    // production. It cannot stall PCM writing, status publication or stop.
    std::unique_lock lock(startup_mutex_);startup_changed_.wait(lock,[&]{return startup_finished_;});
    if(startup_error_){auto error=startup_error_;lock.unlock();if(thread_.joinable())thread_.join();std::rethrow_exception(error);}
  }
  void stop() override{stopping_.store(true,std::memory_order_release);if(thread_.joinable())thread_.join();}
  ~AsyncCapture() override{stop();}
};
