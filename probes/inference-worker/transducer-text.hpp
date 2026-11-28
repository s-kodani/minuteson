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

enum class HotwordRejection { none, empty, invalid_utf8, control_character, reserved_syntax, whitespace, outside_vocabulary };

inline const char *hotword_reason(HotwordRejection reason) {
    switch (reason) {
        case HotwordRejection::empty: return "empty";
        case HotwordRejection::invalid_utf8: return "invalidUtf8";
        case HotwordRejection::control_character: return "controlCharacter";
        case HotwordRejection::reserved_syntax: return "reservedSyntax";
        case HotwordRejection::whitespace: return "whitespace";
        case HotwordRejection::outside_vocabulary: return "outsideVocabulary";
        case HotwordRejection::none: return "unknown";
    }
    return "unknown";
}

inline HotwordRejection hotword_rejection(const std::string &word, const std::set<std::string> &vocabulary) {
    if (word.empty()) return HotwordRejection::empty;
    std::vector<Character> characters;
    if (utf8(word, &characters) != Utf8::complete) return HotwordRejection::invalid_utf8;
    for (const auto &c : characters) {
        if (c.value < 0x20 || (c.value >= 0x7f && c.value <= 0x9f)) return HotwordRejection::control_character;
        // '/' separates per-stream phrases. ':' and '#' are reserved by the
        // hotword grammar. A registered spelling must never become directives.
        if (c.value == ':' || c.value == '#' || c.value == '/') return HotwordRejection::reserved_syntax;
        if (c.value == ' ') return HotwordRejection::whitespace;
    }
    for (const auto &c : characters) {
        if (!vocabulary.count(word.substr(c.first, c.size))) {
            // 語彙にある全角空白等は従来どおり受け付ける。
            if (c.value == 0xa0 || c.value == 0x1680 ||
                (c.value >= 0x2000 && c.value <= 0x200a) ||
                c.value == 0x2028 || c.value == 0x2029 || c.value == 0x202f ||
                c.value == 0x205f || c.value == 0x3000) return HotwordRejection::whitespace;
            return HotwordRejection::outside_vocabulary;
        }
    }
    return HotwordRejection::none;
}
} // namespace mon_asr
