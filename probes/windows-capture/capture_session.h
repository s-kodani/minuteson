#pragma once
// Included inside namespace product. CLI owns parsing only; this session owns
// native stream lifetimes and delegates policy/commits to the Rust core.
struct CaptureSession {
  Settings s;
  explicit CaptureSession(Settings settings):s(std::move(settings)){}
  int run(){
  Json manifest,journal_persistence=nullptr;std::unique_ptr<MetadataWorker> metadata;std::unique_ptr<Lane> mic_lane,system_lane;
  IdleSleepGuard awake;
  auto diagnostics=[&](const char* state,const Json& error){
    if(mic_lane&&system_lane){auto diagnostic=Json{{"schemaVersion",1},{"ownerId",s.owner},{"state",state},{"error",error},{"qpcUnit","100ns"},{"sampleRate",48000},{"microphone",mic_lane->diagnostics()},{"system",system_lane->diagnostics()}};if(metadata)metadata->observe(L"capture-clock.json",std::move(diagnostic));else atomic_json(s.output/L"capture-clock.json",diagnostic,false);}
  };
  try {
    if(s.lease_stdin){
      auto handle=GetStdHandle(STD_INPUT_HANDLE);BY_HANDLE_FILE_INFORMATION info{};wchar_t path[32768];
      auto size=GetFinalPathNameByHandleW(handle,path,32768,0);
      if(!s.test||GetFileType(handle)!=FILE_TYPE_DISK||!GetFileInformationByHandle(handle,&info))throw std::runtime_error("録音前テストの永続所有ハンドルを取得できません");
      if(info.nNumberOfLinks!=1||(info.dwFileAttributes&(FILE_ATTRIBUTE_REPARSE_POINT|FILE_ATTRIBUTE_DIRECTORY))||info.nFileSizeHigh||info.nFileSizeLow||!size||size>=32768||fs::path(path).filename()!=wide("lease-"+s.owner))throw std::runtime_error("録音前テストの永続所有ファイルが一致しません");
      // GetFinalPathNameByHandle returns an extended path (\\?\), whereas
      // filesystem canonicalization may remove that prefix. Compare the held
      // parent directory identities instead of their textual spellings.
      auto parent_info=[](const fs::path& parent){
        Handle directory(CreateFileW(parent.c_str(),FILE_READ_ATTRIBUTES,FILE_SHARE_READ|FILE_SHARE_WRITE|FILE_SHARE_DELETE,nullptr,OPEN_EXISTING,FILE_FLAG_BACKUP_SEMANTICS|FILE_FLAG_OPEN_REPARSE_POINT,nullptr));
        BY_HANDLE_FILE_INFORMATION metadata{};
        if(directory.value==INVALID_HANDLE_VALUE||!GetFileInformationByHandle(directory.value,&metadata)||!(metadata.dwFileAttributes&FILE_ATTRIBUTE_DIRECTORY)||(metadata.dwFileAttributes&FILE_ATTRIBUTE_REPARSE_POINT))throw std::runtime_error("録音前テストの所有ディレクトリを確認できません");
        return metadata;
      };
      const auto actual=parent_info(fs::path(path).parent_path()),expected=parent_info(s.output.parent_path());
      if(actual.dwVolumeSerialNumber!=expected.dwVolumeSerialNumber||actual.nFileIndexHigh!=expected.nFileIndexHigh||actual.nFileIndexLow!=expected.nFileIndexLow||ticks(actual.ftCreationTime)!=ticks(expected.ftCreationTime))throw std::runtime_error("録音前テストの永続所有ディレクトリが一致しません");
    }
    auto attributes=GetFileAttributesW(s.output.c_str());if(attributes==INVALID_FILE_ATTRIBUTES||!(attributes&FILE_ATTRIBUTE_DIRECTORY)||(attributes&FILE_ATTRIBUTE_REPARSE_POINT)||!fs::is_empty(s.output))throw std::runtime_error("capture destination must be an empty ordinary directory");
    mic_lane=std::make_unique<Lane>(s.output/L"microphone.caf",1,s.test);system_lane=std::make_unique<Lane>(s.output/L"system.caf",2,s.test);auto& mic=*mic_lane;auto& system=*system_lane;
    mic.holding_for_window=system.holding_for_window=s.test;
    if(!s.test){mic.raw=std::make_unique<RawSegments>(s.output,s.owner,"microphone",1);system.raw=std::make_unique<RawSegments>(s.output,s.owner,"system",2);}
    manifest=Json{{"schemaVersion",4},{"captureMode",s.mode},{"backend","wasapi"},{"targetPIDs",s.pid?Json::array({s.pid}):Json::array()},{"targetApplicationIdentity",nullptr},{"microphoneDeviceUID",s.uid.empty()?Json(nullptr):Json(s.uid)},{"recordingOwnerId",s.owner},{"inputDiagnosticsState","reported"},{"state","starting"},{"errors",Json::array()}, {"microphone",mic.track(0)}, {"system",system.track(0)}};
    manifest["consoleWindowPresent"]=GetConsoleWindow()!=nullptr;
    manifest["clockDiagnosticsFile"]="capture-clock.json";
    // Every publication, including initialization failures, must be decodable
    // by the same strict reader used while the helper is still running.
    if(s.test)manifest["captureTest"]=Json{{"ownerId",s.owner},{"durationSeconds",5},{"windowStartNs",nullptr},{"windowEndNs",nullptr},{"firstSavedCaptureNs",nullptr},{"completed",false}};
    atomic_json(s.output/L"manifest.json",manifest);
    diagnostics("starting",nullptr);
    if(s.synthetic_startup_delay_ms)Sleep(s.synthetic_startup_delay_ms);
    Handle parent(OpenProcess(SYNCHRONIZE|PROCESS_QUERY_LIMITED_INFORMATION,FALSE,s.parent));if(!parent.value)throw std::runtime_error("capture parent unavailable");
    Handle application_process(s.pid?OpenProcess(SYNCHRONIZE|PROCESS_QUERY_LIMITED_INFORMATION,FALSE,s.pid):INVALID_HANDLE_VALUE);
    Json identity=nullptr;if(s.pid){if(!application_process.value||application_process.value==INVALID_HANDLE_VALUE||WaitForSingleObject(application_process.value,0)!=WAIT_TIMEOUT)throw std::runtime_error("capture application is unavailable");identity=process_identity(s.pid);if(Json::parse(s.identity)!=identity)throw std::runtime_error("capture application identity changed");}
    manifest["targetApplicationIdentity"]=identity;
    FILETIME wall;GetSystemTimePreciseAsFileTime(&wall);auto utc=(ticks(wall)-EPOCH)/10000;auto before=qpc();
    MicrophoneChoice choice{s.uid,"fixed",""};
    std::string selected=s.uid;
    auto reconnect=[&](Lane& lane,bool microphone){
      const auto now=GetTickCount64();
      if(lane.preparation){
        // Expiry wins over a result first observed after the deadline. Such a
        // result is never allowed to restart a timed-out or stopped input.
        if(lane.preparation_schedule.timed_out(now)){
          lane.cancel_preparation();lane.preparation_schedule.initial_resolved=true;lane.binding.lost("acquisition_timeout",true);
        }
        if(!lane.preparation_schedule.cancelled){
          if(!microphone&&s.pid){
            try{if(process_identity(s.pid)!=identity)throw InputFailure(E_FAIL,"selected application identity changed");}
            catch(...){lane.cancel_preparation();lane.binding.lost("application_unavailable",true);return;}
          }
          if(auto ready=lane.preparation->take()){
            if(microphone){
              choice=ready->choice;selected=choice.uid;
              // The manifest records the initial input, matching the stored
              // start configuration. Later switches belong to selection
              // status/history, not a rewrite of that initial identity.
              if(manifest["microphoneDeviceUID"].is_null()){
                manifest["microphoneDeviceUID"]=selected;atomic_json(s.output/L"manifest.json",manifest);
              }
            }
            lane.adopt(std::move(*ready));lane.preparation.reset();return;
          }
        }
        if(lane.preparation->finished()){
          auto error=lane.preparation->error();lane.preparation.reset();lane.preparation_schedule.finish(now);
          if(error&&!lane.preparation_schedule.cancelled){
            try{std::rethrow_exception(error);}
            catch(const InputFailure& failure){lane.binding.lost(failure.result==E_ACCESSDENIED?"permission_unavailable":microphone?"device_disconnected":s.pid?"application_unavailable":"restart_failed",failure.result==E_ACCESSDENIED||(!microphone&&s.pid));}
            catch(...){lane.binding.lost(microphone?"device_disconnected":s.pid?"application_unavailable":"restart_failed",!microphone&&s.pid);}
          }
          // Includes cleanup of a timed-out stream; never overlap activations.
          lane.binding.next_attempt=lane.preparation_schedule.retry_at;
        }
        return;
      }
      if(lane.started||now<lane.binding.next_attempt||!lane.preparation_schedule.may_start(now))return;
      lane.preparation_schedule.begin(now);
      lane.binding.began();
      // Capture values only: a cancelled preparation can outlive this run.
      lane.preparation=std::make_unique<NativePreparation>([microphone,uid=s.uid,pid=s.pid,identity](NativePreparation::Cancellation cancelled){return prepare_input(microphone,uid,pid,identity,cancelled);});
    };
    if(s.synthetic){if(selected.empty())selected="synthetic-microphone";choice={selected,s.uid.empty()?"systemDefault":"fixed",s.uid.empty()?"mon_direct_recording":""};mic.binding.connected();system.binding.connected();mic.preparation_schedule.initial_resolved=system.preparation_schedule.initial_resolved=true;}
    else{reconnect(mic,true);reconnect(system,false);}
    mon::capture::SyntheticDriver synthetic({s.scenario,s.synthetic_clock_overlap,s.synthetic_clock_cases,s.synthetic_clock_resampling,s.synthetic_clock_drift});
    std::uint64_t revision=0,selection_revision=0,last_diagnostics=0,reported_estimates=0;std::uint64_t start=s.test&&!s.synthetic?0:qpc();
    if(s.test&&start){mic.holding_for_window=system.holding_for_window=false;mic.window_start=system.window_start=start;mic.window_end=system.window_end=start+50000000ULL;}
    manifest["state"]=s.test&&!start?"starting":"recording";manifest["microphoneDeviceUID"]=selected.empty()?Json(nullptr):Json(selected);
    if(s.test&&start)manifest["captureTest"]=Json{{"ownerId",s.owner},{"durationSeconds",5},{"windowStartNs",start*100},{"windowEndNs",start*100+5000000000ULL},{"firstSavedCaptureNs",nullptr},{"completed",false}};
    atomic_json(s.output/L"manifest.json",manifest);
    metadata=std::make_unique<MetadataWorker>(s.output,s.owner);
    auto selection=[&]{auto value=microphone_selection_status(s.owner,revision,selection_revision,s.uid,selected,choice,!s.synthetic&&!mic.started);if(!value.is_null())metadata->observe(L"microphone-selection.status.json",std::move(value));};
    auto publish_status=[&](bool sealed){
      Json report={{"schemaVersion",3},{"ownerId",s.owner},{"revision",++revision},{"microphone",mic.binding.status()},{"system",system.binding.status()},{"microphoneHistory",mic.binding.history()},{"systemHistory",system.binding.history()}};
      metadata->publish(std::move(report),sealed);
    };
    using SelectionPreparation=InputPreparation<MicrophoneChoice,CaptureApartment>;
    std::unique_ptr<SelectionPreparation> selection_preparation;
    PreparationSchedule selection_schedule;
    std::uint64_t next_status=0;
    while(true){
      if(WaitForSingleObject(parent.value,0)!=WAIT_TIMEOUT)throw std::runtime_error("capture parent exited");
      if(fs::exists(s.output/L"stop.request"))break;
      // Writer latency is not a clock change or a WASAPI device failure.
      if(!s.synthetic&&s.pid&&system.started){try{if(process_identity(s.pid)!=identity)system.detach("application_unavailable",true);}catch(...){system.detach("application_unavailable",true);}}
      if(s.synthetic){
        const auto packet=synthetic.next(start,qpc());
        if(packet.microphone_lost)mic.binding.lost("device_disconnected",s.scenario=="system-only");
        if(packet.microphone_connected)mic.binding.connected();
        if(packet.system_lost)system.binding.lost("application_unavailable",true);
        if(packet.system_connected)system.binding.connected();
        std::vector<float> tone(packet.frames,0.125f);
        if(mic.binding.state=="connected")mic.append(tone.data(),packet.frames,packet.time,packet.position,packet.flags,packet.arrival);
        std::vector<float> silence(packet.frames*2,0);
        if(system.binding.state=="connected")system.append(silence.data(),packet.frames,packet.time,packet.position,packet.flags,packet.arrival);
      }
      else{
        // Endpoint/session enumeration can block too. Poll it independently of
        // PCM writes and stop requests, with one outstanding query and no Lane
        // references in its worker. An obsolete query cannot select a device.
        const auto selection_now=GetTickCount64();
        if(selection_preparation){
          if(!s.uid.empty()||!mic.started||selection_schedule.timed_out(selection_now)){selection_preparation->cancel();selection_schedule.cancel();}
          if(!selection_schedule.cancelled){
            if(auto next=selection_preparation->take()){
              selection_preparation.reset();selection_schedule.accept();selection_schedule.retry_at=selection_now+PreparationSchedule::retry_ms;
              if(next->uid!=selected)mic.detach("device_changed");choice=std::move(*next);
            }
          }
          if(selection_preparation&&selection_preparation->finished()){
            auto error=selection_preparation->error();selection_preparation.reset();selection_schedule.finish(selection_now);
            if(error&&!selection_schedule.cancelled&&mic.started)mic.detach("device_disconnected");
          }
        }
        if(s.uid.empty()&&mic.started&&!selection_preparation&&selection_schedule.may_start(selection_now)){
          selection_schedule.begin(selection_now);
          selection_preparation=std::make_unique<SelectionPreparation>([uid=s.uid,pid=s.pid,identity](auto){return current_microphone_choice(uid,pid,identity);});
        }
        for(auto lane:{&mic,&system})if(lane->started){try{lane->pump();}catch(const InputFailure& error){lane->detach(error.result==E_ACCESSDENIED?"permission_unavailable":lane==&mic?"device_disconnected":"stream_stopped");}}
        reconnect(mic,true);reconnect(system,false);
        if(s.test&&!start&&mic.preparation_schedule.initial_resolved&&system.preparation_schedule.initial_resolved){
          start=qpc();mic.holding_for_window=system.holding_for_window=false;
          mic.window_start=system.window_start=start;mic.window_end=system.window_end=start+50000000ULL;
          manifest["state"]="recording";manifest["microphoneDeviceUID"]=selected.empty()?Json(nullptr):Json(selected);
          manifest["captureTest"]=Json{{"ownerId",s.owner},{"durationSeconds",5},{"windowStartNs",start*100},{"windowEndNs",start*100+5000000000ULL},{"firstSavedCaptureNs",nullptr},{"completed",false}};
          atomic_json(s.output/L"manifest.json",manifest);
        }
      }
      const auto estimates=mic.clock.estimated_packets+system.clock.estimated_packets;
      if(!last_diagnostics||qpc()-last_diagnostics>=10000000||(estimates&&!reported_estimates)){
        ULARGE_INTEGER available{};if(!GetDiskFreeSpaceExW(s.output.c_str(),&available,nullptr,nullptr))throw std::runtime_error("保存先の空き容量を確認できません。確定済みの音声は保持しています");if(available.QuadPart<16ULL*1024*1024)throw std::runtime_error("保存先の空き容量が不足しています。確定済みの原音声は保持しています");
        diagnostics("recording",nullptr);last_diagnostics=qpc();reported_estimates=estimates;
      }
      if(GetTickCount64()>=next_status){
        next_status=GetTickCount64()+250;++revision;
        metadata->observe(L"input-levels.json",Json{{"schemaVersion",1},{"ownerId",s.owner},{"revision",revision},{"microphone",mic.level()},{"system",system.level()}});
        publish_status(false);selection();
      }
      auto request=s.output/L"microphone-selection.request";
      if(fs::exists(request)){
        auto bytes=selection_request(request);
        if(!mon_capture_valid_selection(reinterpret_cast<const std::uint8_t*>(bytes.data()),bytes.size(),
          reinterpret_cast<const std::uint8_t*>(s.owner.data()),s.owner.size()))throw std::runtime_error("invalid microphone selection policy");
        Json next=Json::parse(bytes);
        auto next_revision=next.at("revision").get<std::uint64_t>();if(next_revision>selection_revision){
          std::string uid=next.at("uid").is_null()?"":next.at("uid").get<std::string>();
          // A running lane keeps its file and timeline. Restart only its input.
          if(selection_preparation){selection_preparation->cancel();selection_schedule.cancel();}
          mic.detach("device_changed");s.uid=uid;selection_revision=next_revision;
          if(s.synthetic){selected=uid.empty()?"synthetic-microphone":uid;choice={selected,uid.empty()?"systemDefault":"fixed",uid.empty()?"mon_direct_recording":""};mic.binding.connected();}
          else{mic.binding.next_attempt=0;reconnect(mic,true);}publish_status(false);selection();
        }
      }
      if(fs::exists(s.output/L"stop.request")||(s.synthetic?synthetic.finished():s.test&&start&&qpc()-start>=50000000ULL))break;
      if(s.synthetic||(!mic.queued()&&!system.queued()))Sleep(10);
    }
    if(selection_preparation)selection_preparation->cancel();
    mic.close();auto mic_closed=std::max(qpc(),mic.clock.parts.empty()?0ULL:mic.clock.parts.back().last_end);system.close();auto closed=std::max(qpc(),system.clock.parts.empty()?0ULL:system.clock.parts.back().last_end);closed=std::max(closed,mic_closed);
    // Preserve the buffered tail even when an unreturned native preparation
    // prevents producer-closure proof. Only finish() may publish closed=true.
    if(mic.raw){mic.raw->seal();system.raw->seal();}
    if(!mic.acquisition_closed()||!system.acquisition_closed())throw std::runtime_error("音声入力の準備処理の終了を確認できません。確定済み原音声を保持しています。録音を回復してください");
    if(mic.raw){mic.raw->finish();system.raw->finish();}mic.binding.stopped();system.binding.stopped();publish_status(true);metadata->finish();journal_persistence=metadata->persistence();if(!metadata->error().empty()){manifest["inputDiagnosticsState"]="unconfirmed";manifest["inputEventJournalError"]=metadata->error();}metadata.reset();
    if(!mic.frames&&!system.frames)throw std::runtime_error("マイク・PC音声を取得できませんでした。Windowsのマイク権限と入力機器を確認してください");
    auto origin=mic.frames?(system.frames?std::min(mic.first,system.first):mic.first):system.first;
    manifest["microphone"]=mic.track(origin);manifest["system"]=system.track(origin);manifest["systemAudioUnreceived"]=system.frames==0;
    manifest["microphoneAudioUnreceived"]=mic.frames==0;
    if(mic.raw){manifest["microphone"]["rawSegments"]=mic.raw->status();manifest["system"]["rawSegments"]=system.raw->status();manifest["clockMappingSource"]="sealed_raw_v1";}
    auto uncertainty=std::max(1000000ULL,std::max(mic.clock.max_boundary_adjustment_ticks,system.clock.max_boundary_adjustment_ticks)*100);
    manifest["timingEstimated"]=(mic.clock.estimated_packets+system.clock.estimated_packets)!=0;
    // Estimated clocks cannot claim the normal 1 ms precision. Retain a
    // conservative whole-recording envelope and explicitly label its origin.
    manifest["captureClock"]=Json{{"domain","query_performance_counter"},{"originUtcMs",static_cast<std::int64_t>(utc)+(static_cast<std::int64_t>(origin)-static_cast<std::int64_t>(before))/10000},{"uncertaintyNs",manifest["timingEstimated"]==true?std::max(uncertainty,(closed-origin)*100+1000000):uncertainty}};
    manifest["captureEnd"]=Json{{"method","acquisition_closed"},{"microphoneClosedUs",(mic_closed-origin)/10},{"systemClosedUs",(closed-origin)/10},{"endUs",(closed-origin)/10}};
    manifest["state"]="saved";if(s.test){manifest["captureTest"]["completed"]=true;manifest["captureTest"]["firstSavedCaptureNs"]=origin*100;}
    if(!journal_persistence.is_null())manifest["inputEventJournalPersistence"]=journal_persistence;
    metadata.reset();diagnostics("saved",nullptr);atomic_json(s.output/L"manifest.json",manifest);return 0;
  }catch(const std::exception& error){
    try{diagnostics("failed",error.what());}catch(...){}
    if(!manifest.is_null()){
      manifest["inputDiagnosticsState"]="unconfirmed";manifest["inputEventJournalError"]="録音異常終了のため履歴の停止時封印を確認できません";
      if(mic_lane&&system_lane){auto origin=mic_lane->frames?mic_lane->first:system_lane->first;if(system_lane->frames)origin=std::min(origin,system_lane->first);manifest["microphone"]=mic_lane->track(origin);manifest["system"]=system_lane->track(origin);if(mic_lane->raw){manifest["microphone"]["rawSegments"]=mic_lane->raw->status();manifest["system"]["rawSegments"]=system_lane->raw->status();}}
      manifest["state"]="failed";manifest["errors"]=Json::array({error.what()});try{atomic_json(s.output/L"manifest.json",manifest);}catch(...){}
    }std::cerr<<error.what()<<std::endl;return 1;
  }
}
};
