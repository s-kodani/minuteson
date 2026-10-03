#include "segmented_wave_writer.h"

#include <array>
#include <cstdint>
#include <filesystem>

int main(int argc, char** argv) {
  if (argc != 2) return 2;
  const std::filesystem::path directory(argv[1]);
  constexpr std::array<std::uint8_t, 16> stereo_pcm = {
      1, 0, 2, 0, 0x80, 0xbb, 0, 0, 0, 0xee, 2, 0, 4, 0, 16, 0};
  constexpr std::array<std::uint8_t, 8> frames = {1, 2, 3, 4, 5, 6, 7, 8};
  mon::SegmentedWaveWriter split(directory, "split", stereo_pcm.data(),
                                 stereo_pcm.size(), 10);
  split.append(frames.data(), frames.size(), false);
  split.append(nullptr, 4, true);
  split.finish();

  constexpr std::array<std::uint8_t, 16> mono_pcm = {
      1, 0, 1, 0, 0x40, 0x1f, 0, 0, 0x40, 0x1f, 0, 0, 1, 0, 8, 0};
  constexpr std::array<std::uint8_t, 1> one_frame = {0x7f};
  mon::SegmentedWaveWriter odd_data(directory, "odd-data", mono_pcm.data(),
                                    mono_pcm.size());
  odd_data.append(one_frame.data(), one_frame.size(), false);
  odd_data.finish();

  constexpr std::array<std::uint8_t, 17> odd_format = {
      1, 0, 1, 0, 0x40, 0x1f, 0, 0, 0x40, 0x1f, 0, 0, 1, 0, 8, 0, 0};
  mon::SegmentedWaveWriter odd_fmt(directory, "odd-format", odd_format.data(),
                                   odd_format.size());
  odd_fmt.append(one_frame.data(), one_frame.size(), false);
  odd_fmt.finish();
  return 0;
}
