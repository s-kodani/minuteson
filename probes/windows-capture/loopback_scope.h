#pragma once
#include <cstdint>

namespace product {
struct LoopbackTarget {std::uint32_t pid;bool exclude_tree;};
// Keep this internal activation target separate from the selected application
// identity: system capture must not turn microphone selection into app mode.
inline LoopbackTarget loopback_target(std::uint32_t selected_pid,std::uint32_t helper_pid) {
  return selected_pid?LoopbackTarget{selected_pid,false}:LoopbackTarget{helper_pid,true};
}
} // namespace product
