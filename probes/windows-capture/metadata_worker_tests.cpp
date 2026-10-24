#include "json.hpp"
#include <atomic>
#include <condition_variable>
#include <filesystem>
#include <iostream>
#include <map>
#include <mutex>
#include <optional>
#include <thread>
#include <vector>
namespace product {
namespace fs=std::filesystem;
using Json=nlohmann::json;
std::mutex gate;std::condition_variable entered;bool blocked=false,release=false;
std::vector<std::pair<std::string,Json>> writes;
void atomic_json(const fs::path& path,const Json& value,bool durable=true){
  if(path.filename()=="input-events.json") {
    std::unique_lock lock(gate);blocked=true;entered.notify_one();entered.wait(lock,[]{return release;});
    if(!durable)throw std::runtime_error("journal lost durability");
  } else if(durable)throw std::runtime_error("telemetry still forces disk flush");
  writes.emplace_back(path.filename().string(),value);
}
#include "metadata_worker.h"
}
int main(){try{
  using namespace product;
  MetadataWorker worker(fs::path("unused"),"owner");
  worker.publish(Json{{"revision",1},{"events",Json::array()}},false);
  {std::unique_lock lock(gate);if(!entered.wait_for(lock,std::chrono::seconds(5),[]{return blocked;}))throw std::runtime_error("writer never started");}
  const auto before=std::chrono::steady_clock::now();
  for(int revision=2;revision<=100;++revision){worker.observe(L"input-levels.json",Json{{"revision",revision}});worker.publish(Json{{"revision",revision},{"events",Json::array({"interrupted","recovered"})}},revision==100);}
  if(std::chrono::steady_clock::now()-before>std::chrono::milliseconds(250))throw std::runtime_error("blocked journal stalled producer");
  // Simulate storage delay longer than the old false clock-change threshold.
  std::this_thread::sleep_for(std::chrono::milliseconds(3200));
  {std::lock_guard lock(gate);release=true;entered.notify_one();}
  worker.finish();
  if(worker.persistence()["committedRevision"]!=100||worker.persistence()["closed"]!=true||!worker.error().empty())throw std::runtime_error("final seal or commit acknowledgement was lost");
  unsigned journals=0;bool ordered=false;
  for(const auto& [name,value]:writes)if(name=="input-events.json"){++journals;if(value["sealed"]==true)ordered=value["report"]["events"]==Json::array({"interrupted","recovered"});}
  if(journals!=2||!ordered)throw std::runtime_error("coalescing lost ordered transitions or retained unbounded heartbeat backlog");
  writes.clear();
  MetadataWorker heartbeat(fs::path("unused"),"owner");heartbeat.publish(Json{{"revision",1},{"events",Json::array()}},false);
  const auto deadline=std::chrono::steady_clock::now()+std::chrono::seconds(5);
  while(heartbeat.persistence().is_null()&&std::chrono::steady_clock::now()<deadline)std::this_thread::sleep_for(std::chrono::milliseconds(1));
  std::this_thread::sleep_for(std::chrono::milliseconds(1300));heartbeat.finish();
  unsigned commits=0,diagnostics=0,bindings=0;
  for(const auto& [name,value]:writes){commits+=name=="input-events.json";diagnostics+=name=="input-journal-status.json";bindings+=name=="input-status.json";}
  if(commits!=1||diagnostics<4||bindings!=1)throw std::runtime_error("journal heartbeat either stalled or freshened stale binding telemetry");
  std::cout<<"metadata worker: independent producer, delayed durable journal, coalesced observations, ordered transitions and final seal passed\n";
  return 0;
}catch(const std::exception& error){std::cerr<<error.what()<<'\n';return 1;}}
