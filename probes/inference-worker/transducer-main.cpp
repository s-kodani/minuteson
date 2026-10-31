#ifdef _WIN32
// Keep Windows min/max macros out of the C++ range calculations below.
#define NOMINMAX
#endif
#include "worker.hpp"
#include "pcm-input.hpp"
#include "sherpa-api.hpp"
#include "transducer-text.hpp"
#include "transducer-bpe.hpp"
#ifdef _WIN32
// worker.hpp maps the POSIX descriptor API; this file closes a C++ stream.
#undef close
#endif
#include <set>
#include <sstream>

static const SherpaOnnxOfflineRecognizer *recognizer = nullptr;
static const SherpaOnnxVoiceActivityDetector *vad = nullptr;
static std::string model_key;

struct TokenFile {
    std::filesystem::path path;
    TokenFile(const std::filesystem::path &directory,const std::string &table)
        :path(std::filesystem::absolute(directory)/"bpe-token-labels.txt") {
        // This derived table belongs to the job's private unit directory. It
        // never changes the installed model or its canonical file identities.
        #ifdef _WIN32
        HANDLE file=CreateFileW(path.c_str(),GENERIC_WRITE,0,nullptr,CREATE_NEW,FILE_ATTRIBUTE_NORMAL,nullptr);
        if(file==INVALID_HANDLE_VALUE)throw std::runtime_error("cannot create private BPE token transport");
        DWORD written=0;
        const bool ok=WriteFile(file,table.data(),DWORD(table.size()),&written,nullptr)&&written==table.size();
        CloseHandle(file);
        #else
        const int file=::open(path.c_str(),O_WRONLY|O_CREAT|O_EXCL|O_CLOEXEC|O_NOFOLLOW,0600);
        if(file<0)throw std::runtime_error("cannot create private BPE token transport");
        size_t written=0;
        while(written<table.size()) {
            auto count=::write(file,table.data()+written,table.size()-written);
            if(count<=0)break;
            written+=size_t(count);
        }
        ::close(file);const bool ok=written==table.size();
        #endif
        if(!ok)throw std::runtime_error("cannot write private BPE token transport");
    }
    ~TokenFile(){std::error_code error;std::filesystem::remove(path,error);}
};
static std::unique_ptr<mon_asr::BpeSymbols> bpe_symbols;
static std::unique_ptr<TokenFile> bpe_token_file;

