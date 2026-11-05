#include "transducer-text.hpp"
#include <cassert>

int main() {
    using namespace mon_asr;
    const std::set<std::string> vocabulary={"M","O","N",u8"\u4f1a",u8"\u8b70"};
    assert(token_piece(u8"\u4f1a\t490")==u8"\u4f1a");
    assert(token_piece(u8"\u8b70 491")==u8"\u8b70");
    assert(token_piece("missing-id").empty());
    assert(hotword_error("MON",vocabulary).empty());
    assert(hotword_error(u8"\u4f1a\u8b70",vocabulary).empty());
    assert(hotword_error("OTHER",vocabulary)=="character outside model vocabulary");
    assert(!hotword_error("MON X",vocabulary).empty());
    for (const auto &invalid : {"MON:99","MON/OTHER","MON#","MON\tX"})
        assert(!hotword_error(invalid,vocabulary).empty());
    for (const auto &invalid : {"\xc0\x80","\xed\xa0\x80","\xf4\x90\x80\x80","\xe6\x97"})
        assert(hotword_error(invalid,vocabulary)=="invalid UTF-8");
}
