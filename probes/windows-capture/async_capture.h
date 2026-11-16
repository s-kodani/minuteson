#pragma once
// Included inside namespace product after InputFailure. Acquisition owns no
// filesystem work. GetBuffer and ReleaseBuffer always run on this one thread.
class AsyncCapture final : public CaptureReader {
  UINT32 channels_;
  std::atomic<bool> stopping_{false};
  std::thread thread_;
  std::mutex startup_mutex_;
  std::condition_variable startup_changed_;
  bool startup_finished_=false;
  std::exception_ptr startup_error_;
  void startup(std::exception_ptr error=nullptr){std::lock_guard lock(startup_mutex_);if(!startup_finished_){startup_error_=error;startup_finished_=true;startup_changed_.notify_all();}}
  void drain(IAudioCaptureClient* capture) {
    UINT32 available=0;
    acquisition(capture->GetNextPacketSize(&available),"poll WASAPI packets");
    for(UINT32 drained=0;available&&drained<256;++drained) {
      BYTE* data=nullptr;UINT32 count=0;DWORD flags=0;UINT64 position=0,time=0;
      acquisition(capture->GetBuffer(&data,&count,&flags,&position,&time),"read WASAPI packet");
      try {
        if(count) {
          if(count>PacketQueue::max_frames)throw std::runtime_error("Windows音声パケットが保存待ち上限を超えています");
          CapturePacket packet;packet.frames=count;packet.flags=flags;packet.time=time;packet.position=position;packet.arrival=qpc();
          if(!(flags&AUDCLNT_BUFFERFLAGS_SILENT)) {
            if(!data)throw std::runtime_error("Windows音声パケットのPCMがありません");
            const auto samples=reinterpret_cast<const float*>(data);
            packet.samples.assign(samples,samples+std::size_t(count)*channels_);
          }
          queue.push(std::move(packet));
        }
      }catch(...){capture->ReleaseBuffer(count);throw;}
      acquisition(capture->ReleaseBuffer(count),"release WASAPI packet");
      acquisition(capture->GetNextPacketSize(&available),"poll next WASAPI packet");
    }
  }
 public:
  AsyncCapture(ComPtr<IAudioClient> client,UINT32 channels):channels_(channels) {
    thread_=std::thread([this,client=std::move(client)]() mutable {
      const auto initialized=CoInitializeEx(nullptr,COINIT_MULTITHREADED);
      DWORD index=0;HANDLE priority=nullptr;
      {
        // WASAPI requires releasing IAudioCaptureClient on the GetService
        // thread. Keep its sole service reference, start/stop and all buffer
        // operations inside this scope, before leaving the COM apartment.
        ComPtr<IAudioCaptureClient> capture;bool started=false;
        try {
          acquisition(initialized,"initialize WASAPI acquisition thread");
          acquisition(client->GetService(IID_PPV_ARGS(capture.GetAddressOf())),"get WASAPI capture service");
          acquisition(client->Start(),"音声入力を開始できません");started=true;
          priority=AvSetMmThreadCharacteristicsW(L"Audio",&index);
          if(priority)AvSetMmThreadPriority(priority,AVRT_PRIORITY_HIGH);
          startup();
          while(!stopping_.load(std::memory_order_acquire)){drain(capture.Get());Sleep(1);}
          client->Stop();started=false;drain(capture.Get());
        }catch(...){auto error=std::current_exception();queue.fail(error);if(started)client->Stop();startup(error);}
      }
      client.Reset();
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