static mon_json transcribe(std::map<std::string,std::string> &o) {
    if (o.count("--bpe-vocab")) {
        if (o.count("--bpeVocab") && o.at("--bpeVocab") != o.at("--bpe-vocab"))
            throw std::runtime_error("conflicting BPE vocabularies");
        o["--bpeVocab"]=o.at("--bpe-vocab");o.erase("--bpe-vocab");
    }
    const auto settings=mon_asr::settings(o);
    load(o.at("--library"));
    auto stable=o;
    for (const auto *key : {"wave","--file","--mon-first-frame","--mon-end-frame","--output"}) stable.erase(key);
    const std::string key=mon_json(stable).dump();
    if (recognizer && key!=model_key) throw std::runtime_error("ASR session configuration changed");
    // Decide the effective decoder before creating the recognizer. The legacy
    // call contract retains its original raw-prompt decoder choice; new adapter
    // configurations use only spellings that can actually reach the engine.
    std::set<std::string> vocabulary;
    std::ifstream tokens(std::filesystem::u8path(o.at("--tokens"))); std::string row;
    if (!tokens) throw std::runtime_error("cannot read ASR tokens");
    while (std::getline(tokens,row)) {
        const auto piece=mon_asr::token_piece(row);
        if(!piece.empty()) vocabulary.insert(piece);
    }
    const bool byte_fallback=mon_asr::has_byte_fallback(vocabulary);
    if (settings.modeling_unit=="bpe" && !bpe_symbols) {
        std::ifstream canonical(std::filesystem::u8path(o.at("--tokens"))),vocab(std::filesystem::u8path(o.at("--bpeVocab")));
        if(!canonical || !vocab)throw std::runtime_error("cannot read canonical BPE vocabulary");
        bpe_symbols=std::make_unique<mon_asr::BpeSymbols>(canonical,vocab);
        bpe_token_file=std::make_unique<TokenFile>(std::filesystem::u8path(o.at("--output")).parent_path(),bpe_symbols->table());
    }
    std::istringstream words(o.at("--hotwords")); std::string word, hotwords;
    mon_json ignored=mon_json::array(), ignored_reasons=mon_json::object();
    while (std::getline(words,word)) {
        std::string error=mon_asr::hotword_error(word,settings.modeling_unit,vocabulary,byte_fallback);
        if (error.empty() && bpe_symbols) {
            try {
                for(const auto &encoded:bpe_symbols->spelling_variants(word)) {
                    if(!hotwords.empty())hotwords+='\n';hotwords+=encoded;
                }
            } catch(const std::runtime_error &e) { error=e.what(); }
        } else if(error.empty()) { if(!hotwords.empty())hotwords+='\n';hotwords+=word; }
        if(!error.empty()) { ignored.push_back(word); ignored_reasons[word]=error; }
    }
    const std::string method=!settings.decoding_method.empty()?settings.decoding_method:
        ((settings.explicit_options?hotwords:o.at("--hotwords")).empty()?"greedy_search":"modified_beam_search");
    if (method=="greedy_search" && !hotwords.empty())
        throw std::runtime_error("hotwords require modified_beam_search");
    if (!recognizer) {
        SherpaOnnxOfflineRecognizerConfig c{};
        c.model_config.transducer.encoder=o.at("--model").c_str();
        c.model_config.transducer.decoder=o.at("--decoder").c_str();
        c.model_config.transducer.joiner=o.at("--joiner").c_str();
        const std::string transport_path=bpe_token_file?bpe_token_file->path.u8string():o.at("--tokens");
        c.model_config.tokens=transport_path.c_str();
        c.model_config.provider="cpu";
        c.model_config.num_threads=std::stoi(o.at("--threads"));
        c.model_config.modeling_unit=bpe_symbols?"cjkchar":settings.modeling_unit.c_str();
        if (!settings.model_type.empty()) c.model_config.model_type=settings.model_type.c_str();
        if (!bpe_symbols && o.count("--bpeVocab")) c.model_config.bpe_vocab=o.at("--bpeVocab").c_str();
        c.feat_config.sample_rate=16000; c.feat_config.feature_dim=settings.feature_dim;
        c.decoding_method=method.c_str();
        c.max_active_paths=4; c.hotwords_score=1.5f;
        recognizer=API(SherpaOnnxCreateOfflineRecognizer)(&c);
        if (!recognizer) throw std::runtime_error("cannot create ASR recognizer");
        model_key=key;
        if (o.at("--vad-enabled")=="1") {
            SherpaOnnxVadModelConfig v{};
            v.silero_vad.model=o.at("--vad").c_str(); v.silero_vad.window_size=512;
            v.silero_vad.threshold=std::stof(o.at("--vad-threshold"));
            v.silero_vad.min_speech_duration=std::stof(o.at("--vad-min-speech"));
            v.silero_vad.min_silence_duration=std::stof(o.at("--vad-min-silence"));
            v.silero_vad.max_speech_duration=std::stof(o.at("--vad-max-speech"));
            v.sample_rate=16000; v.num_threads=1; v.provider="cpu";
            vad=API(SherpaOnnxCreateVoiceActivityDetector)(&v,130.0f);
            if (!vad) throw std::runtime_error("cannot create ASR VAD");
        }
        fprintf(stderr,"mon_model_load: asr-transducer=1\n");
    }
    MonWave wave(o.at("--file"));
    const uint64_t first=std::stoull(o.at("--mon-first-frame")), end=std::stoull(o.at("--mon-end-frame"));
    if (end<=first || end-first>124ULL*16000) throw std::runtime_error("ASR input range exceeded");
    auto pcm=wave.read(first,end);
    std::vector<std::pair<int,int>> ranges;
    if (vad) {
        API(SherpaOnnxVoiceActivityDetectorReset)(vad);
        const int pad=int(std::stof(o.at("--vad-pad"))*16000);
        for (size_t i=0;i<pcm.size();i+=512) {
            float chunk[512]{}; const size_t n=std::min(size_t(512),pcm.size()-i);
            std::copy_n(pcm.data()+i,n,chunk);
            API(SherpaOnnxVoiceActivityDetectorAcceptWaveform)(vad,chunk,512);
        }
        API(SherpaOnnxVoiceActivityDetectorFlush)(vad);
        while (!API(SherpaOnnxVoiceActivityDetectorEmpty)(vad)) {
            auto segment=API(SherpaOnnxVoiceActivityDetectorFront)(vad);
            if (!segment) throw std::runtime_error("invalid VAD segment");
            const int a=std::max(0,segment->start-pad), b=std::min(int(pcm.size()),segment->start+segment->n+pad);
            if (b>a) {
                if (!ranges.empty() && a<=ranges.back().second) ranges.back().second=std::max(b,ranges.back().second);
                else ranges.emplace_back(a,b);
            }
            API(SherpaOnnxDestroySpeechSegment)(segment);
            API(SherpaOnnxVoiceActivityDetectorPop)(vad);
        }
    } else ranges.emplace_back(0,int(pcm.size()));

    mon_json segments=mon_json::array(), result_tokens=mon_json::array();
    // Chunk without overlap: native ranges have single ownership and tokens are
    // never emitted twice. Outer meeting windows retain the common ownership rule.
    for (auto range:ranges) for (int a=range.first;a<range.second;a+=settings.max_segment_seconds*16000) {
        const int b=std::min(range.second,a+settings.max_segment_seconds*16000);
        if (settings.explicit_options) fprintf(stderr,"mon_asr_range: first_frame=%d end_frame=%d\n",a,b);
        auto stream=hotwords.empty()?API(SherpaOnnxCreateOfflineStream)(recognizer):API(SherpaOnnxCreateOfflineStreamWithHotwords)(recognizer,hotwords.c_str());
        if (!stream) throw std::runtime_error("cannot create ASR stream");
        const auto destroy_stream=[](const SherpaOnnxOfflineStream *s){API(SherpaOnnxDestroyOfflineStream)(s);};
        std::unique_ptr<const SherpaOnnxOfflineStream,decltype(destroy_stream)> owned_stream(stream,destroy_stream);
        API(SherpaOnnxAcceptWaveformOffline)(stream,16000,pcm.data()+a,b-a);
        API(SherpaOnnxDecodeOfflineStream)(recognizer,stream);
        auto result=API(SherpaOnnxGetOfflineStreamResult)(stream);
        if (!result) throw std::runtime_error("missing ASR result");
        const auto destroy_result=[](const SherpaOnnxOfflineRecognizerResult *r){API(SherpaOnnxDestroyOfflineRecognizerResult)(r);};
        std::unique_ptr<const SherpaOnnxOfflineRecognizerResult,decltype(destroy_result)> owned_result(result,destroy_result);
        if (result->count<0 || result->count>500000 || (result->count && (!result->timestamps || !result->tokens_arr)))
            throw std::runtime_error("invalid ASR token result");
        for (int i=0;i<result->count;++i) if (!std::isfinite(result->timestamps[i]) || !result->tokens_arr[i])
            throw std::runtime_error("missing ASR timestamps or token");
        int group_start=a; std::string text;
        for (int i=0;i<result->count;++i) {
            if (!result->timestamps || !result->tokens_arr || !std::isfinite(result->timestamps[i])) throw std::runtime_error("missing ASR timestamps");
            const int start=std::clamp(a+int(std::llround(result->timestamps[i]*16000)),a,b-1);
            const int stop=i+1<result->count?std::clamp(a+int(std::llround(result->timestamps[i+1]*16000)),start+1,b):b;
            const std::string token=bpe_symbols?bpe_symbols->canonical_piece(result->tokens_arr[i]):result->tokens_arr[i];
            if (text.empty())group_start=start;
            const bool bpe=settings.modeling_unit=="bpe";
            text+=bpe?mon_asr::decode_piece(token):token;
            result_tokens.push_back({{"startUs",int64_t(start)*1000000/16000},{"text",token}});
            const bool pause=i+1<result->count && result->timestamps[i+1]-result->timestamps[i]>=0.5f;
            const auto encoding=bpe?mon_asr::utf8(text):mon_asr::Utf8::complete;
            if (encoding==mon_asr::Utf8::invalid || (i+1==result->count && encoding!=mon_asr::Utf8::complete))
                throw std::runtime_error("invalid ASR BPE UTF-8 output");
            // TDT may emit multiple tokens at the same timestamp. Wait until a
            // positive time boundary and a complete UTF-8 character before
            // splitting, so adjacent published segments never overlap.
            const bool boundary=!bpe || i+1==result->count || result->timestamps[i+1]>result->timestamps[i];
            if ((i+1==result->count || stop-group_start>=10*16000 || pause) && boundary && encoding==mon_asr::Utf8::complete) {
                const std::string rendered=bpe?mon_asr::render_bpe(text):text;
                if (!rendered.empty()) segments.push_back({{"startUs",int64_t(group_start)*1000000/16000},{"endUs",int64_t(stop)*1000000/16000},{"text",rendered}});
                text.clear();
            }
        }
        if (result->count==0 && result->text && *result->text) throw std::runtime_error("ASR text without timing");
    }
    mon_json models={{"model",o.at("--model")}};
    for (auto slot:{"decoder","joiner","library","tokens"})models[slot]=o.at(std::string("--")+slot);
    for (auto slot:{"bpeVocab","data"}) if(o.count(std::string("--")+slot))models[slot]=o.at(std::string("--")+slot);
    mon_json output={{"schemaVersion",1},{"backendId",settings.backend_id},{"models",models},
        {"durationUs",int64_t(pcm.size())*1000000/16000},{"segments",segments},{"tokens",result_tokens},{"ignoredHotwords",ignored}};
    if (settings.explicit_options && !ignored_reasons.empty()) output["ignoredHotwordReasons"]=ignored_reasons;
    return output;
}
static int infer(const std::string &kind,int argc,char **argv) {
    if (kind!="asr-transducer") return 2;
    auto o=options(argc,argv); auto output=transcribe(o);
    std::ofstream file(std::filesystem::u8path(o.at("--output")),std::ios::binary); file<<output.dump(); file.close();
    if(!file)throw std::runtime_error("cannot write ASR output");
    return 0;
}
int main(int argc,char **argv) {
    int result=2;
    try {
        result=(argc==2 && std::string(argv[1])=="--mon-resident-v1")?mon_loop(argc,argv,infer):infer("asr-transducer",argc,argv);
    } catch(const std::exception &e) {fprintf(stderr,"ASR: %s\n",e.what());result=3;}
    if(vad)API(SherpaOnnxDestroyVoiceActivityDetector)(vad);
    if(recognizer)API(SherpaOnnxDestroyOfflineRecognizer)(recognizer);
    return result;
}
