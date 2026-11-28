#include "transducer-text.hpp"
#include <cassert>

int main() {
    using namespace mon_asr;
    const std::set<std::string> vocabulary={"M","O","N",u8"\u4f1a",u8"\u8b70"};
    assert(token_piece(u8"\u4f1a\t490")==u8"\u4f1a");
    assert(token_piece(u8"\u8b70 491")==u8"\u8b70");
    assert(token_piece("missing-id").empty());
    assert(hotword_rejection("MON",vocabulary)==HotwordRejection::none);
    assert(hotword_rejection(u8"\u4f1a\u8b70",vocabulary)==HotwordRejection::none);
    assert(hotword_rejection("OTHER",vocabulary)==HotwordRejection::outside_vocabulary);
    assert(hotword_rejection("New York",vocabulary)==HotwordRejection::whitespace);
    assert(hotword_rejection(u8"MON\u3000X",vocabulary)==HotwordRejection::whitespace);
    assert(hotword_rejection(u8"MON\u3000", { "M", "O", "N", u8"\u3000" })==HotwordRejection::none);
    for (const auto &invalid : {"MON:99","A/B","C#"})
        assert(hotword_rejection(invalid,vocabulary)==HotwordRejection::reserved_syntax);
    assert(hotword_rejection("MON\tX",vocabulary)==HotwordRejection::control_character);
    assert(hotword_rejection("",vocabulary)==HotwordRejection::empty);
    for (const auto &invalid : {"\xc0\x80","\xed\xa0\x80","\xf4\x90\x80\x80","\xe6\x97"})
        assert(hotword_rejection(invalid,vocabulary)==HotwordRejection::invalid_utf8);
}
