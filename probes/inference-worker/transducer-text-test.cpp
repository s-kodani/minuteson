#include "transducer-text.hpp"
#include "transducer-bpe.hpp"
#include <cassert>

template <typename F> static bool rejects(F action) {
    try { action(); return false; } catch (const std::exception &) { return true; }
}

int main() {
    using namespace mon_asr;
    const auto legacy=settings({});
    assert(legacy.backend_id=="sherpa-onnx-transducer" && legacy.model_type.empty());
    assert(legacy.modeling_unit=="cjkchar" && legacy.feature_dim==80 && legacy.max_segment_seconds==30);
    assert(!legacy.explicit_options && legacy.decoding_method.empty());
    const std::map<std::string,std::string> configured={
        {"--backend-id","sherpa-onnx-nemo-tdt"},{"--model-type","nemo_transducer"},
        {"--feature-dim","128"},{"--modeling-unit","bpe"},
        {"--bpeVocab","/models/bpe.vocab"},{"--max-segment-seconds","29"}};
    auto effective=settings(configured);
    assert(effective.explicit_options && effective.feature_dim==128 && effective.max_segment_seconds==29);
    for (const auto &bad : {"0","31","29seconds","-1","999999999"}) {
        auto options=configured; options["--max-segment-seconds"]=bad;
        assert(rejects([&]{settings(options);}));
    }
    auto options=configured;options.erase("--bpeVocab");assert(rejects([&]{settings(options);}));
    options=configured;options["--decoding-method"]="modified_beam_search";
    assert(settings(options).decoding_method=="modified_beam_search");
    options["--decoding-method"]="unknown";assert(rejects([&]{settings(options);}));

    const std::string japanese=u8"\u65e5\u672c\u8a9e";
    assert(render_bpe(decode_piece(u8"\u2581\u65e5\u672c")+decode_piece(u8"\u2581\u8a9e"))==japanese);
    assert(render_bpe(" hello world ! ")=="hello world!");
    assert(render_bpe("   ").empty());
    std::string bytes=decode_piece("<0xE6>");
    assert(utf8(bytes)==Utf8::incomplete);
    bytes+=decode_piece("<0x97>");assert(utf8(bytes)==Utf8::incomplete);
    bytes+=decode_piece("<0xA5>");assert(utf8(bytes)==Utf8::complete);
    assert(render_bpe(bytes)==u8"\u65e5");
    const std::string emoji=decode_piece("<0xF0>")+decode_piece("<0x9F>")+decode_piece("<0x98>")+decode_piece("<0x80>");
    assert(utf8(emoji)==Utf8::complete && render_bpe(emoji)==u8"\U0001f600");
    assert(decode_piece("<0xA>")=="\n" && decode_piece("MON")=="MON");
    for (const auto &invalid : {std::string("\xc0\x80"),std::string("\xed\xa0\x80"),std::string("\xf4\x90\x80\x80")}) {
        assert(utf8(invalid)==Utf8::invalid);
        assert(rejects([&]{render_bpe(invalid);}));
    }
    assert(rejects([&]{render_bpe(bytes.substr(0,2));}));

    std::set<std::string> vocabulary={"M","O","N",u8"\u65e5",u8"\u672c",u8"\u8a9e",u8"\u2581","hello","world"};
    assert(token_piece(u8"\u4f1a\t490")==u8"\u4f1a");
    assert(token_piece(u8"\u2581hello 18")==u8"\u2581hello");
    assert(token_piece("missing-id").empty());
    assert(hotword_error("MON","cjkchar",vocabulary,false).empty());
    assert(!hotword_error("OTHER","cjkchar",vocabulary,false).empty());
    assert(hotword_error("hello world","bpe",vocabulary,false).empty());
    assert(!hotword_error("missing","bpe",vocabulary,false).empty());
    for (const auto &invalid : {"MON :99","MON/OTHER","MON#","<unk>","<0xFF>","MON\tX"})
        assert(!hotword_error(invalid,"bpe",vocabulary,true).empty());
    assert(!hotword_error(std::string("\xed\xa0\x80"),"bpe",vocabulary,true).empty());
    const char *digits="0123456789ABCDEF";
    for (int i=0;i<256;++i) {
        std::string token="<0x00>";token[3]=digits[i/16];token[4]=digits[i%16];vocabulary.insert(token);
    }
    assert(has_byte_fallback(vocabulary));
    assert(hotword_error(emoji,"bpe",vocabulary,true).empty());
    vocabulary.erase("<0xFF>");assert(!has_byte_fallback(vocabulary));

    std::ostringstream canonical,vocab;
    std::vector<std::string> pieces={"<unk>"};
    for (int i=0;i<256;++i) {
        std::string piece="<0x00>";piece[3]=digits[i/16];piece[4]=digits[i%16];pieces.push_back(piece);
    }
    for (const auto &piece:{u8"\u2581",u8"\u30df",u8"\u30e9\u30a4",u8"\u682a",u8"\u5f0f",u8"\u4f1a",u8"\u793e","<blk>"}) pieces.push_back(piece);
    for(size_t i=0;i<pieces.size();++i) { canonical<<pieces[i]<<' '<<i<<'\n';vocab<<pieces[i]<<"\t-1.0\n"; }
    std::istringstream canonical_input(canonical.str()),vocab_input(vocab.str());
    BpeSymbols symbols(canonical_input,vocab_input);
    std::istringstream table(symbols.table());std::string line;size_t count=0;
    while(std::getline(table,line)) {
        const auto label=token_piece(line);
        std::vector<Character> scalars;
        assert(utf8(label,&scalars)==Utf8::complete);
        if(count==0 || count+1==pieces.size()) assert(label==pieces[count]);
        else assert(scalars.size()==1 && scalars[0].value==0xf0000+count);
        ++count;
    }
    assert(count==pieces.size());
    const auto restore=[&](const std::string &encoded) {
        std::vector<Character> characters;assert(utf8(encoded,&characters)==Utf8::complete);
        std::string decoded;
        for(const auto &c:characters) decoded+=decode_piece(symbols.canonical_piece(encoded.substr(c.first,c.size)));
        return render_bpe(decoded);
    };
    const auto spelling=symbols.spelling_variants(u8"\u30df\u30e9\u30a4");
    assert(spelling.size()==2);
    for(const auto &encoded:spelling) assert(restore(encoded)==u8"\u30df\u30e9\u30a4");
    const auto sentence=symbols.spelling_variants(u8"\u682a\u5f0f\u4f1a\u793e\u30df\u30e9\u30a4");
    assert(sentence[0].find(spelling[0])!=std::string::npos);
    assert(sentence[0].find(spelling[1])==std::string::npos);
    assert(restore(symbols.spelling_variants(emoji)[0])==emoji);
    assert(rejects([&]{symbols.canonical_piece("not a transport label");}));
    std::istringstream duplicate("<unk> 0\n<blk> 0\n"),bad_vocab("<unk> -1\n<blk> -1\n");
    assert(rejects([&]{BpeSymbols invalid(duplicate,bad_vocab);}));
}
