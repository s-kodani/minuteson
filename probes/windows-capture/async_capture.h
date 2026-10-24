#pragma once
// Included inside namespace product after InputFailure. Acquisition owns no
// filesystem work. GetBuffer and ReleaseBuffer always run on this one thread.
class AsyncCapture {
  ComPtr<IAudioClient> client_;
  ComPtr<IAudioCaptureClient> capture_;
  UINT32 channels_;
  std::atomic<bool> stopping_{false};
  std::thread thread_;
  void drain() {
    UINT32 available=0;
    acquisition(capture_->GetNextPacketSize(&available),"poll WASAPI packets");
    for(UINT32 drained=0;available&&drained<256;++drained) {
      BYTE* data=nullptr;UINT32 count=0;DWORD flags=0;UINT64 position=0,time=0;
      acquisition(capture_->GetBuffer(&data,&count,&flags,&position,&time),"read WASAPI packet");
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
      }catch(...){capture_->ReleaseBuffer(count);throw;}
      acquisition(capture_->ReleaseBuffer(count),"release WASAPI packet");
      acquisition(capture_->GetNextPacketSize(&available),"poll next WASAPI packet");
    }
  }
 public:
  PacketQueue queue;
  AsyncCapture(ComPtr<IAudioClient> client,ComPtr<IAudioCaptureClient> capture,UINT32 channels):client_(std::move(client)),capture_(std::move(capture)),channels_(channels) {
    thread_=std::thread([this]{
      const auto initialized=CoInitializeEx(nullptr,COINIT_MULTITHREADED);
      DWORD index=0;HANDLE priority=nullptr;
      try {
        acquisition(initialized,"initialize WASAPI acquisition thread");
        priority=AvSetMmThreadCharacteristicsW(L"Audio",&index);
        if(priority)AvSetMmThreadPriority(priority,AVRT_PRIORITY_HIGH);
        while(!stopping_.load(std::memory_order_acquire)){drain();Sleep(1);}
        client_->Stop();drain();
      }catch(...){queue.fail(std::current_exception());client_->Stop();}
      if(priority)AvRevertMmThreadCharacteristics(priority);
      if(SUCCEEDED(initialized))CoUninitialize();
    });
  }
  void stop(){stopping_.store(true,std::memory_order_release);if(thread_.joinable())thread_.join();}
  ~AsyncCapture(){stop();}
};
