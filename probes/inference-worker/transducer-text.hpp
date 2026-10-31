#pragma once
// Small, dependency-free helpers shared by the native worker and its tests.
#include <algorithm>
#include <cstdint>
#include <map>
#include <set>
#include <sstream>
#include <stdexcept>
#include <string>
#include <vector>

namespace mon_asr {
struct Settings {
    std::string backend_id = "sherpa-onnx-transducer";
    std::string model_type;
    std::string modeling_unit = "cjkchar";
    std::string decoding_method;
    int feature_dim = 80;
    int max_segment_seconds = 30;
    bool explicit_options = false;
};

inline int integer_option(const std::string &value, int maximum) {
    if (value.empty() || value.size() > 6 ||
        value.find_first_not_of("0123456789") != std::string::npos)
        throw std::runtime_error("invalid ASR numeric option");
    const int result = std::stoi(value);
    if (result < 1 || result > maximum) throw std::runtime_error("ASR option out of range");
    return result;
}

inline Settings settings(const std::map<std::string, std::string> &options) {
    Settings result;
    const auto take = [&](const char *key, std::string &value) {
        auto found = options.find(key);
        if (found != options.end()) { value = found->second; result.explicit_options = true; }
    };
    take("--backend-id", result.backend_id);
    take("--model-type", result.model_type);
    take("--modeling-unit", result.modeling_unit);
    take("--decoding-method", result.decoding_method);
    for (const auto &entry : {std::make_pair("--feature-dim", &result.feature_dim),
                              std::make_pair("--max-segment-seconds", &result.max_segment_seconds)}) {
        auto found = options.find(entry.first);
        if (found != options.end()) {
            result.explicit_options = true;
            *entry.second = integer_option(found->second, entry.second == &result.feature_dim ? 256 : 30);
        }
    }
    if (result.backend_id.empty() || result.backend_id.size() > 128 ||
        result.backend_id.find_first_not_of("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_.") != std::string::npos ||
        result.model_type.size() > 128 ||
        result.model_type.find_first_not_of("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_") != std::string::npos ||
        (result.modeling_unit != "cjkchar" && result.modeling_unit != "bpe"))
        throw std::runtime_error("unsupported ASR settings");
    if (!result.decoding_method.empty() && result.decoding_method != "greedy_search" &&
        result.decoding_method != "modified_beam_search") throw std::runtime_error("unsupported ASR decoding method");
    if (result.modeling_unit == "bpe" &&
        (!options.count("--bpeVocab") || options.at("--bpeVocab").empty()))
        throw std::runtime_error("BPE vocabulary is required");
    return result;
}

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

inline int hex(char c) {
    if (c >= '0' && c <= '9') return c - '0';
    if (c >= 'a' && c <= 'f') return c - 'a' + 10;
    if (c >= 'A' && c <= 'F') return c - 'A' + 10;
    return -1;
}

inline std::string decode_piece(const std::string &piece) {
    // sherpa returns non-printable byte-fallback pieces as <0xE3>, and also
    // emits unpadded values such as <0xA>. Ordinary ASCII pieces stay literal.
    if ((piece.size() == 5 || piece.size() == 6) && piece.compare(0, 3, "<0x") == 0 && piece.back() == '>') {
        int value = 0;
        bool valid = true;
        for (size_t i = 3; i + 1 < piece.size(); ++i) {
            const int digit = hex(piece[i]);
            if (digit < 0) { valid = false; break; }
            value = value * 16 + digit;
        }
        if (valid) return std::string(1, char(value));
    }
    std::string result;
    for (size_t i = 0; i < piece.size();) {
        if (piece.compare(i, 3, "\xe2\x96\x81") == 0) { result += ' '; i += 3; }
        else result += piece[i++];
    }
    return result;
}

inline bool cjk(uint32_t cp) {
    return (cp >= 0x1100 && cp <= 0x11ff) || (cp >= 0x2e80 && cp <= 0xa4cf) ||
           (cp >= 0xa840 && cp <= 0xd7af) || (cp >= 0xf900 && cp <= 0xfaff) ||
           (cp >= 0xfe30 && cp <= 0xfe4f) || (cp >= 0xff65 && cp <= 0xffdc) ||
           (cp >= 0x20000 && cp <= 0x2ffff);
}

inline bool punctuation(uint32_t cp) {
    return (cp >= 0x21 && cp <= 0x2f) || (cp >= 0x3a && cp <= 0x40) ||
           (cp >= 0x5b && cp <= 0x60) || (cp >= 0x7b && cp <= 0x7e) ||
           (cp >= 0x3000 && cp <= 0x303f) || (cp >= 0xff01 && cp <= 0xff0f) ||
           (cp >= 0xff1a && cp <= 0xff20) || (cp >= 0xff3b && cp <= 0xff40) ||
           (cp >= 0xff5b && cp <= 0xff65);
}

inline std::string render_bpe(const std::string &text) {
    std::vector<Character> characters;
    if (utf8(text, &characters) != Utf8::complete) throw std::runtime_error("invalid ASR BPE UTF-8 output");
    std::string result;
    for (size_t i = 0; i < characters.size(); ++i) {
        const auto &current = characters[i];
        if (current.value == ' ' && i > 0 && i + 1 < characters.size() &&
            ((cjk(characters[i - 1].value) && cjk(characters[i + 1].value)) || punctuation(characters[i + 1].value)))
            continue;
        result.append(text, current.first, current.size);
    }
    const auto first = result.find_first_not_of(' ');
    if (first == std::string::npos) return {};
    return result.substr(first, result.find_last_not_of(' ') - first + 1);
}

inline bool has_byte_fallback(const std::set<std::string> &vocabulary) {
    const char *digits = "0123456789ABCDEF";
    for (int i = 0; i != 256; ++i) {
        std::string token = "<0x00>"; token[3] = digits[i / 16]; token[4] = digits[i % 16];
        if (!vocabulary.count(token)) return false;
    }
    return true;
}

inline std::string token_piece(const std::string &row) {
    const auto split=row.find_last_of(" \t");
    return split==std::string::npos?std::string():row.substr(0,split);
}

inline bool covered(const std::string &text, const std::set<std::string> &vocabulary) {
    std::vector<bool> reachable(text.size() + 1, false);
    reachable[0] = true;
    for (size_t i = 0; i < text.size(); ++i) if (reachable[i]) {
        for (size_t end = i + 1; end <= text.size(); ++end) {
            if (vocabulary.count(text.substr(i, end - i))) reachable[end] = true;
        }
    }
    return reachable.back();
}

inline std::string hotword_error(const std::string &word, const std::string &unit,
                                 const std::set<std::string> &vocabulary, bool byte_fallback) {
    if (word.empty()) return "empty term";
    std::vector<Character> characters;
    if (utf8(word, &characters) != Utf8::complete) return "invalid UTF-8";
    for (const auto &c : characters) {
        if (c.value < 0x20 || (c.value >= 0x7f && c.value <= 0x9f)) return "control character";
        // '/' separates per-stream phrases. ':' and '#' are reserved by the
        // hotword grammar. A registered spelling must never become directives.
        if (c.value == ':' || c.value == '#' || c.value == '/') return "reserved hotword syntax";
        if (unit == "cjkchar" && (c.value == ' ' || !vocabulary.count(word.substr(c.first, c.size))))
            return "character outside model vocabulary";
    }
    if (unit == "bpe") {
        if (word.find('<') != std::string::npos || word.find('>') != std::string::npos)
            return "reserved BPE token syntax";
        if (!byte_fallback) {
            std::istringstream input(word); std::string part;
            bool any = false;
            while (input >> part) {
                any = true;
                if (!covered(std::string("\xe2\x96\x81") + part, vocabulary)) return "term outside BPE vocabulary";
            }
            if (!any) return "empty term";
        }
    }
    return {};
}
} // namespace mon_asr
