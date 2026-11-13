// Compile the actual producer and its reader implementation with a renamed
// entry point. Only the fake COM devices below are instantiated by this test.
#define wmain unused_capture_entry
#include "main.cpp"
#undef wmain

void require(bool ok,const char* error){if(!ok)throw std::runtime_error(error);}
class FakeClient final : public IAudioClient {
  std::atomic<ULONG> references_{1};
  IAudioCaptureClient* capture_;
 public:
  std::atomic<unsigned> stops{0};DWORD service_thread=0,start_thread=0,stop_thread=0,service_release_thread=0;
  explicit FakeClient(IAudioCaptureClient* capture):capture_(capture){}
  HRESULT STDMETHODCALLTYPE QueryInterface(REFIID,void** value) override{*value=nullptr;return E_NOINTERFACE;}
  ULONG STDMETHODCALLTYPE AddRef() override{return ++references_;}
  ULONG STDMETHODCALLTYPE Release() override{auto n=--references_;if(n==1&&service_thread)service_release_thread=GetCurrentThreadId();if(!n)delete this;return n;}
  HRESULT STDMETHODCALLTYPE Initialize(AUDCLNT_SHAREMODE,DWORD,REFERENCE_TIME,REFERENCE_TIME,const WAVEFORMATEX*,LPCGUID) override{return E_NOTIMPL;}
  HRESULT STDMETHODCALLTYPE GetBufferSize(UINT32*) override{return E_NOTIMPL;}
  HRESULT STDMETHODCALLTYPE GetStreamLatency(REFERENCE_TIME*) override{return E_NOTIMPL;}
  HRESULT STDMETHODCALLTYPE GetCurrentPadding(UINT32*) override{return E_NOTIMPL;}
  HRESULT STDMETHODCALLTYPE IsFormatSupported(AUDCLNT_SHAREMODE,const WAVEFORMATEX*,WAVEFORMATEX**) override{return E_NOTIMPL;}
  HRESULT STDMETHODCALLTYPE GetMixFormat(WAVEFORMATEX**) override{return E_NOTIMPL;}
  HRESULT STDMETHODCALLTYPE GetDevicePeriod(REFERENCE_TIME*,REFERENCE_TIME*) override{return E_NOTIMPL;}
  HRESULT STDMETHODCALLTYPE Start() override{start_thread=GetCurrentThreadId();return S_OK;}
  HRESULT STDMETHODCALLTYPE Stop() override{stop_thread=GetCurrentThreadId();++stops;return S_OK;}
  HRESULT STDMETHODCALLTYPE Reset() override{return E_NOTIMPL;}
  HRESULT STDMETHODCALLTYPE SetEventHandle(HANDLE) override{return E_NOTIMPL;}
  HRESULT STDMETHODCALLTYPE GetService(REFIID iid,void** result) override{if(iid!=__uuidof(IAudioCaptureClient)){*result=nullptr;return E_NOINTERFACE;}service_thread=GetCurrentThreadId();capture_->AddRef();*result=capture_;return S_OK;}
};
class FakeCapture final : public IAudioCaptureClient {
  std::atomic<ULONG> references_{1};
  std::vector<float> buffer_=std::vector<float>(479*2);
  bool fail_;
  void same_thread(){auto id=GetCurrentThreadId();if(!thread)thread=id;require(thread==id,"GetBuffer/ReleaseBuffer changed acquisition threads");}
 public:
  std::atomic<unsigned> released{0};DWORD thread=0,service_release_thread=0;
  explicit FakeCapture(bool fail):fail_(fail){}
  HRESULT STDMETHODCALLTYPE QueryInterface(REFIID,void** value) override{*value=nullptr;return E_NOINTERFACE;}
  ULONG STDMETHODCALLTYPE AddRef() override{return ++references_;}
  ULONG STDMETHODCALLTYPE Release() override{auto n=--references_;if(n==1&&thread)service_release_thread=GetCurrentThreadId();if(!n)delete this;return n;}
  HRESULT STDMETHODCALLTYPE GetNextPacketSize(UINT32* frames) override{same_thread();*frames=released<500?479:0;return !*frames&&fail_?AUDCLNT_E_DEVICE_INVALIDATED:S_OK;}
  HRESULT STDMETHODCALLTYPE GetBuffer(BYTE** data,UINT32* count,DWORD* flags,UINT64* position,UINT64* time) override {
    same_thread();auto p=released.load();*count=479;*position=p*160;*time=p*75000;
    *flags=p%17==0?AUDCLNT_BUFFERFLAGS_SILENT:0;
    for(unsigned i=0;i<479;++i){buffer_[2*i]=float(std::sin(double(p*479+i)*0.04));buffer_[2*i+1]=-buffer_[2*i];}
    *data=(*flags&AUDCLNT_BUFFERFLAGS_SILENT)?nullptr:reinterpret_cast<BYTE*>(buffer_.data());return S_OK;
  }
  HRESULT STDMETHODCALLTYPE ReleaseBuffer(UINT32 count) override{same_thread();require(count==479,"buffer released with wrong frame count");std::fill(buffer_.begin(),buffer_.end(),NAN);++released;return S_OK;}
};
int main(){
  try {
    constexpr auto owner="735027d4-a03e-43d3-830a-21e767dfaf88";
    auto pending_fixed=product::microphone_selection_status(owner,2,1,"B","A",{"A","systemDefault","application_input_unavailable"},true);
    require(pending_fixed["deviceUID"]=="B"&&pending_fixed["mode"]=="fixed"&&pending_fixed["source"]=="pending"&&pending_fixed["fallbackReason"].is_null(),"new fixed request was acknowledged with stale automatic selection");
    auto pending_automatic=product::microphone_selection_status(owner,3,2,"","A",{"A","fixed",""},true);
    require(pending_automatic["mode"]=="automatic"&&pending_automatic["source"]=="pending"&&pending_automatic["fallbackReason"].is_null(),"pending automatic selection reused a fixed source");
    // The actual writer must discard pre-measurement audio even when the
    // asynchronous preparation has already filled its packet queue. This path
    // cannot create the deliberately invalid destination used below.
    product::Lane pending(fs::path(L"missing-parent")/L"not-created.caf",1,true);
    std::vector<float> preparation_audio(480,0.25f);
    pending.holding_for_window=true;
    pending.append(preparation_audio.data(),480,100000000,0,0,100100000);
    require(!pending.frames&&!pending.observed&&pending.clock.parts.empty(),"preparation audio entered the measured clock");
    pending.holding_for_window=false;pending.window_start=100200000;pending.window_end=150200000;
    pending.append(preparation_audio.data(),480,100000000,480,0,100300000);
    require(!pending.frames&&pending.file.value==INVALID_HANDLE_VALUE,"queued pre-window audio was saved");
    // A silent output device must not zero another output's signal on its
    // way through the real meter and saved-lane consumer.
    const auto meter_path=fs::temp_directory_path()/(L"mon-endpoint-meter-"+std::to_wstring(GetCurrentProcessId())+L"-"+std::to_wstring(product::qpc())+L".caf");
    {
      product::Lane meter(meter_path,2,true);meter.clock.begin_stream(true);
      product::EndpointMixer mixer;
      mixer.add(480000,480,nullptr,2);
      std::vector<float> stereo(960,0.25f);mixer.add(480000,480,stereo.data(),0);
      mixer.flush(480480,100100000,[&](product::CapturePacket packet){
        meter.append(packet.samples.data(),packet.frames,packet.time,packet.position,packet.flags,packet.arrival);
      });
      auto level=meter.level();meter.close();
      require(meter.frames==480&&meter.packets==1&&level["state"]=="measured"&&level["peak"]==0.25&&level["rms"]==0.25,"received endpoint audio did not reach the saved lane and PC meter");
    }
    fs::remove(meter_path);
    for(bool fail:{false,true}) {
      ComPtr<IAudioCaptureClient> capture;auto* input=new FakeCapture(fail);capture.Attach(input);
      ComPtr<IAudioClient> client;auto* device=new FakeClient(input);client.Attach(device);
      product::AsyncCapture reader(client,2);
      const auto deadline=GetTickCount64()+10000;
      while(input->released<500&&GetTickCount64()<deadline)Sleep(1);
      require(input->released==500,"storage delay prevented acquisition from draining the device");
      Sleep(3200); // longer than the removed three-second false clock-change threshold
      reader.stop();
      require(input->thread!=GetCurrentThreadId()&&device->stops>0,"reader was not independent or did not stop the device");
      require(device->service_thread==input->thread&&device->start_thread==input->thread&&device->stop_thread==input->thread&&input->service_release_thread==input->thread&&device->service_release_thread==input->thread,"WASAPI service acquisition, start, stop or release changed owning threads");
      for(unsigned p=0;p<500;++p){
        auto packet=reader.queue.pop();require(packet&&packet->frames==479&&packet->time==p*75000ULL&&packet->position==p*160ULL,"reader lost or reordered packets");
        if(p%17==0)require(packet->samples.empty()&&(packet->flags&AUDCLNT_BUFFERFLAGS_SILENT),"silent packet contract changed");
        else for(unsigned i=0;i<479;++i){auto value=float(std::sin(double(p*479+i)*0.04));require(packet->samples[2*i]==value&&packet->samples[2*i+1]==-value,"reader changed PCM or retained released device memory");}
      }
      bool invalidated=false;try{require(!reader.queue.pop(),"unexpected extra packet");}catch(const product::InputFailure& error){invalidated=error.result==AUDCLNT_E_DEVICE_INVALIDATED;}
      require(invalidated==fail,"reader hid device failure or discarded preceding audio");
    }
    std::cout<<"actual asynchronous reader: independent capture, stalled storage, same-thread buffer release, exact stereo PCM, silent buffers, stop and disconnection passed\n";
    return 0;
  }catch(const std::exception& error){std::cerr<<error.what()<<'\n';return 1;}
}
