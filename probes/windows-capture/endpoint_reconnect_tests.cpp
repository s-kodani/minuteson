// Run the real native readers against COM devices that reconnect with the
// same ID while their old IAudioClient keeps returning S_OK. No hardware opens.
#define wmain unused_capture_entry
#include "main.cpp"
#undef wmain

void require(bool value,const char* message){if(!value)throw std::runtime_error(message);}
template<class Predicate>void wait_for(Predicate predicate,const char* message){
  const auto deadline=GetTickCount64()+5000;
  while(!predicate()&&GetTickCount64()<deadline)Sleep(1);
  require(predicate(),message);
}
float normal_generation_sample(unsigned generation){return std::ldexp(1.0f,static_cast<int>(generation)-11);}
struct EndpointStats {
  std::atomic<unsigned> opens{0},live{0},released{0},overlaps{0},stops{0},enumerations{0},wrong_thread{0},packets{0};
  std::atomic<unsigned> polls{0};
  std::atomic<bool> block_enumeration{false},release_enumeration{false};
  std::atomic<bool> continuous{false},block_stop{false},release_stop{false},stop_entered{false};
  std::atomic<bool> fail_poll{false},fail_start{false},block_open{false},release_open{false},open_entered{false},final_tail{false};
  std::atomic<bool> invalid_pcm{false};
  std::atomic<unsigned> tails{0};
  std::atomic<std::uint64_t> stop_at{0};
  std::atomic<std::uint64_t> tail_raw{0},tail_position{0},tail_read_at{0},tail_released_at{0};
  std::atomic<float> tail_value{0};
  std::atomic<unsigned> tail_generation{0};
  std::mutex generation_mutex;
  std::map<unsigned,unsigned> normal_by_generation,tails_by_generation;
  float normal_sample=0.25f;
  bool generation_samples=false;
};
class ReplugCapture final : public Microsoft::WRL::RuntimeClass<Microsoft::WRL::RuntimeClassFlags<Microsoft::WRL::ClassicCom>,IAudioCaptureClient> {
  std::shared_ptr<EndpointStats> stats_;
  std::vector<float> samples_=std::vector<float>(960,0.25f);
  bool delivered_=false,stopped_=false,tail_pending_=false;DWORD owner_=0;
  std::uint64_t position_=0,tail_time_=0,clock_base_=product::qpc();
  float tail_sample_=0.5f;
  float normal_sample_=0.25f;
  unsigned generation_=0;
  void owner(){if(!owner_)owner_=GetCurrentThreadId();if(owner_!=GetCurrentThreadId())++stats_->wrong_thread;}
 public:
  explicit ReplugCapture(std::shared_ptr<EndpointStats> stats):stats_(std::move(stats)),generation_(stats_->opens.load()){
    normal_sample_=stats_->generation_samples?normal_generation_sample(generation_):stats_->normal_sample;
  }
  ~ReplugCapture(){if(owner_&&owner_!=GetCurrentThreadId())++stats_->wrong_thread;}
  void stopped(std::uint64_t at){
    stopped_=true;tail_pending_=stats_->final_tail;stats_->stop_at=at;
    // A deterministic native frame clock makes the held tail adjacent to its
    // previous packet, independently of scheduler delays while Stop is gated.
    tail_time_=clock_base_+position_*10000000/48000;
    tail_sample_=0.5f+static_cast<float>(stats_->tails.load())*0.0625f;
    stats_->tail_raw=tail_time_;stats_->tail_position=position_;stats_->tail_value=tail_sample_;
    stats_->tail_generation=generation_;
    stats_->tail_read_at=0;stats_->tail_released_at=0;
  }
  HRESULT STDMETHODCALLTYPE GetNextPacketSize(UINT32* frames)override{
    owner();++stats_->polls;
    if(stats_->fail_poll.exchange(false))return AUDCLNT_E_DEVICE_INVALIDATED;
    // The device clock runs independently of the servicing thread. A delayed
    // poll exposes every complete 10 ms packet, rather than slowing this
    // endpoint's frame clock by resetting a host timer after ReleaseBuffer.
    const auto due=clock_base_+(position_+480)*10000000/48000;
    *frames=tail_pending_||(!stopped_&&(!delivered_||stats_->continuous)&&product::qpc()>=due)?480:0;return S_OK;
  }
  HRESULT STDMETHODCALLTYPE GetBuffer(BYTE** data,UINT32* frames,DWORD* flags,UINT64* position,UINT64* time)override{
    owner();std::fill(samples_.begin(),samples_.end(),tail_pending_?tail_sample_:normal_sample_);
    if(tail_pending_)stats_->tail_read_at=product::qpc();
    if(stats_->invalid_pcm.exchange(false))samples_[0]=NAN;
    *data=reinterpret_cast<BYTE*>(samples_.data());*frames=480;*flags=0;*position=position_;*time=tail_pending_?tail_time_:clock_base_+position_*10000000/48000;return S_OK;
  }
  HRESULT STDMETHODCALLTYPE ReleaseBuffer(UINT32)override{
    owner();delivered_=true;position_+=480;
    {std::lock_guard lock(stats_->generation_mutex);if(tail_pending_)++stats_->tails_by_generation[generation_];else ++stats_->normal_by_generation[generation_];}
    if(tail_pending_){tail_pending_=false;stats_->tail_released_at=product::qpc();++stats_->tails;}++stats_->packets;return S_OK;
  }
};
class ReplugClient final : public Microsoft::WRL::RuntimeClass<Microsoft::WRL::RuntimeClassFlags<Microsoft::WRL::ClassicCom>,IAudioClient> {
  std::shared_ptr<EndpointStats> stats_;
  ComPtr<IAudioCaptureClient> capture_;ReplugCapture* capture_value_=nullptr;DWORD owner_=0;
 public:
  explicit ReplugClient(std::shared_ptr<EndpointStats> stats):stats_(std::move(stats)){
    if(stats_->live.fetch_add(1))++stats_->overlaps;
    ++stats_->opens;auto capture=Microsoft::WRL::Make<ReplugCapture>(stats_);capture_value_=capture.Get();capture_=capture;
  }
  ~ReplugClient(){capture_.Reset();if(owner_&&owner_!=GetCurrentThreadId())++stats_->wrong_thread;--stats_->live;++stats_->released;}
  HRESULT STDMETHODCALLTYPE Initialize(AUDCLNT_SHAREMODE,DWORD,REFERENCE_TIME,REFERENCE_TIME,const WAVEFORMATEX*,LPCGUID)override{return S_OK;}
  HRESULT STDMETHODCALLTYPE GetBufferSize(UINT32*)override{return E_NOTIMPL;}
  HRESULT STDMETHODCALLTYPE GetStreamLatency(REFERENCE_TIME*)override{return E_NOTIMPL;}
  HRESULT STDMETHODCALLTYPE GetCurrentPadding(UINT32*)override{return E_NOTIMPL;}
  HRESULT STDMETHODCALLTYPE IsFormatSupported(AUDCLNT_SHAREMODE,const WAVEFORMATEX*,WAVEFORMATEX**)override{return E_NOTIMPL;}
  HRESULT STDMETHODCALLTYPE GetMixFormat(WAVEFORMATEX** format)override{
    *format=static_cast<WAVEFORMATEX*>(CoTaskMemAlloc(sizeof(WAVEFORMATEX)));if(!*format)return E_OUTOFMEMORY;
    **format={WAVE_FORMAT_IEEE_FLOAT,2,48000,384000,8,32,0};return S_OK;
  }
  HRESULT STDMETHODCALLTYPE GetDevicePeriod(REFERENCE_TIME*,REFERENCE_TIME*)override{return E_NOTIMPL;}
  HRESULT STDMETHODCALLTYPE Start()override{return stats_->fail_start.exchange(false)?AUDCLNT_E_DEVICE_INVALIDATED:S_OK;}
  HRESULT STDMETHODCALLTYPE Stop()override{
    if(owner_!=GetCurrentThreadId())++stats_->wrong_thread;
    ++stats_->stops;capture_value_->stopped(product::qpc());stats_->stop_entered=true;
    while(stats_->block_stop&&!stats_->release_stop)Sleep(1);
    return S_OK;
  }
  HRESULT STDMETHODCALLTYPE Reset()override{return E_NOTIMPL;}
  HRESULT STDMETHODCALLTYPE SetEventHandle(HANDLE)override{return E_NOTIMPL;}
  HRESULT STDMETHODCALLTYPE GetService(REFIID iid,void** value)override{
    if(iid!=__uuidof(IAudioCaptureClient)){*value=nullptr;return E_NOINTERFACE;}
    owner_=GetCurrentThreadId();capture_->AddRef();*value=capture_.Get();return S_OK;
  }
};
class ReplugDevice final : public Microsoft::WRL::RuntimeClass<Microsoft::WRL::RuntimeClassFlags<Microsoft::WRL::ClassicCom>,IMMDevice> {
  std::shared_ptr<EndpointStats> stats_;
  std::wstring id_;
 public:
  static constexpr auto id=L"bluetooth-same-endpoint-id";
  explicit ReplugDevice(std::shared_ptr<EndpointStats> stats,std::wstring identity=id):stats_(std::move(stats)),id_(std::move(identity)){}
  HRESULT STDMETHODCALLTYPE Activate(REFIID iid,DWORD,PROPVARIANT*,void** value)override{
    *value=nullptr;if(iid!=__uuidof(IAudioClient))return E_NOINTERFACE;
    stats_->open_entered=true;while(stats_->block_open&&!stats_->release_open)Sleep(1);
    auto client=Microsoft::WRL::Make<ReplugClient>(stats_);return client.CopyTo(iid,value);
  }
  HRESULT STDMETHODCALLTYPE OpenPropertyStore(DWORD,IPropertyStore**)override{return E_NOTIMPL;}
  HRESULT STDMETHODCALLTYPE GetId(LPWSTR* value)override{
    const auto bytes=(id_.size()+1)*sizeof(wchar_t);*value=static_cast<LPWSTR>(CoTaskMemAlloc(bytes));
    if(!*value)return E_OUTOFMEMORY;std::memcpy(*value,id_.c_str(),bytes);return S_OK;
  }
  HRESULT STDMETHODCALLTYPE GetState(DWORD* state)override{*state=DEVICE_STATE_ACTIVE;return S_OK;}
};
class ReplugDevices final : public Microsoft::WRL::RuntimeClass<Microsoft::WRL::RuntimeClassFlags<Microsoft::WRL::ClassicCom>,IMMDeviceCollection> {
  std::vector<ComPtr<IMMDevice>> devices_;
 public:
  explicit ReplugDevices(ComPtr<IMMDevice> device,ComPtr<IMMDevice> second={}):devices_{std::move(device)}{if(second)devices_.push_back(std::move(second));}
  HRESULT STDMETHODCALLTYPE GetCount(UINT* count)override{*count=static_cast<UINT>(devices_.size());return S_OK;}
  HRESULT STDMETHODCALLTYPE Item(UINT index,IMMDevice** value)override{if(index>=devices_.size())return E_INVALIDARG;return devices_[index].CopyTo(value);}
};
class ReplugEnumerator final : public Microsoft::WRL::RuntimeClass<Microsoft::WRL::RuntimeClassFlags<Microsoft::WRL::ClassicCom>,IMMDeviceEnumerator> {
  std::shared_ptr<EndpointStats> stats_;ComPtr<IMMDevice> device_,second_;
  std::mutex mutex_;ComPtr<IMMNotificationClient> callback_;
 public:
  explicit ReplugEnumerator(std::shared_ptr<EndpointStats> stats,std::shared_ptr<EndpointStats> second={}):stats_(std::move(stats)),device_(Microsoft::WRL::Make<ReplugDevice>(stats_)){
    if(second)second_=Microsoft::WRL::Make<ReplugDevice>(std::move(second),L"healthy-independent-output");
  }
  HRESULT STDMETHODCALLTYPE EnumAudioEndpoints(EDataFlow,DWORD,IMMDeviceCollection** value)override{
    const auto call=++stats_->enumerations;
    if(call>1&&stats_->block_enumeration){
      const auto deadline=GetTickCount64()+5000;
      while(!stats_->release_enumeration&&GetTickCount64()<deadline)Sleep(1);
      if(!stats_->release_enumeration)return E_ABORT;
    }
    auto devices=Microsoft::WRL::Make<ReplugDevices>(device_,second_);return devices.CopyTo(value);
  }
  HRESULT STDMETHODCALLTYPE GetDefaultAudioEndpoint(EDataFlow,ERole role,IMMDevice** value)override{return second_&&role==eMultimedia?second_.CopyTo(value):device_.CopyTo(value);}
  HRESULT STDMETHODCALLTYPE GetDevice(LPCWSTR,IMMDevice** value)override{return device_.CopyTo(value);}
  HRESULT STDMETHODCALLTYPE RegisterEndpointNotificationCallback(IMMNotificationClient* callback)override{std::lock_guard lock(mutex_);callback_=callback;return S_OK;}
  HRESULT STDMETHODCALLTYPE UnregisterEndpointNotificationCallback(IMMNotificationClient*)override{std::lock_guard lock(mutex_);callback_.Reset();return S_OK;}
  ComPtr<IMMNotificationClient> callback(){std::lock_guard lock(mutex_);return callback_;}
};

