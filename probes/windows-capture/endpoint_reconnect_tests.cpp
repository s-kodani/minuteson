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
struct EndpointStats {
  std::atomic<unsigned> opens{0},live{0},released{0},overlaps{0},stops{0},enumerations{0},wrong_thread{0},packets{0};
  std::atomic<unsigned> polls{0};
  std::atomic<bool> block_enumeration{false},release_enumeration{false};
};
class ReplugCapture final : public Microsoft::WRL::RuntimeClass<Microsoft::WRL::RuntimeClassFlags<Microsoft::WRL::ClassicCom>,IAudioCaptureClient> {
  std::shared_ptr<EndpointStats> stats_;
  std::vector<float> samples_=std::vector<float>(960,0.25f);
  bool delivered_=false;DWORD owner_=0;
  void owner(){if(!owner_)owner_=GetCurrentThreadId();if(owner_!=GetCurrentThreadId())++stats_->wrong_thread;}
 public:
  explicit ReplugCapture(std::shared_ptr<EndpointStats> stats):stats_(std::move(stats)){}
  ~ReplugCapture(){if(owner_&&owner_!=GetCurrentThreadId())++stats_->wrong_thread;}
  HRESULT STDMETHODCALLTYPE GetNextPacketSize(UINT32* frames)override{owner();++stats_->polls;*frames=delivered_?0:480;return S_OK;}
  HRESULT STDMETHODCALLTYPE GetBuffer(BYTE** data,UINT32* frames,DWORD* flags,UINT64* position,UINT64* time)override{
    owner();*data=reinterpret_cast<BYTE*>(samples_.data());*frames=480;*flags=0;*position=0;*time=product::qpc();return S_OK;
  }
  HRESULT STDMETHODCALLTYPE ReleaseBuffer(UINT32)override{owner();delivered_=true;++stats_->packets;return S_OK;}
};
class ReplugClient final : public Microsoft::WRL::RuntimeClass<Microsoft::WRL::RuntimeClassFlags<Microsoft::WRL::ClassicCom>,IAudioClient> {
  std::shared_ptr<EndpointStats> stats_;
  ComPtr<IAudioCaptureClient> capture_;DWORD owner_=0;
 public:
  explicit ReplugClient(std::shared_ptr<EndpointStats> stats):stats_(std::move(stats)){
    if(stats_->live.fetch_add(1))++stats_->overlaps;
    ++stats_->opens;capture_=Microsoft::WRL::Make<ReplugCapture>(stats_);
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
  HRESULT STDMETHODCALLTYPE Start()override{return S_OK;}
  HRESULT STDMETHODCALLTYPE Stop()override{++stats_->stops;return S_OK;}
  HRESULT STDMETHODCALLTYPE Reset()override{return E_NOTIMPL;}
  HRESULT STDMETHODCALLTYPE SetEventHandle(HANDLE)override{return E_NOTIMPL;}
  HRESULT STDMETHODCALLTYPE GetService(REFIID iid,void** value)override{
    if(iid!=__uuidof(IAudioCaptureClient)){*value=nullptr;return E_NOINTERFACE;}
    owner_=GetCurrentThreadId();capture_->AddRef();*value=capture_.Get();return S_OK;
  }
};
class ReplugDevice final : public Microsoft::WRL::RuntimeClass<Microsoft::WRL::RuntimeClassFlags<Microsoft::WRL::ClassicCom>,IMMDevice> {
  std::shared_ptr<EndpointStats> stats_;
 public:
  static constexpr auto id=L"bluetooth-same-endpoint-id";
  explicit ReplugDevice(std::shared_ptr<EndpointStats> stats):stats_(std::move(stats)){}
  HRESULT STDMETHODCALLTYPE Activate(REFIID iid,DWORD,PROPVARIANT*,void** value)override{
    *value=nullptr;if(iid!=__uuidof(IAudioClient))return E_NOINTERFACE;
    auto client=Microsoft::WRL::Make<ReplugClient>(stats_);return client.CopyTo(iid,value);
  }
  HRESULT STDMETHODCALLTYPE OpenPropertyStore(DWORD,IPropertyStore**)override{return E_NOTIMPL;}
  HRESULT STDMETHODCALLTYPE GetId(LPWSTR* value)override{
    const auto bytes=(wcslen(id)+1)*sizeof(wchar_t);*value=static_cast<LPWSTR>(CoTaskMemAlloc(bytes));
    if(!*value)return E_OUTOFMEMORY;std::memcpy(*value,id,bytes);return S_OK;
  }
  HRESULT STDMETHODCALLTYPE GetState(DWORD* state)override{*state=DEVICE_STATE_ACTIVE;return S_OK;}
};
class ReplugDevices final : public Microsoft::WRL::RuntimeClass<Microsoft::WRL::RuntimeClassFlags<Microsoft::WRL::ClassicCom>,IMMDeviceCollection> {
  ComPtr<IMMDevice> device_;
 public:
  explicit ReplugDevices(ComPtr<IMMDevice> device):device_(std::move(device)){}
  HRESULT STDMETHODCALLTYPE GetCount(UINT* count)override{*count=1;return S_OK;}
  HRESULT STDMETHODCALLTYPE Item(UINT index,IMMDevice** value)override{if(index)return E_INVALIDARG;return device_.CopyTo(value);}
};
class ReplugEnumerator final : public Microsoft::WRL::RuntimeClass<Microsoft::WRL::RuntimeClassFlags<Microsoft::WRL::ClassicCom>,IMMDeviceEnumerator> {
  std::shared_ptr<EndpointStats> stats_;ComPtr<IMMDevice> device_;
  std::mutex mutex_;ComPtr<IMMNotificationClient> callback_;
 public:
  explicit ReplugEnumerator(std::shared_ptr<EndpointStats> stats):stats_(std::move(stats)),device_(Microsoft::WRL::Make<ReplugDevice>(stats_)){}
  HRESULT STDMETHODCALLTYPE EnumAudioEndpoints(EDataFlow,DWORD,IMMDeviceCollection** value)override{
    const auto call=++stats_->enumerations;
    if(call>1&&stats_->block_enumeration){
      const auto deadline=GetTickCount64()+5000;
      while(!stats_->release_enumeration&&GetTickCount64()<deadline)Sleep(1);
      if(!stats_->release_enumeration)return E_ABORT;
    }
    auto devices=Microsoft::WRL::Make<ReplugDevices>(device_);return devices.CopyTo(value);
  }
  HRESULT STDMETHODCALLTYPE GetDefaultAudioEndpoint(EDataFlow,ERole,IMMDevice** value)override{return device_.CopyTo(value);}
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
int main(){
  try{
    product::CaptureApartment apartment;
    enumeration_does_not_block_active_readers();
    same_id_reconnection_releases_old_render_client();
    microphone_reconnection_revokes_old_native_generation();
    std::cout<<"same-ID reconnect: fresh WASAPI clients, close before reopen, microphone revocation, owner-thread cleanup and healthy-stream preservation passed\n";
    return 0;
  }catch(const std::exception& error){std::cerr<<error.what()<<'\n';return 1;}
}
