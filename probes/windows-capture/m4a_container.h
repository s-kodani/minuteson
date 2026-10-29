#pragma once
#include "../../crates/mon-capture-ffi/include/mon_capture.h"
#include <cstdint>
#include <filesystem>
#include <fstream>
#include <stdexcept>
#include <vector>

// Both native decoders use the same bounded Rust box walker. The callback
// borrows this already-open file only for validation and never retains it.
namespace m4a_container {
inline void validate(const std::filesystem::path& path) {
  std::ifstream input(path,std::ios::binary);
  if(!input)throw std::runtime_error("cannot open M4A");
  auto read=[](void* context,std::uint64_t offset,std::uint8_t* output,std::size_t count)->std::ptrdiff_t {
    auto& file=*static_cast<std::ifstream*>(context);
    file.clear();file.seekg(static_cast<std::streamoff>(offset));
    file.read(reinterpret_cast<char*>(output),static_cast<std::streamsize>(count));
    return file?static_cast<std::ptrdiff_t>(count):-1;
  };
  if(!mon_media_validate_m4a(std::filesystem::file_size(path),&input,read))
    throw std::runtime_error("invalid M4A audio/chapter track structure");
}
}
