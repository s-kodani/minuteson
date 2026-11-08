#pragma once
#include "../../crates/mon-capture-ffi/include/mon_capture.h"
#include <cstdint>
#include <exception>
#include <functional>
#include <stdexcept>
#include <vector>

// Both native import and recording borrow one bounded Rust converter per lane.
class VoicePCM {
  VoiceConverter* value_;
  struct Sink {
    std::function<void(const std::int16_t*,std::size_t,std::int64_t,std::int64_t)> write;
    std::exception_ptr error;
    static bool callback(void* context,const std::int16_t* pcm,std::size_t count,std::int64_t start,std::int64_t end) noexcept {
      auto& sink=*static_cast<Sink*>(context);
      try{sink.write(pcm,count,start,end);return true;}catch(...){sink.error=std::current_exception();return false;}
    }
  };
 public:
  VoicePCM(std::uint32_t rate,std::uint32_t channels):value_(mon_voice_new(rate,channels)){
    if(!value_)throw std::runtime_error("voice PCM converter unavailable");
  }
  VoicePCM(const VoicePCM&)=delete;VoicePCM& operator=(const VoicePCM&)=delete;
  ~VoicePCM(){mon_voice_free(value_);}
  template<class Write> void push(const float* pcm,std::size_t count,std::int64_t start,std::int64_t end,Write write){
    Sink sink{write,{}};const auto ok=mon_voice_push(value_,pcm,count,start,end,&sink,Sink::callback);
    if(sink.error)std::rethrow_exception(sink.error);if(!ok)throw std::runtime_error("voice PCM conversion failed");
  }
  template<class Write> void finish(Write write){
    Sink sink{write,{}};const auto ok=mon_voice_finish(value_,&sink,Sink::callback);
    if(sink.error)std::rethrow_exception(sink.error);if(!ok)throw std::runtime_error("voice PCM finalization failed");
  }
  static std::vector<std::uint8_t> header(std::uint64_t frames){
    std::vector<std::uint8_t> bytes(68);if(mon_voice_caf_header(frames,bytes.data(),bytes.size())!=68)throw std::runtime_error("invalid voice CAF geometry");return bytes;
  }
};
