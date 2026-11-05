#pragma once
// Small, dependency-free helpers shared by the native worker and its tests.
#include <cstddef>
#include <cstdint>
#include <set>
#include <string>
#include <vector>

namespace mon_asr {
struct Character { uint32_t value; size_t first; size_t size; };
enum class Utf8 { complete, incomplete, invalid };
// Unlike byte-count heuristics, this rejects overlong encodings, surrogates and
// out-of-range code points. An incomplete suffix may wait for the next token.
inline Utf8 utf8(const std::string &text, std::vector<Character> *characters = nullptr) {
    if (characters) characters->clear();
    for (size_t at = 0; at < text.size();) {
        const auto c = static_cast<unsigned char>(text[at]);
        size_t n;
        uint32_t cp;
        if (c < 0x80) { n = 1; cp = c; }
        else if (c >= 0xc2 && c <= 0xdf) { n = 2; cp = c & 0x1f; }
        else if (c >= 0xe0 && c <= 0xef) { n = 3; cp = c & 0x0f; }
        else if (c >= 0xf0 && c <= 0xf4) { n = 4; cp = c & 7; }
        else return Utf8::invalid;
        for (size_t i = 1; i < n; ++i) {
            if (at + i >= text.size()) return Utf8::incomplete;
            const auto next = static_cast<unsigned char>(text[at + i]);
            if ((next & 0xc0) != 0x80) return Utf8::invalid;
            if (i == 1 && ((c == 0xe0 && next < 0xa0) || (c == 0xed && next >= 0xa0) ||
                          (c == 0xf0 && next < 0x90) || (c == 0xf4 && next >= 0x90))) return Utf8::invalid;
            cp = (cp << 6) | (next & 0x3f);
        }
        if (characters) characters->push_back({cp, at, n});
        at += n;
    }
    return Utf8::complete;
}

inline std::string token_piece(const std::string &row) {
    const auto split=row.find_last_of(" \t");
    return split==std::string::npos?std::string():row.substr(0,split);
}

inline std::string hotword_error(const std::string &word, const std::set<std::string> &vocabulary) {
    if (word.empty()) return "empty term";
    std::vector<Character> characters;
    if (utf8(word, &characters) != Utf8::complete) return "invalid UTF-8";
    for (const auto &c : characters) {
        if (c.value < 0x20 || (c.value >= 0x7f && c.value <= 0x9f)) return "control character";
        // '/' separates per-stream phrases. ':' and '#' are reserved by the
        // hotword grammar. A registered spelling must never become directives.
        if (c.value == ':' || c.value == '#' || c.value == '/') return "reserved hotword syntax";
        if (c.value == ' ' || !vocabulary.count(word.substr(c.first, c.size)))
            return "character outside model vocabulary";
    }
    return {};
}
} // namespace mon_asr
