// Compile the actual producer and its reader implementation with a renamed
// entry point. Only the fake COM devices below are instantiated by this test.
#define wmain unused_capture_entry
#include "main.cpp"
#undef wmain

void require(bool ok,const char* error){if(!ok)throw std::runtime_error(error);}
class FakeClient final : public IAudioClient {
  std::atomic<ULONG> references_{1};
 public:
  std::atomic<unsigned> stops{0};
  HRESULT STDMETHODCALLTYPE QueryInterface(REFIID,void** value) override{*value=nullptr;return E_NOINTERFACE;}
  ULONG STDMETHODCALLTYPE AddRef() override{return ++references_;}
  ULONG STDMETHODCALLTYPE Release() override{auto n=--references_;if(!n)delete this;return n;}
  HRESULT STDMETHODCALLTYPE Initialize(AUDCLNT_SHAREMODE,DWORD,REFERENCE_TIME,REFERENCE_TIME,const WAVEFORMATEX*,LPCGUID) override{return E_NOTIMPL;}
  HRESULT STDMETHODCALLTYPE GetBufferSize(UINT32*) override{return E_NOTIMPL;}
  HRESULT STDMETHODCALLTYPE GetStreamLatency(REFERENCE_TIME*) override{return E_NOTIMPL;}
  HRESULT STDMETHODCALLTYPE GetCurrentPadding(UINT32*) override{return E_NOTIMPL;}
  HRESULT STDMETHODCALLTYPE IsFormatSupported(AUDCLNT_SHAREMODE,const WAVEFORMATEX*,WAVEFORMATEX**) override{return E_NOTIMPL;}
  HRESULT STDMETHODCALLTYPE GetMixFormat(WAVEFORMATEX**) override{return E_NOTIMPL;}
  HRESULT STDMETHODCALLTYPE GetDevicePeriod(REFERENCE_TIME*,REFERENCE_TIME*) override{return E_NOTIMPL;}
  HRESULT STDMETHODCALLTYPE Start() override{return S_OK;}
  HRESULT STDMETHODCALLTYPE Stop() override{++stops;return S_OK;}
  HRESULT STDMETHODCALLTYPE Reset() override{return E_NOTIMPL;}
  HRESULT STDMETHODCALLTYPE SetEventHandle(HANDLE) override{return E_NOTIMPL;}
  HRESULT STDMETHODCALLTYPE GetService(REFIID,void**) override{return E_NOINTERFACE;}
};
class FakeCapture final : public IAudioCaptureClient {
  std::atomic<ULONG> references_{1};
  std::vector<float> buffer_=std::vector<float>(479*2);
  bool fail_;
  void same_thread(){auto id=GetCurrentThreadId();if(!thread)thread=id;require(thread==id,"GetBuffer/ReleaseBuffer changed acquisition threads");}
 public:
  std::atomic<unsigned> released{0};DWORD thread=0;
  explicit FakeCapture(bool fail):fail_(fail){}
  HRESULT STDMETHODCALLTYPE QueryInterface(REFIID,void** value) override{*value=nullptr;return E_NOINTERFACE;}
  ULONG STDMETHODCALLTYPE AddRef() override{return ++references_;}
  ULONG STDMETHODCALLTYPE Release() override{auto n=--references_;if(!n)delete this;return n;}
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
    for(bool fail:{false,true}) {
      ComPtr<IAudioClient> client;auto* device=new FakeClient;client.Attach(device);
      ComPtr<IAudioCaptureClient> capture;auto* input=new FakeCapture(fail);capture.Attach(input);
      product::AsyncCapture reader(client,capture,2);
      const auto deadline=GetTickCount64()+10000;
      while(input->released<500&&GetTickCount64()<deadline)Sleep(1);
      require(input->released==500,"storage delay prevented acquisition from draining the device");
      Sleep(3200); // longer than the removed three-second false clock-change threshold
      reader.stop();
      require(input->thread!=GetCurrentThreadId()&&device->stops>0,"reader was not independent or did not stop the device");
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
