#pragma once

// Notifications revoke a native endpoint lifetime, not just its position in
// the device list. Bluetooth can disappear and return with the very same ID
// between two enumeration passes. Callbacks never stop or reopen WASAPI.
class EndpointChanges final : public Microsoft::WRL::RuntimeClass<
  Microsoft::WRL::RuntimeClassFlags<Microsoft::WRL::ClassicCom>,IMMNotificationClient,Microsoft::WRL::FtmBase> {
  const std::wstring watched_;
  std::mutex mutex_;
  std::set<std::wstring> revoked_;
  std::atomic<bool> revoke_all_{false};
  void revoke(LPCWSTR id) noexcept {
    if(!watched_.empty()){
      if(id&&watched_==id)invalidated.store(true,std::memory_order_release);
    }else{
      try{
        std::lock_guard lock(mutex_);
        if(!id||revoked_.size()>=128)revoke_all_.store(true);
        else revoked_.emplace(id);
      }catch(...){revoke_all_.store(true);}
    }
    changed.store(true,std::memory_order_release);
  }
 public:
  std::atomic<bool> changed{false},invalidated{false};
  explicit EndpointChanges(std::wstring watched={}):watched_(std::move(watched)){}
  struct Revocations {
    bool all=false;
    std::set<std::wstring> endpoints;
  };
  Revocations take(){
    std::lock_guard lock(mutex_);Revocations result;
    result.all=revoke_all_.exchange(false);result.endpoints.swap(revoked_);return result;
  }
  HRESULT STDMETHODCALLTYPE OnDefaultDeviceChanged(EDataFlow flow,ERole,LPCWSTR) override{if(flow==eRender)changed.store(true);return S_OK;}
  HRESULT STDMETHODCALLTYPE OnDeviceAdded(LPCWSTR id) override{revoke(id);return S_OK;}
  HRESULT STDMETHODCALLTYPE OnDeviceRemoved(LPCWSTR id) override{revoke(id);return S_OK;}
  HRESULT STDMETHODCALLTYPE OnDeviceStateChanged(LPCWSTR id,DWORD) override{revoke(id);return S_OK;}
  HRESULT STDMETHODCALLTYPE OnPropertyValueChanged(LPCWSTR id,const PROPERTYKEY key) override{
    // Volume, friendly names and selection changes do not invalidate audio.
    // Engine-format changes do, including a Bluetooth profile reconfiguration.
    constexpr PROPERTYKEY oem_format{{0xe4870e26,0x3cc5,0x4cd2,{0xba,0x46,0xca,0x0a,0x9a,0x70,0xed,0x04}},3};
    if((key.pid==audio_inspection::engine_format.pid&&IsEqualGUID(key.fmtid,audio_inspection::engine_format.fmtid))||
       (key.pid==oem_format.pid&&IsEqualGUID(key.fmtid,oem_format.fmtid)))revoke(id);
    return S_OK;
  }
};

class EndpointLifetime {
  ComPtr<IMMDeviceEnumerator> devices_;
 public:
  ComPtr<EndpointChanges> changes;
  EndpointLifetime(ComPtr<IMMDeviceEnumerator> devices,std::wstring id={}):devices_(std::move(devices)),changes(Microsoft::WRL::Make<EndpointChanges>(std::move(id))){
    acquisition(devices_->RegisterEndpointNotificationCallback(changes.Get()),"monitor audio endpoint lifetime");
  }
  ~EndpointLifetime(){devices_->UnregisterEndpointNotificationCallback(changes.Get());}
  EndpointLifetime(const EndpointLifetime&)=delete;
  EndpointLifetime& operator=(const EndpointLifetime&)=delete;
  void require_current()const{
    if(changes->invalidated.load(std::memory_order_acquire))throw InputFailure(AUDCLNT_E_DEVICE_INVALIDATED,"audio endpoint lifetime changed");
  }
};
