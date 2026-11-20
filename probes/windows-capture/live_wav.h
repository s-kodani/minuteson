#pragma once
// Included inside namespace product after the native ownership/durability API.
// The session thread owns DSP; only this worker touches the recording file.
class LiveWav {
 public:
  using Hook=std::function<void(const char*,std::uint64_t)>;
 private:
  struct Job {
    std::vector<BYTE> pcm;
    std::string receipt,tip;
    MonWavPrefix after{};
    std::array<std::uint64_t,2> source_ends{};
    std::uint64_t charged_bytes=0,first=0;
    bool final=false;
    std::size_t bytes()const{return sizeof(*this)+pcm.capacity()+receipt.capacity()+tip.capacity();}
  };
  struct Credit {std::int64_t end;std::uint64_t source_end;};
  fs::path directory_;
  std::string owner_;
  std::unique_ptr<LiveMixer,decltype(&mon_wav_mixer_free)> mixer_{nullptr,mon_wav_mixer_free};
  std::shared_ptr<StoragePending> pending_;
  std::array<std::deque<Credit>,2> credits_;
  std::array<std::uint64_t,2> source_ends_{};
  std::array<std::uint64_t,2> sequence_{};
  std::array<std::int64_t,2> frontiers_{};
  MonWavPrefix planned_=mon_wav_prefix_empty(),durable_=mon_wav_prefix_empty();
  std::vector<BYTE> pcm_;
  std::vector<std::pair<std::uint64_t,std::uint64_t>> missing_;
  std::array<std::uint64_t,2> coverage_frames_{};
  std::int64_t origin_=0;
  bool has_origin_=false,active_=true,closed_=false;
  std::mutex mutex_;std::condition_variable changed_;
  std::deque<Job> jobs_;
  std::optional<Job> in_flight_;
  Hook hook_;
  bool stopping_=false,working_=false,ready_=false;
  std::exception_ptr error_;
  std::thread worker_;
  std::string tip(const MonWavPrefix& prefix,bool closed)const{
    std::uint8_t bytes[2048];const auto count=mon_wav_prefix_tip(&prefix,
      reinterpret_cast<const std::uint8_t*>(owner_.data()),owner_.size(),closed,bytes,sizeof(bytes));
    if(count<0)throw std::runtime_error("invalid WAV checkpoint identity");
    return std::string(reinterpret_cast<char*>(bytes),count);
  }
  static void seek(HANDLE file,std::uint64_t offset){
    LARGE_INTEGER position;position.QuadPart=static_cast<LONGLONG>(offset);
    if(!SetFilePointerEx(file,position,nullptr,FILE_BEGIN))throw std::runtime_error("WAV recording seek failed");
  }
  static void verify(HANDLE file,std::uint64_t offset,const std::vector<BYTE>& expected){
    seek(file,offset);std::array<BYTE,65536> bytes{};std::size_t used=0;
    while(used<expected.size()){
      DWORD count=0;const auto size=static_cast<DWORD>(std::min(bytes.size(),expected.size()-used));
      if(!ReadFile(file,bytes.data(),size,&count,nullptr)||count!=size
        ||!std::equal(bytes.begin(),bytes.begin()+count,expected.begin()+used))
        throw std::runtime_error("stored WAV PCM differs from accepted audio");
      used+=count;
    }
  }
  void stage(const char* name,const Job& job){if(hook_)hook_(name,job.after.frames);}
  void run(){
    try{
      // Deny directory replacement while path-based atomic metadata writes
      // accompany the pinned audio handle. Reparse points are never followed.
      Handle parent(CreateFileW(directory_.c_str(),FILE_READ_ATTRIBUTES|FILE_TRAVERSE,
        FILE_SHARE_READ|FILE_SHARE_WRITE,nullptr,OPEN_EXISTING,
        FILE_FLAG_BACKUP_SEMANTICS|FILE_FLAG_OPEN_REPARSE_POINT,nullptr));
      BY_HANDLE_FILE_INFORMATION directory_identity{};
      if(parent.value==INVALID_HANDLE_VALUE||!GetFileInformationByHandle(parent.value,&directory_identity)
        ||!(directory_identity.dwFileAttributes&FILE_ATTRIBUTE_DIRECTORY)
        ||(directory_identity.dwFileAttributes&FILE_ATTRIBUTE_REPARSE_POINT))
        throw std::runtime_error("録音先ディレクトリの所有を確認できません");
      Handle file(CreateFileW((directory_/L"recording.wav").c_str(),GENERIC_READ|GENERIC_WRITE,
        FILE_SHARE_READ,nullptr,CREATE_NEW,FILE_ATTRIBUTE_NORMAL,nullptr));
      if(file.value==INVALID_HANDLE_VALUE)throw std::runtime_error("録音WAVを新規作成できません");
      BY_HANDLE_FILE_INFORMATION identity{};
      if(!GetFileInformationByHandle(file.value,&identity)||identity.nNumberOfLinks!=1
        ||(identity.dwFileAttributes&(FILE_ATTRIBUTE_DIRECTORY|FILE_ATTRIBUTE_REPARSE_POINT)))
        throw std::runtime_error("録音WAVの所有を確認できません");
      const auto inode=(std::uint64_t(identity.nFileIndexHigh)<<32)|identity.nFileIndexLow;
      const auto birth=(ticks(identity.ftCreationTime)-EPOCH)*100;
      if(!capture_flush(file.value,"wav_reservation_sync"))throw std::runtime_error("録音WAVの予約を同期できません");
      atomic_json(directory_/L"single-wav-owner.json",Json{{"schemaVersion",1},{"ownerId",owner_},
        {"fileName","recording.wav"},{"device",identity.dwVolumeSerialNumber},{"inode",inode},{"birthNanoseconds",birth}});
      auto header=VoicePCM::wave_header(0);write_all(file.value,header.data(),header.size());
      if(!capture_flush(file.value,"wav_header_sync"))throw std::runtime_error("録音WAVのヘッダーを同期できません");
      atomic_json(directory_/L"single-wav.json",Json::parse(tip(durable_,false)));
      {std::lock_guard lock(mutex_);ready_=true;changed_.notify_all();}
      for(;;){
        {std::unique_lock lock(mutex_);changed_.wait(lock,[&]{return stopping_||!jobs_.empty();});
          if(jobs_.empty()){if(stopping_)return;continue;}in_flight_.emplace(std::move(jobs_.front()));jobs_.pop_front();working_=true;}
        auto& job=*in_flight_;
        if(!job.pcm.empty()){
          ULARGE_INTEGER available{};if(!GetDiskFreeSpaceExW(directory_.c_str(),&available,nullptr,nullptr)||available.QuadPart<16ULL*1024*1024)
            throw std::runtime_error("保存先の空き容量が不足しています。確定済みの音声は保持しています");
          stage("pcm_write",job);seek(file.value,80+job.first*2);write_all(file.value,job.pcm.data(),job.pcm.size());
          stage("pcm_sync",job);
          if(!capture_flush(file.value,"wav_pcm_sync"))throw std::runtime_error("録音WAVを同期できません");
          stage("pcm_verify",job);verify(file.value,80+job.first*2,job.pcm);
          std::ostringstream name;name<<"wav-"<<std::setw(8)<<std::setfill('0')<<job.after.checkpoints<<".json";
          stage("receipt",job);atomic_json(directory_/wide(name.str()),Json::parse(job.receipt));
        }
        if(job.final){
          stage("close",job);auto final_header=VoicePCM::wave_header(job.after.frames);seek(file.value,0);write_all(file.value,final_header.data(),final_header.size());
          if(!capture_flush(file.value,"wav_finalize_sync"))throw std::runtime_error("録音WAVの確定を同期できません");
        }
        BY_HANDLE_FILE_INFORMATION current{};
        if(!GetFileInformationByHandle(file.value,&current)||current.dwVolumeSerialNumber!=identity.dwVolumeSerialNumber
          ||current.nFileIndexHigh!=identity.nFileIndexHigh||current.nFileIndexLow!=identity.nFileIndexLow
          ||current.nNumberOfLinks!=1)throw std::runtime_error("録音WAVの実体が変更されました");
        stage("tip",job);atomic_json(directory_/L"single-wav.json",Json::parse(job.tip));
        pending_->committed_wav(job.source_ends,job.charged_bytes);
        {std::lock_guard lock(mutex_);durable_=job.after;in_flight_.reset();working_=false;changed_.notify_all();}
      }
    }catch(...){auto error=std::current_exception();pending_->fail(error);std::lock_guard lock(mutex_);error_=error;ready_=true;working_=false;changed_.notify_all();}
  }
  void enqueue(Job job,bool next_active){
    std::lock_guard lock(mutex_);if(error_)std::rethrow_exception(error_);
    if(stopping_)throw std::logic_error("WAV writer already stopped");
    job.charged_bytes=job.bytes();jobs_.push_back(std::move(job));
    try{pending_->replace_active(jobs_.back().charged_bytes,next_active);}catch(...){jobs_.pop_back();throw;}
    changed_.notify_one();
  }
  void seal(bool final=false){
    if(pcm_.empty()&&!final)return;
    Job job;job.first=planned_.frames;job.after=planned_;job.final=final;job.pcm=pcm_;
    if(!pcm_.empty()){
      auto previous=Json::parse(tip(planned_,false))["lastEntrySHA256"];
      Json missing=Json::array();for(const auto& gap:missing_)missing.push_back(Json::array({gap.first,gap.second}));
      Json entry={{"schemaVersion",1},{"ownerId",owner_},{"sequence",planned_.checkpoints+1},
        {"frameStart",planned_.frames},{"frames",pcm_.size()/2},{"captureOriginNanoseconds",origin_},
        {"pcmSHA256",sha256_bytes(pcm_.data(),pcm_.size())},{"previousEntrySHA256",previous},{"missingRanges",missing},
        {"coverageFrames",Json::array({coverage_frames_[0],coverage_frames_[1]})}};
      job.receipt=entry.dump();std::uint8_t bytes[2048];
      const auto count=mon_wav_prepare_commit(&planned_,reinterpret_cast<const std::uint8_t*>(job.receipt.data()),job.receipt.size(),&job.after,bytes,sizeof(bytes));
      if(count<0)throw std::runtime_error("録音WAVの保存範囲が不正です");
    }
    const auto end=origin_+static_cast<std::int64_t>(job.after.frames*1000000000ULL/16000);
    for(unsigned lane=0;lane<2;++lane)while(!credits_[lane].empty()&&credits_[lane].front().end<=end){source_ends_[lane]=credits_[lane].front().source_end;credits_[lane].pop_front();}
    job.source_ends=source_ends_;job.tip=tip(job.after,final);const auto after=job.after;
    enqueue(std::move(job),!final);planned_=after;pcm_.clear();missing_.clear();coverage_frames_={};if(final)active_=false;
  }
  struct Sink {LiveWav* self;std::exception_ptr error;
    static bool write(void* context,const std::int16_t* samples,std::size_t count,std::uint64_t first,std::int64_t origin,std::uint32_t coverage)noexcept{
      auto& sink=*static_cast<Sink*>(context);try{sink.self->mixed(samples,count,first,origin,coverage);return true;}catch(...){sink.error=std::current_exception();return false;}}
  };
  void mixed(const std::int16_t* samples,std::size_t count,std::uint64_t first,std::int64_t origin,std::uint32_t coverage){
    if(has_origin_&&origin_!=origin)throw std::logic_error("録音WAVの時刻原点が変更されました");
    origin_=origin;has_origin_=true;
    if(first!=planned_.frames+pcm_.size()/2)throw std::logic_error("録音WAVの音声順序が不正です");
    while(count){const auto n=std::min<std::size_t>(count,64000-pcm_.size()/2);
      for(unsigned lane=0;lane<2;++lane)if(coverage&(1u<<lane))coverage_frames_[lane]+=n;
      if(!coverage){const auto start=planned_.frames+pcm_.size()/2;if(!missing_.empty()&&missing_.back().second==start)missing_.back().second=start+n;else missing_.emplace_back(start,start+n);}
      const auto* bytes=reinterpret_cast<const BYTE*>(samples);pcm_.insert(pcm_.end(),bytes,bytes+n*2);samples+=n;count-=n;if(pcm_.size()==128000)seal();}
  }
 public:
  LiveWav(fs::path directory,std::string owner,std::shared_ptr<StoragePending> pending,Hook hook={})
    :directory_(std::move(directory)),owner_(std::move(owner)),mixer_(mon_wav_mixer_new(),mon_wav_mixer_free),pending_(std::move(pending)),hook_(std::move(hook)){
    if(!mixer_)throw std::runtime_error("WAV mixer unavailable");pending_->active();pcm_.reserve(128000);
    worker_=std::thread([this]{run();});std::unique_lock lock(mutex_);changed_.wait(lock,[&]{return ready_;});
    if(error_){auto error=error_;lock.unlock();stop();if(active_){pending_->abandon_active();active_=false;}mixer_.reset();std::rethrow_exception(error);}
  }
  ~LiveWav(){stop();if(active_)pending_->abandon_active();}
  LiveWav(const LiveWav&)=delete;
  std::shared_ptr<StoragePending> pending()const{return pending_;}
  void check(){std::lock_guard lock(mutex_);if(error_)std::rethrow_exception(error_);}
  void push(unsigned lane,std::uint64_t epoch,const std::int16_t* pcm,std::size_t count,std::int64_t start,std::int64_t end,std::uint64_t source_end){
    check();const auto sequence=sequence_.at(lane)+1;
    if(!mon_wav_mixer_push(mixer_.get(),lane,epoch,sequence,pcm,count,start,end))throw std::runtime_error("録音音声の時刻順序が不正です");
    sequence_[lane]=sequence;frontiers_[lane]=std::max(frontiers_[lane],end);credits_[lane].push_back({end,source_end});
  }
  void advance(unsigned lane,std::int64_t through){through=std::max(frontiers_.at(lane),through);if(!mon_wav_mixer_advance(mixer_.get(),lane,sequence_.at(lane),through))throw std::runtime_error("録音音声の確定順序が不正です");frontiers_[lane]=through;}
  void drain(std::int64_t through){check();Sink sink{this,{}};const auto ok=mon_wav_mixer_drain(mixer_.get(),through,&sink,Sink::write);if(sink.error)std::rethrow_exception(sink.error);if(!ok)throw std::runtime_error("録音音声の合成に失敗しました");}
  void finish(){if(closed_)return;Sink sink{this,{}};const auto ok=mon_wav_mixer_finish(mixer_.get(),&sink,Sink::write);if(sink.error)std::rethrow_exception(sink.error);if(!ok)throw std::runtime_error("録音音声の合成を終了できません");seal(true);wait();closed_=true;}
  void wait(){std::unique_lock lock(mutex_);changed_.wait(lock,[&]{return error_||(!working_&&jobs_.empty());});if(error_)std::rethrow_exception(error_);}
  void stop(){{std::lock_guard lock(mutex_);stopping_=true;changed_.notify_all();}if(worker_.joinable())worker_.join();}
  Json status(){std::lock_guard lock(mutex_);return Json::parse(tip(durable_,closed_));}
  std::int64_t origin()const{return has_origin_?origin_:0;}
};

