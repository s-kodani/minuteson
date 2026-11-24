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
      auto packet=queue.pop();require(packet&&packet->sequence==p+1&&packet->time==p*75000&&packet->position==p*160,"packet order changed");
      for(std::uint64_t i=0;i<479;++i)require(packet->samples[i]==float(std::sin(double(p*479+i)*0.04)),"PCM changed at packet boundary");
    }
    bool failed=false;try{queue.pop();}catch(const std::runtime_error& error){failed=std::string(error.what())=="device disconnected";}
    require(failed,"acquisition failure disappeared or preceded queued PCM");
    product::PacketQueue bounded;
    for(unsigned i=0;i<10;++i){product::CapturePacket packet;packet.frames=96000;packet.flags=2;bounded.push(std::move(packet));}
    product::CapturePacket extra;extra.frames=1;extra.flags=2;
    bool overflow=false;try{bounded.push(std::move(extra));}catch(const product::InputOverload&){overflow=true;}
    require(overflow&&bounded.pop()->frames==96000,"overflow replaced already captured audio");
    { // Overload is recoverable per input, unlike storage or disk failures.
      product::PacketQueue probe;probe.fail(std::make_exception_ptr(product::InputOverload("overload")));
      bool recoverable=false;try{probe.pop();}catch(const product::InputDataFault&){recoverable=true;}
      require(recoverable,"queue overload must use the input recovery path");
    }
    product::PacketQueue bytes;
    for(unsigned i=0;i<8;++i){product::CapturePacket large;large.frames=65536;large.samples.resize(65536*8);bytes.push(std::move(large));}
    product::CapturePacket one;one.frames=1;one.samples.push_back(0.25f);
    overflow=false;try{bytes.push(std::move(one));}catch(const product::InputOverload&){overflow=true;}
    require(overflow&&bytes.pop()->samples.size()==65536*8,"byte overflow discarded accepted audio");
    product::CapturePacket released;released.frames=1;released.samples.push_back(0.5f);bytes.push(std::move(released));
    for(unsigned i=0;i<7;++i)bytes.pop();
    require(bytes.pop()->samples[0]==0.5f,"popped bytes did not release budget");
    auto pending=std::make_shared<product::StoragePending>();
    product::PacketQueue checked;checked.attach_storage(pending,0);
    for(float value:{NAN,INFINITY,-INFINITY,17.0f}){
      product::CapturePacket bad;bad.frames=1;bad.samples={value};bool rejected=false;
      try{checked.push(std::move(bad));}catch(const product::InputDataFault&){rejected=true;}
      require(rejected&&checked.pending_frames()==0&&pending->snapshot().frames[0]==0,"invalid PCM acquired storage credits");
    }
    product::CapturePacket valid;valid.frames=1;valid.samples={0.25f};checked.push(std::move(valid));
    const auto accepted=checked.pop();require(accepted->sequence==1&&accepted->samples[0]==0.25f&&pending->snapshot().frames[0]==1,"rejection poisoned later valid data");
    product::PacketQueue progress;progress.observe(1000);
    require(progress.observed_through(0)==1000,"empty acquisition observation missing");
    product::CapturePacket before;before.frames=1;before.flags=2;progress.push(std::move(before));
    progress.observe(2000);
    require(!progress.observed_through(1),"acquisition observation overtook unread PCM");
    const auto first=progress.pop();require(first->sequence==1&&!progress.observed_through(0)
      &&progress.observed_through(1)==2000,"observation did not follow its accepted sequence");
    product::CapturePacket after;after.frames=1;after.flags=2;progress.push(std::move(after));
    require(!progress.observed_through(1),"old observation overtook newly accepted PCM");
    progress.pop();require(!progress.observed_through(2),"old observation authorized an unobserved sequence");
    progress.observe(3000);require(progress.observed_through(2)==3000,"latest drained observation missing");
    std::cout<<"acquisition queue: delayed storage, exact continuous PCM, ordered drain before failure, and bounded overflow passed\n";
    return 0;
  }catch(const std::exception& error){std::cerr<<error.what()<<'\n';return 1;}
}
