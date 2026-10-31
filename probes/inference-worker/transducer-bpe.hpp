#pragma once
// Token-ID transport for the stock sherpa C API. BPE spellings need both a
// sentence-boundary and an in-sentence form. The SDK's word encoder always
// inserts a boundary, so encode here and carry each ID as one private-use
// scalar through its character-hotword API. No model ID or weight is changed.
#include "transducer-text.hpp"
#include <cmath>
#include <iomanip>
#include <istream>
#include <limits>
#include <utility>

namespace mon_asr {
inline std::string scalar_utf8(uint32_t value) {
    std::string out;
    if (value <= 0x7f) out += char(value);
    else if (value <= 0x7ff) {
        out += char(0xc0 | value >> 6); out += char(0x80 | (value & 63));
    } else if (value <= 0xffff && !(value >= 0xd800 && value <= 0xdfff)) {
        out += char(0xe0 | value >> 12); out += char(0x80 | ((value >> 6) & 63)); out += char(0x80 | (value & 63));
    } else if (value >= 0x10000 && value <= 0x10ffff) {
        out += char(0xf0 | value >> 18); out += char(0x80 | ((value >> 12) & 63));
        out += char(0x80 | ((value >> 6) & 63)); out += char(0x80 | (value & 63));
    } else throw std::runtime_error("invalid Unicode transport scalar");
    return out;
}

class BpeSymbols {
    std::vector<std::string> pieces_, labels_;
    std::map<std::string,int> ids_, label_ids_;
    std::map<std::string,float> scores_;
    size_t max_piece_ = 0;

    std::vector<int> encode(const std::string &text) const {
        // Equal-score vocabularies choose the fewest pieces; ties prefer a
        // longer first piece. Unknown Unicode uses the model's byte tokens.
        const float missing=-std::numeric_limits<float>::infinity();
        std::vector<float> best(text.size()+1,missing);
        std::vector<size_t> next(text.size());
        std::vector<int> token(text.size(),-1);
        best.back()=0;
        const char *hex_digits="0123456789ABCDEF";
        for (size_t i=text.size();i-- > 0;) {
            for (size_t n=1;n<=std::min(max_piece_,text.size()-i);++n) {
                const auto piece=text.substr(i,n);
                const auto score=scores_.find(piece);
                if (score==scores_.end() || !std::isfinite(best[i+n]) || piece=="<unk>" || piece=="<blk>") continue;
                const float total=score->second+best[i+n];
                if (total>best[i] || (total==best[i] && i+n>next[i])) {
                    best[i]=total;next[i]=i+n;token[i]=ids_.at(piece);
                }
            }
            if (token[i]<0 && std::isfinite(best[i+1])) {
                const auto byte=static_cast<unsigned char>(text[i]);
                std::string piece="<0x00>";piece[3]=hex_digits[byte/16];piece[4]=hex_digits[byte%16];
                const auto found=ids_.find(piece);
                if (found!=ids_.end()) { best[i]=best[i+1]-100;next[i]=i+1;token[i]=found->second; }
            }
        }
        if (!std::isfinite(best[0])) throw std::runtime_error("term outside BPE vocabulary");
        std::vector<int> out;
        for (size_t i=0;i<text.size();i=next[i]) {
            if (token[i]<0 || next[i]<=i) throw std::runtime_error("invalid BPE spelling path");
            out.push_back(token[i]);
        }
        return out;
    }

public:
    BpeSymbols(std::istream &tokens,std::istream &vocab) {
        std::map<int,std::string> ordered;
        std::string row;
        while (std::getline(tokens,row)) {
            const auto split=row.find_last_of(" \t");
            if (split==std::string::npos) throw std::runtime_error("invalid canonical BPE tokens");
            const auto number=row.substr(split+1);
            size_t consumed=0;const int id=std::stoi(number,&consumed);
            const auto piece=row.substr(0,split);
            if (id<0 || id>65533 || consumed!=number.size() || piece.empty() ||
                utf8(piece)!=Utf8::complete || !ordered.emplace(id,piece).second || !ids_.emplace(piece,id).second)
                throw std::runtime_error("invalid canonical BPE token identity");
        }
        for (const auto &entry:ordered) {
            if (entry.first!=int(pieces_.size())) throw std::runtime_error("BPE IDs must be contiguous");
            const auto &piece=entry.second;
            pieces_.push_back(piece);
            const auto label=(piece=="<unk>" || piece=="<blk>")?piece:scalar_utf8(0xf0000+entry.first);
            labels_.push_back(label);label_ids_.emplace(label,entry.first);
        }
        if (pieces_.empty() || pieces_.back()!="<blk>" || !ids_.count("<unk>"))
            throw std::runtime_error("BPE special tokens do not match transducer contract");
        while (std::getline(vocab,row)) {
            std::istringstream line(row);std::string piece,extra;float score;
            if (!(line>>piece>>score) || line>>extra || !std::isfinite(score) || !ids_.count(piece) ||
                !scores_.emplace(piece,score).second) throw std::runtime_error("BPE vocabulary does not match canonical IDs");
            max_piece_=std::max(max_piece_,piece.size());
        }
        if (scores_.size()!=pieces_.size()) throw std::runtime_error("BPE vocabulary is incomplete");
    }

    std::string table() const {
        std::ostringstream output;
        for (size_t id=0;id<labels_.size();++id) output<<labels_[id]<<' '<<id<<'\n';
        return output.str();
    }

    std::vector<std::string> spelling_variants(const std::string &spelling) const {
        std::istringstream words(spelling);std::string word,plain;
        const std::string boundary="\xe2\x96\x81";
        while (words>>word) { if (!plain.empty()) plain+=boundary;plain+=word; }
        if (plain.empty()) throw std::runtime_error("empty BPE spelling");
        std::set<std::string> seen;
        std::vector<std::string> out;
        for (const auto &form:{plain,boundary+plain}) {
            std::string encoded;
            for (int id:encode(form)) encoded+=labels_.at(id);
            if (seen.insert(encoded).second) out.push_back(encoded);
        }
        return out;
    }

    std::string canonical_piece(const std::string &label) const {
        const auto found=label_ids_.find(label);
        if (found==label_ids_.end()) throw std::runtime_error("unknown ASR token transport symbol");
        auto piece=pieces_.at(found->second);
        // Reproduce sherpa's public token spelling, including byte-fallback
        // markers, so raw artifacts are equivalent to original-label runs.
        if (piece.size()==6 && piece.compare(0,3,"<0x")==0 && piece.back()=='>') {
            const int hi=hex(piece[3]),lo=hex(piece[4]);
            if (hi>=0 && lo>=0) {
                const int byte=hi*16+lo;std::ostringstream digits;digits<<std::hex<<std::uppercase<<byte;
                if (piece.substr(3,2)==digits.str() && byte>=0x20 && byte<=0x7e) return std::string(1,char(byte));
            }
        }
        if (piece.compare(0,3,"\xe2\x96\x81")==0) piece.replace(0,3," ");
        return piece;
    }
};
} // namespace mon_asr
