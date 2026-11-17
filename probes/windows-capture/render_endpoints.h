#pragma once

// Loopback must not activate an idle Bluetooth hands-free output. Windows
// decides which profile carries playback; capture only follows that decision.
inline bool render_session_uses_output(bool active,bool own_process,bool shared_process=false){
  return active&&(!own_process||shared_process);
}
inline bool needs_render_loopback(bool is_default,std::optional<bool> playing,bool already_open){
  return is_default||playing.value_or(already_open);
}

template<class Streams,class Devices,class Retire,class Open>
void reconcile_render_streams(Streams& streams,const Devices& devices,Retire retire,Open open,
    const std::set<typename Streams::key_type>& revoked={},bool revoke_all=false){
  // Close retired profiles before starting a new one, preserving every queued
  // packet. Unchanged outputs keep their clients, clock and partial mixer block.
  for(auto it=streams.begin();it!=streams.end();){
    if(revoke_all||revoked.count(it->first)||!devices.count(it->first)){retire(*it->second);it=streams.erase(it);}
    else ++it;
  }
  for(const auto& [id,device]:devices)if(!streams.count(id))streams.emplace(id,open(device));
}
