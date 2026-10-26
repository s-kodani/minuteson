#pragma once
#include <algorithm>
#include <cstdint>
#include <stdexcept>
#include <limits>
#include <string_view>
#include <vector>

namespace product {
struct ClockPart { std::uint64_t first, frames, start, last_end; };
struct PacketTime {
  std::uint64_t start, end;
  const char* source;
  const char* reason;
  bool boundary;
};

// GetBuffer reports the first frame's QPC timestamp in 100 ns units. A
// discontinuity invalidates correlation of device positions, not necessarily
// the order of these timestamps. Conversion can also produce overlapping
// nominal packet intervals. Recover a monotone sample map before raw commits;
// retain the original observations separately for diagnosis.
// https://learn.microsoft.com/windows/win32/api/audioclient/nf-audioclient-iaudiocaptureclient-getbuffer
class PacketClock {
 public:
  static constexpr std::size_t bounded_capacity=1024,recording_capacity=65536;
  explicit PacketClock(std::size_t capacity=bounded_capacity):capacity_(capacity) {
    if(!capacity_||capacity_>recording_capacity)throw std::invalid_argument("invalid recording clock capacity");
  }
  std::vector<ClockPart> parts;
  std::uint64_t boundary_adjustments=0, max_boundary_adjustment_ticks=0;
  std::uint64_t equal_timestamps=0, reversed_timestamps=0, timestamp_errors=0,
    device_fallbacks=0, frame_fallbacks=0, estimated_packets=0, stream_epoch=0;

  // Device positions are correlated only within one uninterrupted stream.
  // Once QPC becomes unreliable, retain a sample-driven clock until reopen;
  // accepting the next raw QPC again could introduce another jump or overlap.
  void begin_stream(bool matching_position_rate=true){have_observation_=false;estimated_=false;remainder_=0;matching_position_rate_=matching_position_rate;++stream_epoch;}
  PacketTime observe(std::uint32_t frames,std::uint64_t raw,
                     std::uint64_t position,std::uint32_t flags,std::uint64_t arrival) {
    const bool discontinuity=(flags&1)!=0, timestamp_error=(flags&4)!=0;
    const bool equal=have_observation_&&raw==previous_raw_;
    const bool reversed=have_observation_&&raw<previous_raw_;
    equal_timestamps+=equal;reversed_timestamps+=reversed;timestamp_errors+=timestamp_error;
    // Converted WASAPI packets may overlap even while their starts increase.
    // A raw segment can be sealed in the middle of such a packet, so later
    // timestamps must never move behind its already committed fragment.
    const bool overlap=have_observation_&&raw<previous_end_;
    const bool unusable=timestamp_error||!raw||equal||reversed||overlap||raw>arrival
      ||(!have_observation_&&!parts.empty()&&raw<parts.back().last_end);
    const char* reason=timestamp_error?"timestamp_error":!raw?"zero_timestamp":equal?"equal_timestamp":reversed?"reversed_timestamp":raw>arrival?"future_timestamp":overlap?"overlapping_timestamp":unusable?"stream_restart_overlap":"none";
    const bool entering=unusable&&!estimated_;estimated_|=unusable;
    std::uint64_t time=raw;
    bool recovered_gap=false;
    const char* source="wasapi_qpc";
    if(estimated_) {
      ++estimated_packets;
      if(have_observation_) {
        time=previous_end_;
        if(matching_position_rate_&&!discontinuity&&position>=previous_position_
          &&position-previous_position_>=previous_frames_) {
          const auto missing=position-previous_position_-previous_frames_;
          // A corrupted device counter must not generate an unbounded gap.
          // Larger/unknown gaps remain explicitly estimated in diagnostics.
          if(missing<=48000ULL*60) {
            time=add(time,frame_ticks(missing));recovered_gap=missing!=0;source="device_position";++device_fallbacks;
          } else {source="sample_count";++frame_fallbacks;reason="device_position_jump";}
        } else {source="sample_count";++frame_fallbacks;if(std::string_view(reason)=="none")reason="device_position_uncorrelated";}
      } else {
        const auto duration=std::uint64_t(frames)*10000000/48000;
        time=arrival>=duration?arrival-duration:0;
        if(!parts.empty())time=std::max(time,parts.back().last_end);
        source="host_arrival";
      }
    }
    const auto end=add(time,frame_ticks(frames));
    previous_raw_=raw;previous_position_=position;previous_frames_=frames;
    previous_end_=end;have_observation_=true;
    return {time,end,source,reason,entering||recovered_gap||(!estimated_&&discontinuity)};
  }

  void append(std::uint64_t first_frame, std::uint32_t frames,
              std::uint64_t time, bool discontinuity) {
    if(!frames)return;
    if(have_packet_ && time<=last_packet_start_)
      throw std::runtime_error("recording packet start timestamp reversed");
    const auto end=time+std::uint64_t(frames)*10000000/48000;
    if(end<time)throw std::runtime_error("recording clock overflow");
    const bool boundary=parts.empty()||discontinuity||time>parts.back().last_end+200000;
    if(boundary) {
      if(parts.size()>=capacity_)throw std::runtime_error("too many recording clock discontinuities");
      if(!parts.empty()&&time<parts.back().last_end) {
        // Close the preceding affine map at the newly observed first frame.
        // This retains every captured sample, introduces no overlap or invented
        // silence, and leaves positive gaps intact. Reject an implausible fit.
        const auto& previous=parts.back();
        const auto duration=time-previous.start;
        const auto nominal=static_cast<long double>(previous.frames)*10000000/48000;
        if(!duration||nominal>static_cast<long double>(duration)*64
          ||static_cast<long double>(duration)>nominal*64)
          throw std::runtime_error("recording clock boundary cannot map captured frames");
        const auto adjustment=previous.last_end-time;
        ++boundary_adjustments;
        max_boundary_adjustment_ticks=std::max(max_boundary_adjustment_ticks,adjustment);
        parts.back().last_end=time;
      }
      parts.push_back({first_frame,0,time,end});
    }
    parts.back().frames+=frames;
    parts.back().last_end=end;
    last_packet_start_=time;
    have_packet_=true;
  }
 private:
  std::size_t capacity_;
  bool have_packet_=false;
  std::uint64_t last_packet_start_=0;
  bool have_observation_=false,estimated_=false,matching_position_rate_=true;
  std::uint64_t previous_raw_=0,previous_position_=0,previous_frames_=0,previous_end_=0,remainder_=0;
  static std::uint64_t add(std::uint64_t a,std::uint64_t b) {
    if(b>std::numeric_limits<std::uint64_t>::max()-a)throw std::runtime_error("recording clock overflow");
    return a+b;
  }
  std::uint64_t frame_ticks(std::uint64_t frames) {
    if(frames>(std::numeric_limits<std::uint64_t>::max()-remainder_)/10000000)
      throw std::runtime_error("recording frame clock overflow");
    const auto numerator=frames*10000000+remainder_;
    remainder_=numerator%48000;return numerator/48000;
  }
};
} // namespace product
