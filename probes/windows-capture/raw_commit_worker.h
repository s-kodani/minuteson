#pragma once
// Included inside namespace product, after the platform durability functions.
struct RawCommitState {
  std::mutex mutex;
  MonRawPrefix prefix=mon_capture_prefix_empty();
  std::vector<ClockPart> clocks;
  bool closed=false;
  std::string failure;
};
struct RawCommitJob {
  fs::path directory;std::string lane,audio_name,receipt_name,receipt,tip;
  unsigned lane_index=0;
  std::vector<BYTE> body;
  std::vector<ClockPart> clocks;
  MonRawPrefix before{},after{};
  std::uint64_t source_end=0,charged_bytes=0;
  std::shared_ptr<RawCommitState> state;
  bool final=false;
  std::uint64_t bytes()const{return sizeof(*this)+body.capacity()+receipt.capacity()+tip.capacity()
    +lane.capacity()+audio_name.capacity()+receipt_name.capacity()+directory.native().capacity()*sizeof(wchar_t)
    +clocks.capacity()*sizeof(ClockPart);}
};
// One FIFO disk owner per recording avoids competing FlushFileBuffers calls.
// The control thread only constructs compact immutable jobs and reads snapshots.
class RawCommitWorker {
 public:
  using Hook=std::function<void(const char*,const RawCommitJob&)>;
 private:
  std::shared_ptr<StoragePending> pending_;
  Hook hook_;
  std::mutex mutex_;std::condition_variable changed_;
  std::array<std::deque<RawCommitJob>,2> jobs_;
  unsigned next_lane_=0;
  std::optional<RawCommitJob> failed_job_;
  std::exception_ptr error_;
  bool stopping_=false,working_=false;
  std::thread thread_;
  bool empty()const{return jobs_[0].empty()&&jobs_[1].empty();}
  void stage(const char* name,const RawCommitJob& job){if(hook_)hook_(name,job);}
  void persist(const RawCommitJob& job){
    {
      std::lock_guard lock(job.state->mutex);
      if(job.state->closed||job.state->prefix.segments!=job.before.segments
        ||job.state->prefix.frames!=job.before.frames
        ||std::memcmp(job.state->prefix.last_hash,job.before.last_hash,32))
        throw std::logic_error("recording commit order changed");
      // Complete potentially throwing bookkeeping allocation before publishing
      // a durable tip; its in-memory acknowledgement must then be infallible.
      job.state->clocks.reserve(job.state->clocks.size()+job.clocks.size());
    }
    if(!job.final){
      ULARGE_INTEGER available{};
      if(!GetDiskFreeSpaceExW(job.directory.c_str(),&available,nullptr,nullptr))throw std::runtime_error("保存先の空き容量を確認できません。確定済みの音声は保持しています");
      if(available.QuadPart<16ULL*1024*1024)throw std::runtime_error("保存先の空き容量が不足しています。確定済みの原音声は保持しています");
      stage("raw_create",job);
      // Deny replacement/writing until the PCM, receipt and tip are committed.
      Handle file(CreateFileW((job.directory/wide(job.audio_name)).c_str(),GENERIC_READ|GENERIC_WRITE,
        FILE_SHARE_READ,nullptr,CREATE_NEW,FILE_FLAG_WRITE_THROUGH,nullptr));
      if(file.value==INVALID_HANDLE_VALUE)throw std::runtime_error("原PCMの確定先を作成できません");
      stage("raw_write",job);write_all(file.value,job.body.data(),job.body.size());
      stage("raw_sync",job);if(!capture_flush(file.value,"raw_pcm_sync"))throw std::runtime_error("原PCMを同期できません");
      stage("raw_verify",job);verify_raw_pcm(file.value,job.body);
      stage("receipt",job);atomic_json(job.directory/wide(job.receipt_name),Json::parse(job.receipt));
      stage("tip",job);atomic_json(job.directory/wide("raw-"+job.lane+".json"),Json::parse(job.tip));
    }else{
      stage("closed_tip",job);atomic_json(job.directory/wide("raw-"+job.lane+".json"),Json::parse(job.tip));
    }
    {
      std::lock_guard lock(job.state->mutex);
      for(auto part:job.clocks){part.first+=job.before.frames;job.state->clocks.push_back(part);}
      job.state->prefix=job.after;job.state->closed=job.final;
    }
    pending_->committed(job.lane_index,job.source_end,job.charged_bytes);
  }
  void run(){
    for(;;){
      RawCommitJob job;
      {
        std::unique_lock lock(mutex_);changed_.wait(lock,[&]{return stopping_||!empty();});
        if(empty()){if(stopping_)return;continue;}
        const auto lane=jobs_[next_lane_].empty()?1-next_lane_:next_lane_;
        job=std::move(jobs_[lane].front());jobs_[lane].pop_front();next_lane_=1-lane;working_=true;
      }
      try{persist(job);}
      catch(const std::exception& error){
        auto failure=std::current_exception();
        {std::lock_guard lock(job.state->mutex);job.state->failure=error.what();}
        pending_->fail(failure);
        {std::lock_guard lock(mutex_);failed_job_=std::move(job);error_=failure;working_=false;changed_.notify_all();}
        return; // Accepted pending jobs remain owned until the recording stops.
      }
      {std::lock_guard lock(mutex_);working_=false;changed_.notify_all();}
    }
  }
 public:
  explicit RawCommitWorker(std::shared_ptr<StoragePending> pending,Hook hook={}):pending_(std::move(pending)),hook_(std::move(hook)),thread_([this]{run();}){}
  ~RawCommitWorker(){stop();}
  RawCommitWorker(const RawCommitWorker&)=delete;
  std::shared_ptr<StoragePending> pending()const{return pending_;}
  void enqueue(RawCommitJob job,bool next_active){
    std::lock_guard lock(mutex_);if(error_)std::rethrow_exception(error_);
    if(stopping_)throw std::logic_error("recording commit owner already stopped");
    job.charged_bytes=job.bytes();
    // Allocate the deque slot before transferring the assembler's reservation.
    auto& queue=jobs_.at(job.lane_index);queue.push_back(std::move(job));
    try{pending_->replace_active(queue.back().charged_bytes,next_active);}
    catch(...){queue.pop_back();throw;}
    changed_.notify_one();
  }
  void wait(){std::unique_lock lock(mutex_);changed_.wait(lock,[&]{return error_||(!working_&&empty());});if(error_)std::rethrow_exception(error_);}
  void check(){std::lock_guard lock(mutex_);if(error_)std::rethrow_exception(error_);}
  void stop(){{std::lock_guard lock(mutex_);stopping_=true;changed_.notify_all();}if(thread_.joinable())thread_.join();}
};
