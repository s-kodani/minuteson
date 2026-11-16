#pragma once

// Loopback must not activate an idle Bluetooth hands-free output. Windows
// decides which profile carries playback; capture only follows that decision.
inline bool render_session_uses_output(bool active,bool own_process,bool shared_process=false){
  return active&&(!own_process||shared_process);
}
inline bool needs_render_loopback(bool is_default,std::optional<bool> playing,bool already_open){
  return is_default||playing.value_or(already_open);
}

template<class Devices,class Blocked,class ContainerOf>
void exclude_gated_render_endpoints(Devices& devices,const Blocked& blocked,ContainerOf container_of){
  if(blocked.empty())return;
  for(auto it=devices.begin();it!=devices.end();){
    if(blocked.count(container_of(it->first,it->second)))it=devices.erase(it);
    else ++it;
  }
}

template<class Streams,class Devices,class Retire,class Open>
void reconcile_render_streams(Streams& streams,const Devices& devices,Retire retire,Open open){
  // Close retired profiles before starting a new one, preserving every queued
  // packet. Unchanged outputs keep their clients, clock and partial mixer block.
  for(auto it=streams.begin();it!=streams.end();){
    if(!devices.count(it->first)){retire(*it->second);it=streams.erase(it);}
    else ++it;
  }
  for(const auto& [id,device]:devices)if(!streams.count(id))streams.emplace(id,open(device));
}
