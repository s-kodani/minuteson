#pragma once
// Included inside namespace product after CaptureApartment and CaptureReader.
// Teams desktop render streams can be silent in process loopback, including
// EXCLUDE_TARGET_PROCESS_TREE. System scope must read the render endpoints.
// Application scope continues to use process loopback, without broadening it.
#include "render_endpoints.h"
class SystemCapture final : public CaptureReader {
  struct Stream {
    std::unique_ptr<AsyncCapture> reader;
    PacketClock clock;
    EndpointCursor cursor;
    std::uint64_t admitted_sequence=0;
    ~Stream(){if(reader)reader->stop();}
  };
  using RetirementOperation=InputPreparation<bool,CaptureApartment>;
  struct Retirement {
    std::unique_ptr<Stream> stream;
    std::unique_ptr<RetirementOperation> operation;
    bool queue_ended=false;
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
  template<class Known>
  static std::map<std::wstring,ComPtr<IMMDevice>> endpoints(IMMDeviceEnumerator* e,const Known& streams){
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
  static std::unique_ptr<Stream> open(IMMDevice* device,std::shared_ptr<PacketIngressBudget> ingress){
    auto stream=std::make_unique<Stream>();
    ComPtr<IAudioClient> client;
    acquisition(device->Activate(__uuidof(IAudioClient),CLSCTX_ALL,nullptr,reinterpret_cast<void**>(client.GetAddressOf())),"activate PC audio output");
    WAVEFORMATEX* mix=nullptr;
    const bool matching=SUCCEEDED(client->GetMixFormat(&mix))&&mix&&mix->nSamplesPerSec==48000;
    if(mix)CoTaskMemFree(mix);stream->clock.begin_stream(matching);
    WAVEFORMATEX format{};format.wFormatTag=WAVE_FORMAT_IEEE_FLOAT;format.nChannels=2;format.nSamplesPerSec=48000;
    format.wBitsPerSample=32;format.nBlockAlign=8;format.nAvgBytesPerSec=384000;
    acquisition(client->Initialize(AUDCLNT_SHAREMODE_SHARED,AUDCLNT_STREAMFLAGS_LOOPBACK|AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM|AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,5000000,0,&format,nullptr),"initialize PC audio output loopback");
    // Every endpoint keeps GetService, buffer calls, Stop and native release
    // on its own owner. One Bluetooth driver cannot stall a healthy peer.
    stream->reader=std::make_unique<AsyncCapture>(std::move(client),2,false,std::string{},nullptr,std::move(ingress));
    return stream;
  }
  bool drain(Stream& stream,EndpointMixer& mixer,unsigned budget=1){
    for(unsigned drained=0;drained<budget;++drained){
      auto packet=stream.reader->queue.pop();if(!packet)break;
      if(packet->sequence!=stream.admitted_sequence+1)throw InputDataFault("endpoint ingress order changed");
      stream.admitted_sequence=packet->sequence;
      auto flags=packet->flags;const auto mapped=stream.clock.observe(packet->frames,packet->time,packet->position,flags,packet->arrival);
      const auto first=EndpointMixer::frame_at(mapped.start);
      if(mapped.boundary)flags|=AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY;
      if(stream.clock.estimated_packets)flags|=AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR;
      const auto result=mixer.add(stream.cursor,first,packet->frames,packet->samples.data(),flags,packet->arrival,
        [&](CapturePacket mixed){queue.push_wait(std::move(mixed));});
      if(result.adjusted||result.pressure)observe_alignment(mixer);
    }
    return stream.reader->queue.pending_frames()!=0;
  }
 public:
  explicit SystemCapture(std::function<ComPtr<IMMDeviceEnumerator>()> device_enumerator=enumerator){
    thread_=std::thread([this,device_enumerator=std::move(device_enumerator)]{
      try{
        CaptureApartment apartment;
        auto e=device_enumerator();
        EndpointLifetime lifetime(e);auto changes=lifetime.changes;
        std::map<std::wstring,std::unique_ptr<Stream>> streams;
        auto devices=endpoints(e.Get(),streams);
        if(devices.empty())throw InputFailure(AUDCLNT_E_DEVICE_INVALIDATED,"PC audio output unavailable");
        auto ingress=std::make_shared<PacketIngressBudget>();
        DWORD index=0;auto priority_handle=AvSetMmThreadCharacteristicsW(L"Audio",&index);
        // AVRT handles require AvRevertMmThreadCharacteristics, not CloseHandle.
        struct Priority {HANDLE value;~Priority(){if(value)AvRevertMmThreadCharacteristics(value);}} priority_owner{priority_handle};
        if(priority_handle)AvSetMmThreadPriority(priority_handle,AVRT_PRIORITY_HIGH);
        EndpointMixer mixer(true);auto next_check=GetTickCount64()+100;bool announced=false;
        using Devices=std::map<std::wstring,ComPtr<IMMDevice>>;
        using Enumeration=InputPreparation<Devices,CaptureApartment>;
        using Opening=InputPreparation<std::unique_ptr<Stream>,CaptureApartment>;
        std::unique_ptr<Enumeration> enumeration;
        std::map<std::wstring,std::unique_ptr<Opening>> openings;
        std::map<std::wstring,Retirement> retirements;
        std::map<std::wstring,MonCaptureInputRetry> retries;
        Devices desired=devices;
        auto emit=[&](CapturePacket packet){queue.push_wait(std::move(packet));};
        auto failed=[&](const std::wstring& id,const InputFailure& error){
          mon_capture_input_fault(&retries[id],GetTickCount64()*1000000ULL);
          std::cerr<<"mon_capture_endpoint: "+Json{{"event","retry"},{"uid",utf8(id)},{"hresult",static_cast<std::int64_t>(error.result)},{"attempt",retries[id].failures}}.dump()+"\n";
        };
        auto retire=[&](auto iterator){
          auto id=iterator->first;Retirement retiring;retiring.stream=std::move(iterator->second);streams.erase(iterator);
          auto* source=retiring.stream->reader.get();
          retiring.operation=std::make_unique<RetirementOperation>([source](auto){source->stop();return true;});
          retirements.emplace(std::move(id),std::move(retiring));
        };
        std::exception_ptr failure;
        auto recoverable=[](std::exception_ptr error){
          try{std::rethrow_exception(error);}
          catch(const InputFailure&){return true;}
          catch(const InputDataFault&){return true;}
          catch(...){return false;}
        };
        auto remember_failure=[&](std::exception_ptr error){
          // A storage/capacity failure cannot become an input retry just
          // because a disconnected endpoint failed earlier during shutdown.
          if(error&&(!failure||(recoverable(failure)&&!recoverable(error))))failure=error;
        };
        auto poll_retirements=[&](bool closing){
          bool backlog=false;
          for(auto it=retirements.begin();it!=retirements.end();){
            auto& retiring=it->second;
            auto drain_owned=[&]{
              try{backlog|=drain(*retiring.stream,mixer);}
              catch(const InputFailure&){retiring.queue_ended=true;}
              catch(...){if(!closing)throw;remember_failure(std::current_exception());retiring.queue_ended=true;}
            };
            if(!retiring.queue_ended)drain_owned();
            (void)retiring.operation->take();
            if(!retiring.operation->finished()){++it;continue;}
            if(auto error=retiring.operation->error()){if(!closing)std::rethrow_exception(error);remember_failure(error);}
            if(!retiring.queue_ended&&retiring.stream->reader->queue.pending_frames()){++it;continue;}
            // The final accepted packet is followed by a terminal error, not
            // by another frame. Observe it after native closure before erase.
            if(!retiring.queue_ended)drain_owned();
            // stop() joined the native owner, including release of both COM
            // interfaces. Erase its last reader before allowing the same ID.
            it=retirements.erase(it);
          }
          return backlog;
        };
        auto flush_ready=[&](bool observe){
          const auto now=qpc(),frame=EndpointMixer::frame_at(now);
          auto through=frame>EndpointMixer::latency_frames?frame-EndpointMixer::latency_frames:0;
          auto constrain=[&](Stream& stream){
            // After output backpressure, an old arrival time does not grant
            // permission to flush across this endpoint's unread PCM. Drain
            // and flush each round, keeping the bounded mixer below capacity.
            if(stream.reader->queue.pending_frames())through=std::min(through,stream.cursor.next.value_or(0));
            else through=stream.cursor.safe_through(now,through);
          };
          for(auto& [id,stream]:streams)constrain(*stream);
          for(auto& [id,retiring]:retirements)constrain(*retiring.stream);
          mixer.flush(through,now,emit);
          if(observe)queue.observe(EndpointMixer::ticks_at(mixer.observed_through(through)));
        };
        try{
          while(!stopping_.load(std::memory_order_acquire)){
            const auto now_ms=GetTickCount64();
            if(changes->changed.exchange(false)||now_ms>=next_check){
              auto revoked=changes->take();next_check=now_ms+100;
              // Revoke native lifetimes immediately. A preparation remains
              // owned until its cancellation really completes, while other
              // endpoints continue draining their acquisition buffers.
              for(auto& [id,pending]:openings)if(revoked.all||revoked.endpoints.count(id))pending->cancel();
              if(revoked.all)desired.clear();else for(const auto& id:revoked.endpoints)desired.erase(id);
              if(enumeration&&(revoked.all||!revoked.endpoints.empty())){enumeration->cancel();next_check=0;}
              for(auto it=streams.begin();it!=streams.end();){
                if(revoked.all||revoked.endpoints.count(it->first)){
                  auto retired=it++;retire(retired);
                }else ++it;
              }
              if(!enumeration){
                std::set<std::wstring> known;for(const auto& [id,stream]:streams)known.insert(id);
                enumeration=std::make_unique<Enumeration>([known=std::move(known),factory=device_enumerator](auto cancelled){
                  auto devices=factory();auto result=endpoints(devices.Get(),known);
                  if(cancelled())return Devices{};return result;
                });
              }
            }
            if(enumeration){
              if(auto current=enumeration->take()){
                enumeration.reset();desired=std::move(*current);
                if(desired.empty())throw InputFailure(AUDCLNT_E_DEVICE_INVALIDATED,"PC audio output unavailable");
                for(auto& [id,pending]:openings)if(!desired.count(id))pending->cancel();
                for(auto it=streams.begin();it!=streams.end();){
                  if(!desired.count(it->first)){
                    auto retired=it++;retire(retired);
                  }else ++it;
                }
              }else if(enumeration->finished()){
                auto error=enumeration->error();enumeration.reset();
                if(error){try{std::rethrow_exception(error);}catch(const InputFailure&){next_check=now_ms+1000;}}
              }
            }
            for(auto it=openings.begin();it!=openings.end();){
              if(auto stream=it->second->take()){
                streams.emplace(it->first,std::move(*stream));it=openings.erase(it);
              }else if(it->second->finished()){
                auto error=it->second->error();auto id=it->first;it=openings.erase(it);
                if(error){try{std::rethrow_exception(error);}catch(const InputFailure& input){failed(id,input);}}
              }else ++it;
            }
            bool backlog=poll_retirements(false);
            for(const auto& [id,device]:desired)if(!streams.count(id)&&!openings.count(id)&&!retirements.count(id)
                &&now_ms*1000000ULL>=retries[id].retry_at&&streams.size()+openings.size()+retirements.size()<128){
              openings.emplace(id,std::make_unique<Opening>([device,ingress](auto cancelled){
                if(cancelled())return std::unique_ptr<Stream>{};auto stream=open(device.Get(),ingress);
                if(cancelled())return std::unique_ptr<Stream>{};return stream;
              }));
            }
            for(auto it=streams.begin();it!=streams.end();){
              try{
                const auto previous=it->second->cursor.last_arrival;backlog|=drain(*it->second,mixer);
                if(it->second->cursor.last_arrival!=previous)mon_capture_input_valid(&retries[it->first],now_ms*1000000ULL);
                ++it;
              }catch(const InputFailure& error){failed(it->first,error);auto retired=it++;retire(retired);}
            }
            if(!announced&&!streams.empty()){
              announced=true;std::cerr<<"mon_capture_system: source=render_endpoints count="<<streams.size()<<std::endl;startup();
            }
            if(streams.empty()&&openings.empty()&&retirements.empty()&&!enumeration)throw InputFailure(AUDCLNT_E_DEVICE_INVALIDATED,"PC audio outputs are unavailable");
            for(auto it=retries.begin();it!=retries.end();)if(!desired.count(it->first)&&!streams.count(it->first)&&!openings.count(it->first)&&!retirements.count(it->first))it=retries.erase(it);else ++it;
            // An empty observation follows every currently queued endpoint
            // packet. A timer must never overtake a backlog on another peer.
            flush_ready(!backlog);
            // Round-robin packets keep endpoints fair, but sleeping with a
            // backlog can make a 10 ms stream fall behind the Windows timer.
            if(!backlog)Sleep(1);
          }
        }catch(...){remember_failure(std::current_exception());startup(failure);}
        // Cancellation is not native closure. Own every opening/enumeration
        // and retirement through disposal before stop() may return to its Lane.
        if(enumeration)enumeration->cancel();
        for(auto& [id,pending]:openings)pending->cancel();
        while(!streams.empty())retire(streams.begin());
        while(enumeration||!openings.empty()||!retirements.empty()){
          const auto backlog=poll_retirements(true);
          try{flush_ready(false);}catch(...){remember_failure(std::current_exception());}
          if(enumeration&&enumeration->finished())enumeration.reset();
          for(auto it=openings.begin();it!=openings.end();)if(it->second->finished())it=openings.erase(it);else ++it;
          if(!backlog)Sleep(1);
        }
        try{mixer.flush(std::numeric_limits<std::uint64_t>::max(),qpc(),emit);}catch(...){remember_failure(std::current_exception());}
        observe_alignment(mixer);if(failure)std::rethrow_exception(failure);
      }catch(...){auto error=std::current_exception();queue.fail(error);startup(error);}
    });
    std::unique_lock lock(startup_mutex_);startup_changed_.wait(lock,[&]{return startup_finished_;});
    if(startup_error_){auto error=startup_error_;lock.unlock();queue.discard_waiters();if(thread_.joinable())thread_.join();std::rethrow_exception(error);}
  }
  void stop() override{stopping_.store(true,std::memory_order_release);if(thread_.joinable())thread_.join();}
  void discard() override{queue.discard_waiters();stop();}
  AlignmentDiagnostics alignment_diagnostics() override{std::lock_guard lock(alignment_mutex_);return alignment_;}
  ~SystemCapture() override{discard();}
};
