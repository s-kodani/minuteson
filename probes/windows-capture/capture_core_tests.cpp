#include "../../crates/mon-capture-ffi/include/mon_capture.h"
#include <cstdint>
#include <iostream>
#include <stdexcept>
#include <string>
void require(bool ok,const char* message){if(!ok)throw std::runtime_error(message);}
int main(){
  try{
    require(mon_capture_abi_version()==1,"capture ABI differs");
    const auto metadata=mon_capture_limit(3)-mon_capture_limit(4);
    require(mon_capture_permits_commit(32767,metadata-1,65535,1,1),"last compatible prefix rejected");
    require(!mon_capture_permits_commit(32768,0,0,1,1),"unreadable segment prefix permitted");
    require(!mon_capture_permits_commit(0,metadata,0,1,1),"unreadable metadata prefix permitted");
    require(!mon_capture_permits_commit(0,0,65536,1,1),"unreadable clock prefix permitted");
    require(mon_capture_permits_pending(0,19999999999ULL,4095,1,1),"last pending unit rejected");
    require(!mon_capture_permits_pending(0,20000000000ULL,0,1,1),"time budget exceeded");
    require(!mon_capture_permits_pending(16777216,0,0,1,1),"byte budget exceeded");
    require(!mon_capture_permits_pending(0,0,4096,1,1),"packet budget exceeded");
    MonRawPrefix prefix=mon_capture_prefix_empty(),next{};
    const std::string receipt=R"({"schemaVersion":2,"ownerId":"735027d4-a03e-43d3-830a-21e767dfaf88","lane":"microphone","sequence":1,"audioName":"raw-microphone-00000001.caf","sourceFrameStart":0,"frames":1,"sampleRate":48000,"channels":1,"captureOriginNanoseconds":1000000,"clockSegments":[{"sourceFrameStart":0,"sourceFrameEnd":1,"timelineStartUs":0,"timelineEndUs":21,"mappingJson":"{\"schema_version\":1,\"method\":\"linear\"}"}],"clockNanoseconds":[{"startNanoseconds":1000000,"endNanoseconds":1021000}],"audioBytes":72,"audioSHA256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","previousEntrySHA256":null})";
    std::uint8_t tip[1024];
    require(mon_capture_prepare_commit(&prefix,reinterpret_cast<const std::uint8_t*>(receipt.data()),receipt.size(),&next,tip,sizeof(tip))>0,"first commit plan rejected");
    require(prefix.segments==0&&next.segments==1&&next.frames==1,"prepare mutated accepted prefix before IO");
    prefix=next;
    require(mon_capture_prepare_commit(&prefix,reinterpret_cast<const std::uint8_t*>(receipt.data()),receipt.size(),&next,tip,sizeof(tip))<0,"duplicate receipt advanced prefix twice");
    const std::string owner="735027d4-a03e-43d3-830a-21e767dfaf88";
    auto selection=[&](const std::string& value){return mon_capture_valid_selection(
      reinterpret_cast<const uint8_t*>(value.data()),value.size(),reinterpret_cast<const uint8_t*>(owner.data()),owner.size());};
    const auto base=std::string("{\"schemaVersion\":1,\"ownerId\":\"")+owner+"\",\"revision\":1,";
    require(selection(base+"\"mode\":\"automatic\",\"uid\":null}"),"automatic request rejected");
    require(selection(base+"\"mode\":\"fixed\",\"uid\":\"usb-1\"}"),"fixed request rejected");
    for(auto invalid:{"\"mode\":\"fixed\",\"uid\":null}","\"mode\":\"automatic\",\"uid\":\"usb\"}","\"mode\":\"fixed\",\"uid\":\"bad\\u0080uid\"}","\"mode\":\"automatic\",\"uid\":null,\"extra\":true}"})
      require(!selection(base+invalid),"invalid request accepted");
    MonCaptureLifecycle state{};
    require(mon_capture_lifecycle(&state,1)==1&&state.epoch==1,"first epoch lost");
    require(mon_capture_lifecycle(&state,2)==1&&state.phase==1,"connected state lost");
    require(mon_capture_lifecycle(&state,3)==1&&state.interruptions==1,"interruption lost");
    require(mon_capture_lifecycle(&state,4)==0&&state.interruptions==1,"duplicate loss counted twice");
    require(mon_capture_lifecycle(&state,1)==1&&state.phase==0&&state.epoch==2,"retry not reconnecting");
    mon_capture_lifecycle(&state,5);require(mon_capture_lifecycle(&state,2)<0,"late success reactivated stopped input");
    const std::string devices="[{\"uid\":\"z\",\"name\":\"Zoom\"},{\"uid\":\"a\",\"name\":\"alpha\"}]";
    uint32_t order[128];
    require(mon_capture_catalog_order(reinterpret_cast<const uint8_t*>(devices.data()),devices.size(),false,0,0,order,128)==2&&order[0]==1,"catalog order differs");
    std::cout<<"shared Rust capture ABI: commit, queue, requests, lifecycle and catalog contracts passed\n";
    return 0;
  }catch(const std::exception& error){std::cerr<<error.what()<<'\n';return 1;}
}