// Per-input converter/clock; its normalized PCM is sent to the shared mixer.
class WavInput {
  fs::path path_;
  unsigned lane_;
  UINT32 channels_;
  std::shared_ptr<LiveWav> recording_;
  std::unique_ptr<VoicePCM> converter_;
  Handle test_file_;
  std::uint64_t epoch_=0,submitted_=0,source_base_=0,converted_=0;
  std::int64_t last_end_=0;
  bool boundary_=false;
  PacketClock clock_{PacketClock::recording_capacity,16000};
  void converted(const std::int16_t* pcm,std::size_t count,std::int64_t start,std::int64_t end){
    if(!count)return;clock_.append(frames,static_cast<std::uint32_t>(count),static_cast<std::uint64_t>(start)/100,boundary_,static_cast<std::uint64_t>(end)/100);boundary_=false;
    converted_+=count;const auto source_end=std::min(submitted_,source_base_+converted_*3);
    if(recording_)recording_->push(lane_,epoch_,pcm,count,start,end,source_end);
    else{
      if(test_file_.value==INVALID_HANDLE_VALUE){test_file_.value=CreateFileW(path_.c_str(),GENERIC_WRITE,FILE_SHARE_READ,nullptr,CREATE_NEW,FILE_ATTRIBUTE_NORMAL,nullptr);if(test_file_.value==INVALID_HANDLE_VALUE)throw std::runtime_error("テストWAVを新規作成できません");auto h=VoicePCM::wave_header(0);write_all(test_file_.value,h.data(),h.size());}
      write_all(test_file_.value,pcm,count*2);
    }
    frames+=count;last_end_=end;
  }
 public:
  std::uint64_t frames=0;
  WavInput(fs::path path,unsigned lane,UINT32 channels,std::shared_ptr<LiveWav> recording={}):path_(std::move(path)),lane_(lane),channels_(channels),recording_(std::move(recording)){}
  std::shared_ptr<StoragePending> pending()const{return recording_?recording_->pending():nullptr;}
  unsigned lane_index()const{return lane_;}
  void drain_converter(){if(converter_){converter_->finish([&](auto pcm,auto count,auto start,auto end){converted(pcm,count,start,end);});converter_.reset();}}
  void append(const float* pcm,UINT32 count,std::uint64_t time,bool boundary,std::uint64_t source_end){
    if(recording_&&!source_end)source_end=recording_->pending()->acquire(lane_,count);
    if(boundary){drain_converter();boundary_=true;++epoch_;}
    if(!converter_){converter_=std::make_unique<VoicePCM>(48000,channels_);source_base_=source_end>=count?source_end-count:0;converted_=0;}
    submitted_=source_end;
    converter_->push(pcm,std::size_t(count)*channels_,static_cast<std::int64_t>(time*100),static_cast<std::int64_t>(time*100+std::uint64_t(count)*1000000000/48000),
      [&](auto samples,auto n,auto start,auto end){converted(samples,n,start,end);});
  }
  void finish(){drain_converter();if(test_file_.value!=INVALID_HANDLE_VALUE){auto h=VoicePCM::wave_header(frames);LARGE_INTEGER zero{};if(!SetFilePointerEx(test_file_.value,zero,nullptr,FILE_BEGIN))throw std::runtime_error("テストWAVの確定位置が不正です");write_all(test_file_.value,h.data(),h.size());if(!capture_flush(test_file_.value,"test_wav_sync"))throw std::runtime_error("テストWAVを同期できません");CloseHandle(test_file_.value);test_file_.value=INVALID_HANDLE_VALUE;}}
  std::int64_t normalized_through()const{return last_end_;}
  Json timeline(std::uint64_t origin)const{Json result=Json::array();for(const auto& p:clock_.parts)result.push_back(Json{{"sourceFrameStart",p.first},{"sourceFrameEnd",p.first+p.frames},{"timelineStartUs",(p.start-origin)/10},{"timelineEndUs",(p.last_end-origin)/10},{"mappingJson","{\"schema_version\":1,\"method\":\"linear\"}"}});return result;}
};
