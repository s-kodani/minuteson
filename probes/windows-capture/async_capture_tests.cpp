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
  std::atomic<unsigned> starts{0},stops{0};DWORD service_thread=0,start_thread=0,stop_thread=0,service_release_thread=0;
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
  HRESULT STDMETHODCALLTYPE Start() override{start_thread=GetCurrentThreadId();++starts;return S_OK;}
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
  std::atomic<unsigned> released{0},packet_limit{500};DWORD thread=0,service_release_thread=0;
  unsigned invalid_at=std::numeric_limits<unsigned>::max();
  explicit FakeCapture(bool fail):fail_(fail){}
  HRESULT STDMETHODCALLTYPE QueryInterface(REFIID,void** value) override{*value=nullptr;return E_NOINTERFACE;}
  ULONG STDMETHODCALLTYPE AddRef() override{return ++references_;}
  ULONG STDMETHODCALLTYPE Release() override{auto n=--references_;if(n==1&&thread)service_release_thread=GetCurrentThreadId();if(!n)delete this;return n;}
  HRESULT STDMETHODCALLTYPE GetNextPacketSize(UINT32* frames) override{same_thread();*frames=released<packet_limit?479:0;return !*frames&&fail_?AUDCLNT_E_DEVICE_INVALIDATED:S_OK;}
  HRESULT STDMETHODCALLTYPE GetBuffer(BYTE** data,UINT32* count,DWORD* flags,UINT64* position,UINT64* time) override {
    same_thread();auto p=released.load();*count=479;*position=p*160;*time=p*75000;
    *flags=p%17==0?AUDCLNT_BUFFERFLAGS_SILENT:0;
    for(unsigned i=0;i<479;++i){buffer_[2*i]=float(std::sin(double(p*479+i)*0.04));buffer_[2*i+1]=-buffer_[2*i];}
    if(p==invalid_at){*flags=0;buffer_[0]=NAN;}
    *data=(*flags&AUDCLNT_BUFFERFLAGS_SILENT)?nullptr:reinterpret_cast<BYTE*>(buffer_.data());return S_OK;
  }
  HRESULT STDMETHODCALLTYPE ReleaseBuffer(UINT32 count) override{same_thread();require(count==479,"buffer released with wrong frame count");std::fill(buffer_.begin(),buffer_.end(),NAN);++released;return S_OK;}
};
class QueuedReader final : public product::CaptureReader {
 public: void stop() override{}
};
void source_fault_preserves_both_recording_lanes(){
  const auto directory=fs::temp_directory_path()/(L"mon-input-fault-"+std::to_wstring(GetCurrentProcessId())+L"-"+std::to_wstring(product::qpc()));
  fs::create_directory(directory);
  struct Cleanup {fs::path path;~Cleanup(){fs::remove_all(path);}} cleanup{directory};
  auto pending=std::make_shared<product::StoragePending>();
  auto commits=std::make_shared<product::RawCommitWorker>(pending);
  product::Lane mic(directory/L"microphone.caf",1),system(directory/L"system.caf",2);
  constexpr auto owner="735027d4-a03e-43d3-830a-21e767dfaf88";
  mic.raw=std::make_unique<product::RawSegments>(directory,owner,"microphone",1,commits);
  system.raw=std::make_unique<product::RawSegments>(directory,owner,"system",2,commits);
  auto prepare=[](unsigned channels,std::uint64_t start,bool fault){
    product::PreparedInput ready;ready.matching_position_rate=true;auto reader=std::make_unique<QueuedReader>();
    for(unsigned i=0;i<2;++i){product::CapturePacket p;p.frames=480;p.samples.assign(480*channels,0.25f);p.time=start+i*100000;p.arrival=p.time;p.position=i*480;reader->queue.push(std::move(p));}
    if(fault)reader->queue.fail(std::make_exception_ptr(product::InputDataFault("invalid source data")));
    ready.reader=std::move(reader);return ready;
  };
  mic.adopt(prepare(1,100000000,true));system.adopt(prepare(2,100000000,false));
  bool rejected=false;try{mic.pump();}catch(const product::InputDataFault&){rejected=true;mic.input_fault();}
  require(rejected&&mic.binding.state=="unavailable"&&mic.binding.reason=="invalid_audio_data","input data error did not retire the affected binding");
  require(system.binding.state=="connected","microphone fault interrupted PC audio");
  system.pump();
  const auto retired_by=GetTickCount64()+5000;
  while(mic.retirement&&GetTickCount64()<retired_by){mic.poll_retirement();Sleep(1);}
  require(!mic.retirement,"source fault did not finish its owned retirement");
  mic.adopt(prepare(1,110000000,false));mic.pump();
  mic.close();system.close();mic.raw->finish();system.raw->finish();commits->wait();
  require(mic.raw->stored_frames()==640&&system.raw->stored_frames()==320,"reacquisition lost valid PCM or included the rejected packet");
  const auto state=pending->snapshot();require(state.frames[0]==0&&state.frames[1]==0&&state.bytes==0&&state.jobs==0,"source fault leaked pending storage credits");
}
struct StopGate {
  std::mutex mutex;std::condition_variable changed;bool entered=false,open=false;
  void wait(){std::unique_lock lock(mutex);entered=true;changed.notify_all();changed.wait(lock,[&]{return open;});}
  void release(){{std::lock_guard lock(mutex);open=true;}changed.notify_all();}
};
class HeldStopReader final : public product::CaptureReader {
  std::shared_ptr<StopGate> gate_;
 public: explicit HeldStopReader(std::shared_ptr<StopGate> gate):gate_(std::move(gate)){}
  void stop() override{gate_->wait();}
};
void bluetooth_stop_wait_cannot_block_pc_audio_or_status(){
  const auto directory=fs::temp_directory_path()/(L"mon-bt-stop-"+std::to_wstring(GetCurrentProcessId())+L"-"+std::to_wstring(product::qpc()));
  fs::create_directory(directory);
  struct Cleanup {fs::path path;~Cleanup(){fs::remove_all(path);}} cleanup{directory};
  constexpr auto owner="735027d4-a03e-43d3-830a-21e767dfaf88";
  auto pending=std::make_shared<product::StoragePending>();auto commits=std::make_shared<product::RawCommitWorker>(pending);
  product::Lane mic(directory/L"microphone.caf",1),pc(directory/L"system.caf",2);
  mic.raw=std::make_unique<product::RawSegments>(directory,owner,"microphone",1,commits);
  pc.raw=std::make_unique<product::RawSegments>(directory,owner,"system",2,commits);
  auto gate=std::make_shared<StopGate>();
  // A watchdog makes the old synchronous implementation fail instead of
  // hanging the test forever. No physical Bluetooth or WASAPI device opens.
  std::jthread watchdog([gate]{std::unique_lock lock(gate->mutex);gate->changed.wait_for(lock,std::chrono::seconds(5),[&]{return gate->open;});lock.unlock();gate->release();});
  struct Release {std::shared_ptr<StopGate> gate;~Release(){gate->release();}} release{gate};
  product::PreparedInput input;input.matching_position_rate=true;input.reader=std::make_unique<HeldStopReader>(gate);
  for(unsigned i=0;i<20;++i){product::CapturePacket first;first.frames=480;first.samples.assign(480,0.25f);first.time=100000000+i*100000ULL;first.arrival=first.time;first.position=i*480;input.reader->queue.push(std::move(first));}
  mic.adopt(std::move(input));
  pc.clock.begin_stream(true);pc.binding.connected();
  const auto started=GetTickCount64();mic.detach("device_changed");
  require(GetTickCount64()-started<1000,"Bluetooth Stop blocked input status/control thread");
  bool overlap=false;try{product::PreparedInput replacement;replacement.reader=std::make_unique<QueuedReader>();mic.adopt(std::move(replacement));}catch(const std::logic_error&){overlap=true;}
  require(overlap,"replacement input opened before old native stop completed");
  product::MetadataWorker metadata(directory,owner);std::vector<float> signal(960,0.25f);std::uint64_t revision=0;
  for(unsigned i=0;i<125;++i){
    mic.poll_retirement();pc.append(signal.data(),480,100000000+i*100000ULL,i*480,0,100000000+i*100000ULL);
    if(i%20==0)metadata.publish(product::Json{{"schemaVersion",3},{"ownerId",owner},{"revision",++revision},{"microphone",mic.binding.status()},{"system",pc.binding.status()},{"microphoneHistory",mic.binding.history()},{"systemHistory",pc.binding.history()}},false);
    Sleep(10);
  }
  {std::lock_guard lock(gate->mutex);require(gate->entered&&!gate->open,"native stop was not held during status publication");}
  metadata.finish();
  product::Json status;{std::ifstream file(directory/L"input-status.json");file>>status;}
  require(status["revision"]==revision&&status["system"]["state"]=="connected"&&status["microphone"]["state"]=="disconnected","pending Bluetooth retirement aged or falsified live PC status");
  // Virtual expiry proves stop cannot publish closure or start another Stop
  // while the original native call is still owned. Accepted PCM remains salvageable.
  mic.retirement_wait_expired=true;bool unproven=false;
  try{mic.close();}catch(const std::runtime_error&){unproven=true;}
  require(unproven&&!mic.acquisition_closed(),"pending native stop manufactured recording closure");
  mic.raw->drain();mic.raw->seal();commits->wait();
  require(mic.raw->stored_frames()==20*160&&mic.raw->status()["producerClosed"]==false,"pending stop lost the validated prefix or closed it falsely");
  gate->release();watchdog.join();
  const auto deadline=GetTickCount64()+5000;while(mic.retirement&&GetTickCount64()<deadline){mic.poll_retirement();Sleep(1);}
  require(!mic.retirement,"completed native stop remained owned");
  mic.close();pc.close();mic.raw->finish();pc.raw->finish();commits->wait();
  require(mic.raw->stored_frames()==20*160&&pc.raw->stored_frames()==125*160,"retirement lost accepted mic/PC samples");
  const auto remaining=pending->snapshot();require(!remaining.frames[0]&&!remaining.frames[1]&&!remaining.jobs&&!remaining.bytes,"native retirement leaked recording credits");
}
int main(){
  try {
    source_fault_preserves_both_recording_lanes();
    bluetooth_stop_wait_cannot_block_pc_audio_or_status();
    product::MicrophoneFlow flow(100);
    require(!flow.expired(3099)&&flow.expired(3100)&&!flow.received,"microphone first-packet deadline changed");
    flow.packet(3000);
    require(flow.received&&!flow.expired(5999)&&flow.expired(6000),"microphone packet watchdog did not use reception time");
    // Start returns S_OK but GetNextPacketSize stays successful and empty.
    // This is the real PowerConf failure: it must not become connected or
    // retain the Bluetooth capture client indefinitely.
    {
      ComPtr<IAudioCaptureClient> capture;auto* input=new FakeCapture(false);capture.Attach(input);input->packet_limit=0;
      ComPtr<IAudioClient> client;auto* device=new FakeClient(input);client.Attach(device);
      product::NativePreparation preparation([client](auto)mutable{product::PreparedInput value;value.reader=std::make_unique<product::AsyncCapture>(std::move(client),2,true,"no-first-packet");return value;});
      auto deadline=GetTickCount64()+10000;
      while(!device->starts&&GetTickCount64()<deadline)Sleep(1);
      require(device->starts&&!preparation.take(),"microphone connected before receiving its first packet");
      while(!preparation.finished()&&GetTickCount64()<deadline)Sleep(1);
      require(preparation.finished()&&!preparation.take()&&device->stops==1,"empty microphone was not stopped and released");
      bool timed_out=false;try{if(preparation.error())std::rethrow_exception(preparation.error());}catch(const product::InputFailure& error){timed_out=error.result==HRESULT_FROM_WIN32(ERROR_TIMEOUT);}
      require(timed_out&&input->service_release_thread==input->thread&&device->service_release_thread==input->thread,"stalled startup hid failure or released COM on another thread");
    }
    {
      ComPtr<IAudioCaptureClient> capture;auto* input=new FakeCapture(false);capture.Attach(input);input->packet_limit=1;
      ComPtr<IAudioClient> client;auto* device=new FakeClient(input);client.Attach(device);
      product::AsyncCapture reader(client,2,true,"stalled-after-silence");
      auto first=reader.queue.pop();
      require(first&&first->frames==479&&(first->flags&AUDCLNT_BUFFERFLAGS_SILENT),"silent microphone packet was mistaken for missing delivery");
      auto deadline=GetTickCount64()+10000;while(!device->stops&&GetTickCount64()<deadline)Sleep(1);
      reader.stop();bool timed_out=false;try{reader.queue.pop();}catch(const product::InputFailure& error){timed_out=error.result==HRESULT_FROM_WIN32(ERROR_TIMEOUT);}
      require(timed_out&&device->stops==1,"microphone delivery stall did not invalidate and close capture");
    }
    require(product::needs_render_loopback(true,false,false),"silent default output was not prepared");
    require(!product::needs_render_loopback(false,false,false),"idle Bluetooth output would be activated");
    require(product::needs_render_loopback(false,true,false),"application-routed output was not captured");
    require(product::needs_render_loopback(false,std::nullopt,true),"metadata failure discarded a live output");
    require(!product::needs_render_loopback(false,std::nullopt,false),"metadata failure activated an unused profile");
    require(!product::render_session_uses_output(true,true),"loopback's own session kept retired Bluetooth output alive");
    require(product::render_session_uses_output(true,false)&&product::render_session_uses_output(true,true,true),"real or multi-process playback was omitted");
    require(!product::render_session_uses_output(false,false),"inactive playback was captured");
    struct Output {int identity;unsigned pending=7;};
    std::map<std::string,std::unique_ptr<Output>> outputs;
    outputs.emplace("keep",std::make_unique<Output>(Output{1}));
    outputs.emplace("old-profile",std::make_unique<Output>(Output{2}));
    auto retained=outputs.at("keep").get();unsigned rescued=0;std::vector<std::string> operations;
    product::reconcile_render_streams(outputs,std::map<std::string,int>{{"keep",1},{"new-profile",3}},
      [&](Output& output){rescued+=output.pending;operations.push_back("retire");},
      [&](int id){operations.push_back("open");return std::make_unique<Output>(Output{id});});
    require(outputs.size()==2&&outputs.at("keep").get()==retained&&rescued==7,"route change restarted a retained output or lost its old tail");
    require(operations==std::vector<std::string>{"retire","open"},"Bluetooth profiles were opened concurrently");
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
      product::EndpointCursor silent,signal;
      auto emit=[&](product::CapturePacket packet){
        meter.append(packet.samples.data(),packet.frames,packet.time,packet.position,packet.flags,packet.arrival);
      };
      mixer.add(silent,480000,480,nullptr,2,100100000,emit);
      std::vector<float> stereo(960,0.25f);mixer.add(signal,480000,480,stereo.data(),0,100100000,emit);
      mixer.flush(480480,100100000,emit);
      auto level=meter.level();meter.close();
      require(meter.frames==480&&meter.packets==1&&level["state"]=="measured"&&level["peak"]==0.25&&level["rms"]==0.25,"received endpoint audio did not reach the saved lane and PC meter");
    }
    fs::remove(meter_path);
    for(UINT32 channels:{1u,2u}){
      ComPtr<IAudioCaptureClient> capture;auto* input=new FakeCapture(false);capture.Attach(input);input->invalid_at=1;
      ComPtr<IAudioClient> client;auto* device=new FakeClient(input);client.Attach(device);
      product::AsyncCapture reader(client,channels);
      const auto deadline=GetTickCount64()+5000;
      while(input->released<2&&GetTickCount64()<deadline)Sleep(1);
      reader.stop();
      require(input->released==2&&device->stops>0,"invalid packet did not release native buffer and stop on owner thread");
      require(reader.queue.pop()->frames==479,"valid packet before input fault was lost");
      bool rejected=false;try{reader.queue.pop();}catch(const product::InputDataFault&){rejected=true;}
      require(rejected,"native input fault became a generic writer error");
    }
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
