#pragma once

// Included inside namespace product. A history entry is emitted by an actual
// transition, never reconstructed by the UI's sampled status reader.
struct InputFailure : std::runtime_error {
  HRESULT result;
  InputFailure(HRESULT code,const char* operation):std::runtime_error(operation),result(code){}
};
void acquisition(HRESULT result,const char* operation){if(FAILED(result))throw InputFailure(result,operation);}
struct Binding {
  std::string state="reconnecting";Json reason=nullptr;
  std::uint64_t epoch=0,interruptions=0,sequence=0,next_attempt=0;
  Json events=Json::array();
  Json status() const {return Json{{"state",state},{"sourceEpoch",epoch},{"interruptions",interruptions},{"reason",reason}};}
  Json history() const {return Json{{"lastSequence",sequence},{"droppedThrough",sequence>64?sequence-64:0},{"events",events}};}
  void event(const char* kind){if(events.size()==64)events.erase(events.begin());events.push_back(Json{{"sequence",++sequence},{"kind",kind},{"status",status()}});}
  void lost(const char* why,bool unavailable=false){
    state=unavailable?"unavailable":"disconnected";reason=why;
    if(sequence%2==0){++interruptions;event("interrupted");}
    next_attempt=GetTickCount64()+1000;
  }
  void connected(){epoch=std::max(epoch+1,interruptions+1);state="connected";reason=nullptr;if(sequence%2)event("recovered");}
  void stopped(){state="stopped";reason=nullptr;}
};
struct IdleSleepGuard {
  IdleSleepGuard(){if(!SetThreadExecutionState(ES_CONTINUOUS|ES_SYSTEM_REQUIRED))throw std::runtime_error("録音中のスリープ抑止を開始できません");}
  ~IdleSleepGuard(){SetThreadExecutionState(ES_CONTINUOUS);}
};
struct ProcessFamily {std::set<DWORD> ids;std::vector<std::unique_ptr<Handle>> pins;};
ProcessFamily application_family(DWORD target){
  ProcessFamily family;std::map<DWORD,std::uint64_t> births;
  auto pin=[&](DWORD pid,std::uint64_t earliest){auto process=std::make_unique<Handle>(OpenProcess(SYNCHRONIZE|PROCESS_QUERY_LIMITED_INFORMATION,FALSE,pid));FILETIME created{},ended{},kernel{},user{};if(!process->value||WaitForSingleObject(process->value,0)!=WAIT_TIMEOUT||!GetProcessTimes(process->value,&created,&ended,&kernel,&user)||ticks(created)<earliest)return false;births[pid]=ticks(created);family.ids.insert(pid);family.pins.push_back(std::move(process));return true;};
  if(!pin(target,0))return family;
  Handle snapshot(CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS,0));
  if(snapshot.value==INVALID_HANDLE_VALUE)throw std::runtime_error("application process metadata unavailable");
  PROCESSENTRY32W item{};item.dwSize=sizeof(item);std::map<DWORD,DWORD> parents;
  if(Process32FirstW(snapshot.value,&item)){do{parents[item.th32ProcessID]=item.th32ParentProcessID;}while(Process32NextW(snapshot.value,&item));}
  for(std::size_t pass=0;pass<parents.size();++pass){bool changed=false;for(auto [pid,parent]:parents)if(!family.ids.count(pid)&&family.ids.count(parent))changed|=pin(pid,births.at(parent));if(!changed)break;}
  return family;
}
struct MicrophoneChoice {std::string uid,source="systemDefault",fallback="mon_direct_recording";};
MicrophoneChoice choose_microphone(const std::string& fixed,DWORD application){
  if(!fixed.empty())return {fixed,"fixed",""};
  auto e=enumerator();std::set<std::string> candidates;std::string fallback=application?"application_input_unavailable":"mon_direct_recording";
  if(application){try{
    auto family=application_family(application);ComPtr<IMMDeviceCollection> devices;check(e->EnumAudioEndpoints(eCapture,DEVICE_STATE_ACTIVE,&devices),"enumerate application microphones");UINT count=0;check(devices->GetCount(&count),"count application microphones");
    for(UINT i=0;i<count;++i){ComPtr<IMMDevice> device;check(devices->Item(i,&device),"read application microphone");ComPtr<IAudioSessionManager2> manager;
      if(FAILED(device->Activate(__uuidof(IAudioSessionManager2),CLSCTX_ALL,nullptr,reinterpret_cast<void**>(manager.GetAddressOf()))))continue;
      ComPtr<IAudioSessionEnumerator> sessions;check(manager->GetSessionEnumerator(&sessions),"enumerate input sessions");int n=0;check(sessions->GetCount(&n),"count input sessions");
      for(int j=0;j<n;++j){ComPtr<IAudioSessionControl> session;ComPtr<IAudioSessionControl2> control;DWORD pid=0;AudioSessionState state;
        if(SUCCEEDED(sessions->GetSession(j,&session))&&SUCCEEDED(session.As(&control))&&SUCCEEDED(control->GetProcessId(&pid))&&SUCCEEDED(control->GetState(&state))&&state==AudioSessionStateActive&&family.ids.count(pid))candidates.insert(utf8(endpoint_id(device.Get())));
      }
    }
    if(candidates.size()==1)return {*candidates.begin(),"application",""};
    if(candidates.size()>1)fallback="application_input_ambiguous";
  }catch(...){fallback="application_input_unavailable";}}
  ComPtr<IMMDevice> device;acquisition(e->GetDefaultAudioEndpoint(eCapture,eCommunications,&device),"既定のマイクがありません");
  return {utf8(endpoint_id(device.Get())),"systemDefault",fallback};
}