void enumeration_does_not_block_active_readers(){
  auto stats=std::make_shared<EndpointStats>();stats->block_enumeration=true;
  auto devices=Microsoft::WRL::Make<ReplugEnumerator>(stats);
  product::SystemCapture reader([devices]{ComPtr<IMMDeviceEnumerator> value;devices.As(&value);return value;});
  wait_for([&]{return stats->packets.load()==1&&stats->enumerations.load()>1;},"independent enumeration did not start");
  const auto before=stats->polls.load();
  wait_for([&]{return stats->polls.load()>before+50;},"device enumeration blocked acquisition buffer draining");
  stats->release_enumeration=true;reader.stop();
  require(!stats->live&&!stats->wrong_thread,"enumeration preparation leaked an acquisition owner");
}

void same_id_reconnection_releases_old_render_client(){
  auto stats=std::make_shared<EndpointStats>();auto devices=Microsoft::WRL::Make<ReplugEnumerator>(stats);
  product::SystemCapture reader([devices]{ComPtr<IMMDeviceEnumerator> value;devices.As(&value);return value;});
  wait_for([&]{return stats->packets.load()==1;},"initial render audio did not arrive");
  auto notifications=devices->callback();require(notifications.Get()!=nullptr,"render endpoint notifications were not owned");
  // Both transitions occur before enumeration can observe an absent device.
  // ID, default endpoint, format and even all old WASAPI HRESULTs stay equal.
  notifications->OnDeviceStateChanged(ReplugDevice::id,DEVICE_STATE_UNPLUGGED);
  notifications->OnDeviceStateChanged(ReplugDevice::id,DEVICE_STATE_ACTIVE);
  wait_for([&]{return stats->opens.load()==2;},"same-ID Bluetooth reconnection reused its old render client");
  wait_for([&]{return stats->packets.load()==2;},"reconnected render client did not deliver audio");
  require(stats->released==1&&stats->overlaps==0,"old Bluetooth render interfaces remained alive during reactivation");
  notifications->OnPropertyValueChanged(ReplugDevice::id,product::audio_inspection::engine_format);
  wait_for([&]{return stats->opens.load()==3;},"Bluetooth format reconfiguration reused its old render client");
  const auto enumerations=stats->enumerations.load();
  notifications->OnDefaultDeviceChanged(eRender,eConsole,ReplugDevice::id);
  notifications->OnPropertyValueChanged(ReplugDevice::id,PROPERTYKEY{});
  notifications->OnDeviceStateChanged(L"unrelated-microphone",DEVICE_STATE_ACTIVE);
  wait_for([&]{return stats->enumerations.load()>enumerations+1;},"device refresh did not finish");
  require(stats->opens==3,"selection or unrelated property changes restarted a healthy render stream");
  reader.stop();unsigned frames=0;while(auto packet=reader.queue.pop())frames+=packet->frames;
  require(frames>0&&stats->packets==3,"endpoint replacement lost received render audio");
  require(stats->released==3&&!stats->live&&!stats->overlaps&&!stats->wrong_thread,"render clients were not released on their WASAPI owner");
  require(devices->callback().Get()==nullptr,"render notification registration survived native shutdown");
}
void microphone_reconnection_revokes_old_native_generation(){
  auto stats=std::make_shared<EndpointStats>();auto devices=Microsoft::WRL::Make<ReplugEnumerator>(stats);
  ComPtr<IMMDeviceEnumerator> enumerator;devices.As(&enumerator);
  auto lifetime=std::make_shared<product::EndpointLifetime>(enumerator,ReplugDevice::id);
  auto notifications=devices->callback();
  notifications->OnDeviceStateChanged(L"another-output",DEVICE_STATE_UNPLUGGED);
  notifications->OnDefaultDeviceChanged(eRender,eConsole,L"another-output");
  lifetime->require_current();
  ComPtr<IAudioClient> client=Microsoft::WRL::Make<ReplugClient>(stats);
  product::AsyncCapture reader(std::move(client),2,true,"same-id-microphone",lifetime);
  require(reader.queue.pop().has_value(),"microphone did not receive its valid prefix");
  notifications->OnDeviceStateChanged(ReplugDevice::id,DEVICE_STATE_UNPLUGGED);
  notifications->OnDeviceStateChanged(ReplugDevice::id,DEVICE_STATE_ACTIVE);
  wait_for([&]{return stats->stops.load()!=0;},"disconnected microphone native owner was retained");
  reader.stop();
  bool invalidated=false;try{reader.queue.pop();}catch(const product::InputFailure& error){invalidated=error.result==AUDCLNT_E_DEVICE_INVALIDATED;}
  require(invalidated,"microphone waited for another receive timeout instead of honoring physical disconnection");
  bool obsolete=false;try{lifetime->require_current();}catch(const product::InputFailure&){obsolete=true;}
  require(obsolete,"rapid reconnection made the old microphone preparation current again");
  require(stats->released==1&&!stats->wrong_thread&&!stats->live,"old microphone interfaces survived disconnection");
  lifetime.reset();require(devices->callback().Get()==nullptr,"microphone endpoint callback was not unregistered");
  auto fresh=std::make_shared<product::EndpointLifetime>(enumerator,ReplugDevice::id);
  // An in-flight callback belonging to the old registration cannot revoke a
  // new native connection just because the device ID was reused.
  notifications->OnDeviceStateChanged(ReplugDevice::id,DEVICE_STATE_UNPLUGGED);
  fresh->require_current();
  ComPtr<IAudioClient> replacement=Microsoft::WRL::Make<ReplugClient>(stats);
  product::AsyncCapture recovered(std::move(replacement),2,true,"reconnected-microphone",fresh);
  require(recovered.queue.pop().has_value(),"fresh microphone generation did not receive audio");
  recovered.stop();fresh.reset();
  require(stats->released==2&&!stats->live&&!stats->overlaps&&!stats->wrong_thread,"microphone reactivation overlapped its old native generation");
  require(devices->callback().Get()==nullptr,"fresh microphone callback survived shutdown");
}
struct EndpointRelease {
  std::shared_ptr<EndpointStats> stats;
  ~EndpointRelease(){stats->release_stop=true;stats->release_open=true;stats->release_enumeration=true;}
};
void repeated_slow_endpoint_stops_keep_the_healthy_output(){
  auto bluetooth=std::make_shared<EndpointStats>(),healthy=std::make_shared<EndpointStats>();
  bluetooth->continuous=healthy->continuous=true;bluetooth->final_tail=true;
  bluetooth->normal_sample=healthy->normal_sample=0;
  bluetooth->generation_samples=true;
  auto devices=Microsoft::WRL::Make<ReplugEnumerator>(bluetooth,healthy);
  product::SystemCapture reader([devices]{ComPtr<IMMDeviceEnumerator> value;devices.As(&value);return value;});
  EndpointRelease release{bluetooth};
  wait_for([&]{return bluetooth->packets>5&&healthy->packets>5;},"both render owners did not start");
  auto notifications=devices->callback();std::uint64_t mixed_frames=0,mixed_through=0,marked_estimated=0;
  std::array<unsigned,3> tail_frames{};
  std::array<unsigned,4> normal_frames{};
  std::array<std::uint64_t,3> tail_first{},tail_end{};
  std::map<float,std::uint64_t> emitted_values;
  product::Json recent_nonzero=product::Json::array();
  auto consume=[&]{while(auto packet=reader.queue.pop()){
    mixed_frames+=packet->frames;mixed_through=std::max(mixed_through,packet->time+std::uint64_t(packet->frames)*10000000/48000);
    if(packet->samples.empty())continue;
    const auto extremes=std::minmax_element(packet->samples.begin(),packet->samples.end());
    if(*extremes.first!=0||*extremes.second!=0){
      if(recent_nonzero.size()==24)recent_nonzero.erase(recent_nonzero.begin());
      recent_nonzero.push_back(product::Json{{"time",packet->time},{"arrival",packet->arrival},{"frames",packet->frames},{"flags",packet->flags},{"min",*extremes.first},{"max",*extremes.second}});
    }
    for(auto sample:packet->samples)++emitted_values[sample];
    for(unsigned frame=0;frame<packet->frames;++frame){
      for(unsigned generation=1;generation<=normal_frames.size();++generation){
        const auto sentinel=normal_generation_sample(generation);
        if(packet->samples[frame*2]==sentinel&&packet->samples[frame*2+1]==sentinel)++normal_frames[generation-1];
      }
      for(unsigned cycle=0;cycle<tail_frames.size();++cycle){
        const auto marker=0.5f+static_cast<float>(cycle)*0.0625f;
        if(packet->samples[frame*2]==marker&&packet->samples[frame*2+1]==marker){
          ++tail_frames[cycle];if(packet->flags&AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR)++marked_estimated;
          const auto first=packet->time+std::uint64_t(frame)*10000000/48000;
          if(!tail_first[cycle])tail_first[cycle]=first;tail_end[cycle]=first+10000000/48000;
        }
      }
    }
  }};
  auto dump_tail_failure=[&](unsigned cycle){
    const auto alignment=reader.alignment_diagnostics();product::Json values=product::Json::array(),ranges=product::Json::array();
    product::Json generations=product::Json::array();
    {std::lock_guard lock(bluetooth->generation_mutex);for(unsigned generation=1;generation<=bluetooth->opens.load();++generation)generations.push_back(product::Json{{"generation",generation},{"normalPackets",bluetooth->normal_by_generation[generation]},{"tailPackets",bluetooth->tails_by_generation[generation]}});}
    for(const auto& [value,count]:emitted_values)values.push_back(product::Json{{"value",value},{"samples",count}});
    for(const auto& entry:alignment.losses)ranges.push_back(product::Json{{"rawFirstFrame",entry.first},{"frames",entry.frames},{"reason",entry.reason},{"mappedFirstFrame",entry.mapped_first}});
    std::cerr<<"retirement_tail_context: "+product::Json{{"cycle",cycle},{"normalGenerationFrames",normal_frames},{"markerFrames",tail_frames},{"markerFirstQpc",tail_first},{"markerEndQpc",tail_end},{"mixedFrames",mixed_frames},{"mixedThrough",mixed_through},{"emittedValues",values},{"recentNonzeroPackets",recent_nonzero},{"generations",generations},{"nativeTailCount",bluetooth->tails.load()},{"nativeTailGeneration",bluetooth->tail_generation.load()},{"nativeTailRawQpc",bluetooth->tail_raw.load()},{"nativeTailPosition",bluetooth->tail_position.load()},{"nativeTailValue",bluetooth->tail_value.load()},{"nativeStopQpc",bluetooth->stop_at.load()},{"nativeTailReadQpc",bluetooth->tail_read_at.load()},{"nativeTailReleaseQpc",bluetooth->tail_released_at.load()},{"opens",bluetooth->opens.load()},{"released",bluetooth->released.load()},{"live",bluetooth->live.load()},{"nativePackets",bluetooth->packets.load()},{"retimedPackets",alignment.retimed_packets},{"retimedFrames",alignment.retimed_frames},{"clockAdjustmentFrames",alignment.clock_adjustment_frames},{"ranges",ranges}}.dump()+"\n";
  };
  auto await_mixed_generation=[&](unsigned generation){
    const auto deadline=GetTickCount64()+5000;
    while(!normal_frames.at(generation-1)&&GetTickCount64()<deadline){consume();Sleep(1);}
    if(!normal_frames.at(generation-1))dump_tail_failure(generation-1);
    require(normal_frames.at(generation-1)>0,"replacement native generation was not adopted into mixed PCM");
  };
  for(unsigned cycle=0;cycle<3;++cycle){
    // Native construction/Start is not adoption. Revoke only after this exact
    // generation has crossed SystemCapture's acceptance and mixing boundary.
    await_mixed_generation(cycle+1);
    const auto opens=bluetooth->opens.load(),peer_packets=healthy->packets.load(),enumerations=bluetooth->enumerations.load();
    bluetooth->release_stop=false;bluetooth->stop_entered=false;bluetooth->block_stop=true;
    notifications->OnDeviceStateChanged(ReplugDevice::id,DEVICE_STATE_UNPLUGGED);
    notifications->OnDeviceStateChanged(ReplugDevice::id,DEVICE_STATE_ACTIVE);
    wait_for([&]{return bluetooth->stop_entered.load();},"retired Bluetooth owner did not enter Stop");
    const auto mixed_before=mixed_frames;
    // Observe the actual mixed frontier past the held source, instead of
    // assuming a fixed sleep produced a particular scheduling/clock outcome.
    wait_for([&]{consume();return healthy->packets>peer_packets+50&&bluetooth->enumerations>enumerations+2&&mixed_through>bluetooth->stop_at+5000000;},"gated Stop prevented the healthy mixed frontier from progressing");
    require(healthy->packets>peer_packets+50&&healthy->opens==1,"Bluetooth Stop stalled or reopened the healthy endpoint");
    require(bluetooth->enumerations>enumerations+2&&mixed_frames>mixed_before,"Bluetooth Stop blocked output reconciliation or healthy mixed PCM");
    require(bluetooth->opens==opens&&!bluetooth->overlaps,"Bluetooth endpoint reopened before actual native release");
    bluetooth->release_stop=true;
    wait_for([&]{consume();return bluetooth->opens==opens+1&&bluetooth->released==opens;},"completed retirement did not permit one fresh same-ID owner");
    const auto tail_deadline=GetTickCount64()+5000;
    while(tail_frames[cycle]<480&&GetTickCount64()<tail_deadline){consume();Sleep(1);}
    if(tail_frames[cycle]!=480||bluetooth->tails!=cycle+1)dump_tail_failure(cycle);
    require(tail_frames[cycle]>=480,"slow Stop lost its distinct received native tail");
    require(tail_frames[cycle]==480&&bluetooth->tails==cycle+1,"native retirement tail was duplicated or only partially retained");
    bluetooth->block_stop=false;
  }
  await_mixed_generation(4);
  reader.stop();consume();
  require(mixed_frames>0&&std::all_of(tail_frames.begin(),tail_frames.end(),[](auto count){return count==480;}),"repeated retirement did not retain each complete distinct tail exactly once");
  const auto alignment=reader.alignment_diagnostics();
  require(!marked_estimated||(alignment.retimed_frames>=marked_estimated&&!alignment.losses.empty()),"estimated retirement PCM lacked its clock-rebase diagnostics");
  for(auto stats:{bluetooth,healthy})require(!stats->live&&!stats->overlaps&&!stats->wrong_thread&&stats->released==stats->opens,"render retirement leaked native ownership or released COM on a different thread");
}
void rapid_replug_discards_only_the_unadopted_opening(){
  auto bluetooth=std::make_shared<EndpointStats>(),healthy=std::make_shared<EndpointStats>();
  bluetooth->continuous=healthy->continuous=true;bluetooth->final_tail=true;
  bluetooth->generation_samples=true;healthy->normal_sample=0;bluetooth->block_open=true;
  auto devices=Microsoft::WRL::Make<ReplugEnumerator>(bluetooth,healthy);
  std::jthread watchdog([bluetooth]{const auto deadline=GetTickCount64()+5000;while(!bluetooth->release_open&&GetTickCount64()<deadline)Sleep(1);bluetooth->release_open=true;});
  product::SystemCapture reader([devices]{ComPtr<IMMDeviceEnumerator> value;devices.As(&value);return value;});
  EndpointRelease release{bluetooth};
  std::array<unsigned,2> normal_frames{},tail_frames{};std::uint64_t mixed_frames=0;
  auto consume=[&]{while(auto packet=reader.queue.pop()){
    mixed_frames+=packet->frames;if(packet->samples.empty())continue;
    for(unsigned frame=0;frame<packet->frames;++frame)for(unsigned generation=1;generation<=2;++generation){
      if(packet->samples[frame*2]==normal_generation_sample(generation)&&packet->samples[frame*2+1]==normal_generation_sample(generation))++normal_frames[generation-1];
      const auto tail=0.5f+static_cast<float>(generation-1)*0.0625f;
      if(packet->samples[frame*2]==tail&&packet->samples[frame*2+1]==tail)++tail_frames[generation-1];
    }
  }};
  wait_for([&]{consume();return bluetooth->open_entered&&healthy->packets>5&&mixed_frames>0;},"pending Bluetooth activation blocked healthy mixed startup");
  require(!bluetooth->opens,"opening gate did not hold the obsolete native factory");
  const auto enumerations=bluetooth->enumerations.load(),peer_packets=healthy->packets.load();
  auto notifications=devices->callback();
  notifications->OnDeviceStateChanged(ReplugDevice::id,DEVICE_STATE_UNPLUGGED);
  notifications->OnDeviceStateChanged(ReplugDevice::id,DEVICE_STATE_ACTIVE);
  // At most one enumeration was already in flight. Two newly started
  // snapshots prove the controller processed revocation while Activate waits.
  wait_for([&]{consume();return bluetooth->enumerations>enumerations+1&&healthy->packets>peer_packets+5;},"rapid replug did not reconcile the cancelled opening independently");
  require(!bluetooth->opens,"same-ID factory overlapped its still-pending obsolete activation");
  bluetooth->release_open=true;
  wait_for([&]{consume();return normal_frames[1]>0;},"fresh generation did not adopt after the cancelled native factory closed");
  require(!normal_frames[0]&&!tail_frames[0]&&bluetooth->opens==2&&bluetooth->released==1&&bluetooth->tails==1,"an unadopted generation's PCM was recorded or its native disposal was skipped");
  require(healthy->opens==1&&healthy->packets>peer_packets+10,"rapid opening cancellation interrupted the healthy endpoint");
  reader.stop();watchdog.join();consume();
  if(normal_frames[0]||tail_frames[0]||tail_frames[1]!=480)std::cerr<<"cancelled_open_context: "+product::Json{{"normalFrames",normal_frames},{"tailFrames",tail_frames},{"nativeTails",bluetooth->tails.load()},{"opens",bluetooth->opens.load()},{"released",bluetooth->released.load()}}.dump()+"\n";
  require(!normal_frames[0]&&!tail_frames[0]&&tail_frames[1]==480,"cancelled opening PCM leaked or the adopted replacement lost its exact native tail");
  for(auto stats:{bluetooth,healthy})require(!stats->live&&!stats->overlaps&&!stats->wrong_thread&&stats->released==stats->opens,"rapid replug leaked, overlapped or released a native owner on a different thread");
}
void endpoint_failures_retry_without_restarting_the_peer(){
  auto bluetooth=std::make_shared<EndpointStats>(),healthy=std::make_shared<EndpointStats>();
  bluetooth->continuous=healthy->continuous=true;
  auto devices=Microsoft::WRL::Make<ReplugEnumerator>(bluetooth,healthy);
  product::SystemCapture reader([devices]{ComPtr<IMMDeviceEnumerator> value;devices.As(&value);return value;});
  wait_for([&]{return bluetooth->packets>5&&healthy->packets>5;},"initial peer capture was unavailable");
  const auto initial=bluetooth->opens.load(),peer_packets=healthy->packets.load();
  bluetooth->fail_start=true;bluetooth->fail_poll=true;
  wait_for([&]{while(reader.queue.pop()){}return bluetooth->opens==initial+1;},"invalidated endpoint was not retried");
  const auto failed_open_at=GetTickCount64();
  while(GetTickCount64()-failed_open_at<500){while(reader.queue.pop()){}Sleep(10);}
  require(bluetooth->opens==initial+1,"failed endpoint Start bypassed acquisition backoff");
  wait_for([&]{while(reader.queue.pop()){}return bluetooth->opens==initial+2&&bluetooth->packets>10;},"endpoint did not recover after its bounded local retry");
  require(healthy->opens==1&&healthy->packets>peer_packets+100,"endpoint read/Start failure interrupted its healthy peer");
  reader.stop();while(reader.queue.pop()){}
  for(auto stats:{bluetooth,healthy})require(!stats->live&&!stats->overlaps&&!stats->wrong_thread&&stats->released==stats->opens,"endpoint failure leaked or overlapped a native generation");
}
void final_stop_waits_for_cancelled_open_disposal(){
  auto bluetooth=std::make_shared<EndpointStats>(),healthy=std::make_shared<EndpointStats>();
  bluetooth->block_open=true;healthy->continuous=true;
  auto devices=Microsoft::WRL::Make<ReplugEnumerator>(bluetooth,healthy);
  // A watchdog also makes an old synchronous startup fail finitely.
  std::jthread watchdog([bluetooth]{const auto deadline=GetTickCount64()+5000;while(!bluetooth->release_open&&GetTickCount64()<deadline)Sleep(1);bluetooth->release_open=true;});
  product::SystemCapture reader([devices]{ComPtr<IMMDeviceEnumerator> value;devices.As(&value);return value;});
  wait_for([&]{return bluetooth->open_entered&&healthy->packets>2;},"independent endpoint opening stalled healthy startup");
  std::atomic<bool> stopped{false};std::exception_ptr error;
  std::jthread stopper([&]{try{reader.stop();}catch(...){error=std::current_exception();}stopped=true;});
  EndpointRelease release{bluetooth};
  Sleep(50);require(!stopped,"SystemCapture advertised closure with an unreturned native open");
  bluetooth->release_open=true;stopper.join();watchdog.join();if(error)std::rethrow_exception(error);
  require(stopped&&bluetooth->opens==1&&bluetooth->released==1&&!bluetooth->live&&!bluetooth->wrong_thread,"cancelled late native open survived final stop");
  require(!healthy->live&&healthy->released==healthy->opens&&!healthy->wrong_thread,"final stop retained the healthy native owner");
}
void endpoint_ingress_has_one_aggregate_budget(){
  auto budget=std::make_shared<product::PacketIngressBudget>();
  {
    product::PacketQueue first,second;first.attach_ingress(budget);second.attach_ingress(budget);
    auto silence=[](){product::CapturePacket packet;packet.frames=48000;packet.flags=AUDCLNT_BUFFERFLAGS_SILENT;return packet;};
    for(unsigned i=0;i<10;++i){first.push(silence());second.push(silence());}
    bool bounded=false;try{second.push(silence());}catch(const std::runtime_error&){bounded=true;}
    require(bounded&&budget->pending_nanoseconds()==20000000000ULL,"render endpoints multiplied the transient 20-second ingress allowance");
    require(first.pop().has_value(),"aggregate ingress lost its accepted packet");second.push(silence());
  }
  require(!budget->pending_nanoseconds(),"cancelled endpoint queue retained shared transient ingress credits");
}
void fatal_error_is_terminal_only_after_retirement_tail(){
  auto bluetooth=std::make_shared<EndpointStats>(),healthy=std::make_shared<EndpointStats>();
  bluetooth->continuous=healthy->continuous=true;bluetooth->final_tail=true;
  bluetooth->normal_sample=healthy->normal_sample=0;
  auto devices=Microsoft::WRL::Make<ReplugEnumerator>(bluetooth,healthy);
  product::SystemCapture reader([devices]{ComPtr<IMMDeviceEnumerator> value;devices.As(&value);return value;});
  EndpointRelease release{bluetooth};
  wait_for([&]{return bluetooth->packets>5&&healthy->packets>5;},"fatal-tail fixture did not receive its valid prefix");
  bluetooth->block_stop=true;bluetooth->release_stop=false;healthy->invalid_pcm=true;
  wait_for([&]{return bluetooth->stop_entered.load();},"fatal PCM error did not retire the gated endpoint");
  bool premature=false,terminal=false;unsigned tail_frames=0;
  auto consume=[&]{
    try{while(auto packet=reader.queue.pop()){
      if(packet->samples.empty())continue;
      for(unsigned frame=0;frame<packet->frames;++frame)if(packet->samples[frame*2]==0.5f&&packet->samples[frame*2+1]==0.5f)++tail_frames;
    }}
    catch(const product::InputDataFault&){terminal=true;}
  };
  const auto until=GetTickCount64()+250;
  while(GetTickCount64()<until){consume();premature|=terminal;Sleep(10);}
  bluetooth->release_stop=true;reader.stop();consume();
  require(!premature,"terminal queue failure preceded an owned retirement's final PCM");
  require(terminal&&tail_frames==480&&bluetooth->tails==1,"fatal shutdown lost or duplicated its distinct native tail, or hid the real PCM error");
  bool terminal_again=false;try{reader.queue.pop();}catch(const product::InputDataFault&){terminal_again=true;}
  require(terminal_again&&!reader.queue.pending_frames(),"PCM appeared after the queue's terminal failure");
  for(auto stats:{bluetooth,healthy})require(!stats->live&&!stats->wrong_thread&&stats->released==stats->opens,"fatal shutdown left an owned native generation alive");
}
void shutdown_storage_failure_supersedes_an_input_fault(){
  auto bluetooth=std::make_shared<EndpointStats>(),healthy=std::make_shared<EndpointStats>();
  bluetooth->continuous=healthy->continuous=true;bluetooth->final_tail=true;
  bluetooth->normal_sample=healthy->normal_sample=0;
  auto devices=Microsoft::WRL::Make<ReplugEnumerator>(bluetooth,healthy);
  product::SystemCapture reader([devices]{ComPtr<IMMDeviceEnumerator> value;devices.As(&value);return value;});
  EndpointRelease release{bluetooth};auto pending=std::make_shared<product::StoragePending>();
  reader.queue.attach_storage(pending,1);
  wait_for([&]{return bluetooth->packets>5&&healthy->packets>5;},"storage-priority fixture did not receive its valid prefix");
  bluetooth->block_stop=true;bluetooth->release_stop=false;healthy->invalid_pcm=true;
  wait_for([&]{return bluetooth->stop_entered.load();},"input fault did not enter the owned shutdown gate");
  constexpr auto marker="fatal shutdown storage acquisition marker";
  pending->fail(std::make_exception_ptr(std::runtime_error(marker)));
  bluetooth->release_stop=true;reader.stop();
  bool fatal=false;
  try{while(reader.queue.pop()){}}
  catch(const std::runtime_error& error){fatal=std::string(error.what())==marker;}
  require(fatal,"shutdown storage failure was hidden by an earlier recoverable input fault");
  require(bluetooth->tails==1,"shutdown priority fixture did not attempt its held native tail");
  for(auto stats:{bluetooth,healthy})require(!stats->live&&!stats->wrong_thread&&stats->released==stats->opens,"storage failure priority left an owned native generation alive");
}
int main(){
  try{
    product::CaptureApartment apartment;
    enumeration_does_not_block_active_readers();
    same_id_reconnection_releases_old_render_client();
    microphone_reconnection_revokes_old_native_generation();
    repeated_slow_endpoint_stops_keep_the_healthy_output();
    rapid_replug_discards_only_the_unadopted_opening();
    endpoint_failures_retry_without_restarting_the_peer();
    final_stop_waits_for_cancelled_open_disposal();
    endpoint_ingress_has_one_aggregate_budget();
    fatal_error_is_terminal_only_after_retirement_tail();
    shutdown_storage_failure_supersedes_an_input_fault();
    std::cout<<"same-ID reconnect: repeated gated endpoint retirement, peer continuity, local retry, cancelled-open closure, shared ingress budget and COM owner cleanup passed\n";
    return 0;
  }catch(const std::exception& error){std::cerr<<error.what()<<'\n';return 1;}
}
