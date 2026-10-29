#include "packet_queue.h"
#include <cmath>
#include <iostream>
#include <thread>
void require(bool ok,const char* error){if(!ok)throw std::runtime_error(error);}
int main(){
  try {
    product::PacketQueue queue;
    // Acquisition completes while the filesystem consumer is deliberately
    // not running. Every sample and original timestamp must survive in order.
    std::thread producer([&]{for(std::uint64_t p=0;p<1000;++p){
      product::CapturePacket packet;packet.frames=479;packet.time=p*75000;packet.position=p*160;
      for(std::uint64_t i=0;i<479;++i)packet.samples.push_back(float(std::sin(double(p*479+i)*0.04)));
      queue.push(std::move(packet));
    }});
    producer.join();
    require(queue.peak()==479000,"queue lost buffered frames during storage delay");
    queue.fail(std::make_exception_ptr(std::runtime_error("device disconnected")));
    for(std::uint64_t p=0;p<1000;++p){
      auto packet=queue.pop();require(packet&&packet->time==p*75000&&packet->position==p*160,"packet order changed");
      for(std::uint64_t i=0;i<479;++i)require(packet->samples[i]==float(std::sin(double(p*479+i)*0.04)),"PCM changed at packet boundary");
    }
    bool failed=false;try{queue.pop();}catch(const std::runtime_error& error){failed=std::string(error.what())=="device disconnected";}
    require(failed,"acquisition failure disappeared or preceded queued PCM");
    product::PacketQueue bounded;
    product::CapturePacket maximum;maximum.frames=product::PacketQueue::max_frames;bounded.push(std::move(maximum));
    product::CapturePacket extra;extra.frames=1;
    bool overflow=false;try{bounded.push(std::move(extra));}catch(const std::runtime_error&){overflow=true;}
    require(overflow&&bounded.pop()->frames==product::PacketQueue::max_frames,"overflow replaced already captured audio");
    product::PacketQueue bytes;
    product::CapturePacket large;large.frames=1;large.samples.resize(16*1024*1024/sizeof(float));
    bytes.push(std::move(large));
    product::CapturePacket one;one.frames=1;one.samples.push_back(0.25f);
    overflow=false;try{bytes.push(std::move(one));}catch(const std::runtime_error&){overflow=true;}
    require(overflow&&bytes.pop()->samples.size()==16*1024*1024/sizeof(float),"byte overflow discarded accepted audio");
    product::CapturePacket released;released.frames=1;released.samples.push_back(0.5f);bytes.push(std::move(released));
    require(bytes.pop()->samples[0]==0.5f,"popped bytes did not release budget");
    std::cout<<"acquisition queue: delayed storage, exact continuous PCM, ordered drain before failure, and bounded overflow passed\n";
    return 0;
  }catch(const std::exception& error){std::cerr<<error.what()<<'\n';return 1;}
}
