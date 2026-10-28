#ifdef _WIN32
// Keep Windows min/max macros out of the C++ range calculations below.
#define NOMINMAX
#endif
#include "worker.hpp"
#include "pcm-input.hpp"
#include "sherpa-api.hpp"
#ifdef _WIN32
// worker.hpp maps the POSIX descriptor API; this file closes a C++ stream.
#undef close
#endif
#include <set>
#include <sstream>

static const SherpaOnnxOfflineRecognizer *recognizer = nullptr;
static const SherpaOnnxVoiceActivityDetector *vad = nullptr;
static std::string model_key;

static mon_json transcribe(std::map<std::string,std::string> &o) {
    load(o.at("--library"));
    auto stable=o;
    for (const auto *key : {"wave","--file","--mon-first-frame","--mon-end-frame","--output"}) stable.erase(key);
    const std::string key=mon_json(stable).dump();
    if (recognizer && key!=model_key) throw std::runtime_error("ASR session configuration changed");
    if (!recognizer) {
        SherpaOnnxOfflineRecognizerConfig c{};
        c.model_config.transducer.encoder=o.at("--model").c_str();
        c.model_config.transducer.decoder=o.at("--decoder").c_str();
        c.model_config.transducer.joiner=o.at("--joiner").c_str();
        c.model_config.tokens=o.at("--tokens").c_str();
        c.model_config.provider="cpu";
        c.model_config.num_threads=std::stoi(o.at("--threads"));
        c.model_config.modeling_unit="cjkchar";
        c.feat_config.sample_rate=16000; c.feat_config.feature_dim=80;
        c.decoding_method=o.at("--hotwords").empty()?"greedy_search":"modified_beam_search";
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

    // The character transducer accepts hotwords in its own token vocabulary.
    // Never pass punctuation that could be interpreted as a weighting directive.
    std::set<std::string> vocabulary;
    std::ifstream tokens(std::filesystem::u8path(o.at("--tokens"))); std::string row;
    while (std::getline(tokens,row)) { auto split=row.rfind(' '); if(split!=std::string::npos) vocabulary.insert(row.substr(0,split)); }
    std::istringstream words(o.at("--hotwords")); std::string word, hotwords;
    mon_json ignored=mon_json::array();
    while (std::getline(words,word)) {
        bool valid=!word.empty();
        for (size_t i=0;i<word.size();) {
            unsigned char c=word[i]; size_t n=c<0x80?1:c<0xe0?2:c<0xf0?3:4;
            if(i+n>word.size() || !vocabulary.count(word.substr(i,n)) || c==':' || c=='#' || c==' ') valid=false;
            i+=n;
        }
        if (valid) { if(!hotwords.empty())hotwords+='\n';hotwords+=word; } else ignored.push_back(word);
    }
    mon_json segments=mon_json::array(), result_tokens=mon_json::array();
    // Chunk without overlap: native ranges have single ownership and tokens are
    // never emitted twice. Outer meeting windows retain the common ownership rule.
    for (auto range:ranges) for (int a=range.first;a<range.second;a+=30*16000) {
        const int b=std::min(range.second,a+30*16000);
        auto stream=hotwords.empty()?API(SherpaOnnxCreateOfflineStream)(recognizer):API(SherpaOnnxCreateOfflineStreamWithHotwords)(recognizer,hotwords.c_str());
        if (!stream) throw std::runtime_error("cannot create ASR stream");
        API(SherpaOnnxAcceptWaveformOffline)(stream,16000,pcm.data()+a,b-a);
        API(SherpaOnnxDecodeOfflineStream)(recognizer,stream);
        auto result=API(SherpaOnnxGetOfflineStreamResult)(stream);
        if (!result) { API(SherpaOnnxDestroyOfflineStream)(stream); throw std::runtime_error("missing ASR result"); }
        int group_start=a; std::string text;
        for (int i=0;i<result->count;++i) {
            if (!result->timestamps || !result->tokens_arr || !std::isfinite(result->timestamps[i])) throw std::runtime_error("missing ASR timestamps");
            const int start=std::clamp(a+int(std::llround(result->timestamps[i]*16000)),a,b-1);
            const int stop=i+1<result->count?std::clamp(a+int(std::llround(result->timestamps[i+1]*16000)),start+1,b):b;
            const std::string token=result->tokens_arr[i];
            if (text.empty())group_start=start;
            text+=token;
            result_tokens.push_back({{"startUs",int64_t(start)*1000000/16000},{"text",token}});
            const bool pause=i+1<result->count && result->timestamps[i+1]-result->timestamps[i]>=0.5f;
            if (i+1==result->count || stop-group_start>=10*16000 || pause) {
                segments.push_back({{"startUs",int64_t(group_start)*1000000/16000},{"endUs",int64_t(stop)*1000000/16000},{"text",text}});text.clear();
            }
        }
        if (result->count==0 && result->text && *result->text) throw std::runtime_error("ASR text without timing");
        API(SherpaOnnxDestroyOfflineRecognizerResult)(result);
        API(SherpaOnnxDestroyOfflineStream)(stream);
    }
    mon_json models={{"model",o.at("--model")}};
    for (auto slot:{"decoder","joiner","library","tokens"})models[slot]=o.at(std::string("--")+slot);
    return {{"schemaVersion",1},{"backendId","sherpa-onnx-transducer"},{"models",models},
        {"durationUs",int64_t(pcm.size())*1000000/16000},{"segments",segments},{"tokens",result_tokens},{"ignoredHotwords",ignored}};
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
