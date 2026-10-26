#include "m4a_container.h"
#include <iostream>
#include <string>

// No Windows SDK, devices, decoder, or third-party JSON library is required.
int main(int argc, char** argv) {
  if (argc != 2) return 2;
  const std::filesystem::path root(argv[1]);
  const std::vector<std::string> accepted = {"sound", "chapter", "chapter-v1", "two-chapters"};
  const std::vector<std::string> rejected = {"unreferenced-text", "missing-chapter", "self-reference",
    "duplicate-reference", "duplicate-id", "zero-id", "unknown-track", "video", "multi-audio",
    "chapter-references", "truncated", "invalid-size", "duplicate-moov"};
  int failures = 0;
  auto test = [&](const std::string& name, bool expected) {
    const auto file = root / name;
    if (!std::filesystem::exists(file)) { std::cerr << "missing fixture: " << name << '\n'; ++failures; return; }
    bool actual = true;
    try { m4a_container::validate(file); } catch (const std::exception&) { actual = false; }
    if (actual != expected) { std::cerr << "unexpected acceptance: " << name << '\n'; ++failures; }
  };
  for (const auto& name : accepted) test("structure-" + name + ".m4a", true);
  for (const auto& name : rejected) test("structure-" + name + ".m4a", false);
  for (const auto& codec : {"aac", "alac"}) test(std::string("chapters-") + codec + ".m4a", true);
  return failures ? 1 : 0;
}
