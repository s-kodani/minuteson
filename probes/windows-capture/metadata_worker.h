#pragma once

// Only observations are coalesced. Each report carries the producer's ordered,
// bounded transition history (including droppedThrough), not sampled events.
// Durable journal commits and transient telemetry never block PCM writing.
class MetadataWorker {
  fs::path output_;
  std::string owner_;
  std::mutex mutex_;
  std::condition_variable changed_;
  std::map<std::wstring, Json> pending_;
  std::optional<Json> report_;
  Json persistence_=nullptr;
  std::string error_;
  bool stopping_=false;
  std::thread thread_;
  void run() {
    std::uint64_t committed=0,diagnostic_revision=0;
    Json last_committed=nullptr;
    for(;;) {
      std::map<std::wstring,Json> files;
      std::optional<Json> report;
      {
        std::unique_lock lock(mutex_);
        changed_.wait_for(lock,std::chrono::milliseconds(250),[&]{return stopping_||report_||!pending_.empty();});
        if(!report_&&pending_.empty()&&stopping_)break;
        files.swap(pending_);report.swap(report_);
      }
      try {
        if(report) {
          const auto revision=report->at("report").at("revision").get<std::uint64_t>();
          Json content=*report;content["report"]["revision"]=0;
          if(content!=last_committed) {
            bool saved=false;
            try {atomic_json(output_/L"input-events.json",*report);committed=revision;saved=true;last_committed=content;}
            catch(const std::exception& error){std::lock_guard lock(mutex_);error_=error.what();}
            if(committed) {
              Json result={{"requestedRevision",revision},{"committedRevision",committed},{"state",saved?"confirmed":"failed"},{"closed",report->at("sealed")}};
              std::lock_guard lock(mutex_);persistence_=result;if(saved)error_.clear();
            }
          }
          atomic_json(output_/L"input-status.json",report->at("report"),false);
        }
        // This heartbeat certifies the last journal commit only. It never
        // refreshes old input levels or old binding/connectivity reports.
        Json result;{std::lock_guard lock(mutex_);result=persistence_;}
        if(!result.is_null())atomic_json(output_/L"input-journal-status.json",Json{{"schemaVersion",1},{"ownerId",owner_},{"revision",++diagnostic_revision},{"persistence",result}},false);
        for(const auto& [name,value]:files)atomic_json(output_/name,value,false);
      }catch(const std::exception& error){std::lock_guard lock(mutex_);error_=error.what();}
    }
  }
 public:
  MetadataWorker(fs::path output,std::string owner):output_(std::move(output)),owner_(std::move(owner)),thread_([this]{run();}){}
  MetadataWorker(const MetadataWorker&)=delete;
  ~MetadataWorker(){finish();}
  void publish(Json report,bool sealed){std::lock_guard lock(mutex_);report_=Json{{"schemaVersion",1},{"sealed",sealed},{"report",std::move(report)}};changed_.notify_one();}
  void observe(std::wstring name,Json value){std::lock_guard lock(mutex_);pending_.insert_or_assign(std::move(name),std::move(value));changed_.notify_one();}
  void finish(){ {std::lock_guard lock(mutex_);stopping_=true;changed_.notify_one();}if(thread_.joinable())thread_.join();}
  Json persistence(){std::lock_guard lock(mutex_);return persistence_;}
  std::string error(){std::lock_guard lock(mutex_);return error_;}
};
