#include "worker.hpp"
#include "c-api.h"
#include <dlfcn.h>
#include <map>
#include <memory>
#include <cmath>
#include <cstring>
#include <filesystem>
static void *library = nullptr;
template <typename T> static T symbol(const char *name) {
    auto pointer = dlsym(library,name); if (!pointer) throw std::runtime_error(std::string("missing ABI symbol: ")+name);
    return reinterpret_cast<T>(pointer);
}
#define API(name) symbol<decltype(&name)>(#name)
static const SherpaOnnxOfflineSpeakerDiarization *diarizer = nullptr;
static const SherpaOnnxSpeakerEmbeddingExtractor *extractor = nullptr;
static std::string diarizer_key, extractor_key, library_path;
static std::map<std::string,std::string> options(int argc,char **argv) {
    std::map<std::string,std::string> result;
    for (int i=1;i<argc;++i) {
        std::string arg=argv[i]; auto equal=arg.find('=');
        if (equal!=std::string::npos) result[arg.substr(0,equal)]=arg.substr(equal+1);
        else if (arg.rfind("--",0)==0) { if (i+1>=argc) throw std::runtime_error("missing option"); result[arg]=argv[++i]; }
        else result["wave"]=arg;
    }
    return result;
}
static void load(const std::string &path) {
    if (library) { if (path!=library_path) throw std::runtime_error("runtime changed"); return; }
    library = dlopen(path.c_str(),RTLD_NOW|RTLD_LOCAL); if (!library) throw std::runtime_error("cannot load sherpa C API");
    library_path=path;
    if (std::string(API(SherpaOnnxGetVersionStr)()) != "1.13.8") throw std::runtime_error("unsupported sherpa ABI");
}
static int progress(int32_t done,int32_t total,void *) {
    if (total>0) fprintf(stderr,"progress %.2f%%\n",100.0*done/total);
    return 0;
}
static int diarize(std::map<std::string,std::string> &o) {
    load(o.at("--library"));
    // The C ABI treats zero as "use default" for these settings. Echo and
    // freeze the effective configuration, rather than claiming a disabled filter.
    if (std::stof(o.at("--min-duration-on"))==0) o["--min-duration-on"]="0.3";
    if (std::stof(o.at("--min-duration-off"))==0) o["--min-duration-off"]="0.5";
    if (std::stof(o.at("--clustering.cluster-threshold"))==0) o["--clustering.cluster-threshold"]="0.5";
    SherpaOnnxOfflineSpeakerDiarizationConfig c{};
    c.segmentation.pyannote.model=o.at("--segmentation.pyannote-model").c_str();
    c.segmentation.pyannote.window_shift_ratio=std::stof(o.at("--segmentation.pyannote-window-shift-ratio"));
    c.segmentation.num_threads=std::stoi(o.at("--segmentation.num-threads")); c.segmentation.provider="cpu";
    c.embedding.model=o.at("--embedding.model").c_str(); c.embedding.num_threads=std::stoi(o.at("--embedding.num-threads")); c.embedding.provider="cpu";
    c.clustering.num_clusters=std::stoi(o.at("--clustering.num-clusters")); c.clustering.threshold=std::stof(o.at("--clustering.cluster-threshold"));
    c.min_duration_on=std::stof(o.at("--min-duration-on")); c.min_duration_off=std::stof(o.at("--min-duration-off"));
    auto stable=o; stable.erase("wave"); stable.erase("--clustering.num-clusters");
    std::string key=mon_json(stable).dump();
    if (!diarizer) {
        diarizer=API(SherpaOnnxCreateOfflineSpeakerDiarization)(&c); if (!diarizer) return 3;
        diarizer_key=key; fprintf(stderr,"mon_model_load: diarizer=1\n");
    } else if (diarizer_key!=key) throw std::runtime_error("diarizer configuration changed");
    API(SherpaOnnxOfflineSpeakerDiarizationSetConfig)(diarizer,&c);
    // Keep the existing strict config-echo parser and saved stdout format.
    printf("OfflineSpeakerDiarizationConfig(segmentation=OfflineSpeakerSegmentationModelConfig(pyannote=OfflineSpeakerSegmentationPyannoteModelConfig(model=\"%s\", window_shift_ratio=%s), num_threads=%s, debug=False, provider=\"cpu\"), embedding=SpeakerEmbeddingExtractorConfig(model=\"%s\", num_threads=%s, debug=False, provider=\"cpu\"), clustering=FastClusteringConfig(num_clusters=%s, threshold=%s, compute_confidence=False), min_duration_on=%s, min_duration_off=%s)\nStarted\n",
        c.segmentation.pyannote.model,o.at("--segmentation.pyannote-window-shift-ratio").c_str(),o.at("--segmentation.num-threads").c_str(),
        c.embedding.model,o.at("--embedding.num-threads").c_str(),o.at("--clustering.num-clusters").c_str(),o.at("--clustering.cluster-threshold").c_str(),
        o.at("--min-duration-on").c_str(),o.at("--min-duration-off").c_str());

    auto wave=std::unique_ptr<const SherpaOnnxWave,decltype(API(SherpaOnnxFreeWave))>(API(SherpaOnnxReadWave)(o.at("wave").c_str()),API(SherpaOnnxFreeWave));
    if (!wave || wave->sample_rate!=16000) return 3;
    auto result=std::unique_ptr<const SherpaOnnxOfflineSpeakerDiarizationResult,decltype(API(SherpaOnnxOfflineSpeakerDiarizationDestroyResult))>(API(SherpaOnnxOfflineSpeakerDiarizationProcessWithCallback)(diarizer,wave->samples,wave->num_samples,progress,nullptr),API(SherpaOnnxOfflineSpeakerDiarizationDestroyResult));
    if (!result) return 3;
    auto segments=std::unique_ptr<const SherpaOnnxOfflineSpeakerDiarizationSegment,decltype(API(SherpaOnnxOfflineSpeakerDiarizationDestroySegment))>(API(SherpaOnnxOfflineSpeakerDiarizationResultSortByStartTime)(result.get()),API(SherpaOnnxOfflineSpeakerDiarizationDestroySegment));
    auto count=API(SherpaOnnxOfflineSpeakerDiarizationResultGetNumSegments)(result.get());
    for (int i=0;i<count;++i) printf("%.3f -- %.3f speaker_%02d\n",segments.get()[i].start,segments.get()[i].end,segments.get()[i].speaker);
    return 0;
}
static int embedding(std::map<std::string,std::string> &o) {
    load(o.at("--library"));
    const std::string key=o.at("--model")+":"+o.at("--threads");
    if (!extractor) {
        SherpaOnnxSpeakerEmbeddingExtractorConfig c{}; c.model=o.at("--model").c_str(); c.num_threads=std::stoi(o.at("--threads")); c.provider="cpu";
        extractor=API(SherpaOnnxCreateSpeakerEmbeddingExtractor)(&c); if (!extractor) return 3;
        extractor_key=key; fprintf(stderr,"mon_model_load: extractor=1\n");
    } else if (key!=extractor_key) throw std::runtime_error("extractor configuration changed");
    const uint64_t first=std::stoull(o.at("--first-frame")), end=std::stoull(o.at("--end-frame"));
    if (end<=first || end-first>480000) return 3;
    // Only read the selected PCM window, rather than loading an entire long
    // recording again for every feature request. Parent validates this WAV.
    std::ifstream wave(o.at("--wave"),std::ios::binary); char header[44]; wave.read(header,44);
    if (!wave || std::memcmp(header,"RIFF",4) || std::memcmp(header+8,"WAVEfmt ",8) || std::memcmp(header+36,"data",4)) return 3;
    wave.seekg(44+first*2); std::vector<int16_t> pcm(end-first); wave.read(reinterpret_cast<char *>(pcm.data()),pcm.size()*2); if (!wave) return 3;
    std::vector<float> samples; samples.reserve(pcm.size()); for (auto s:pcm) samples.push_back(float(s)/32768.0f);
    auto stream=std::unique_ptr<const SherpaOnnxOnlineStream,decltype(API(SherpaOnnxDestroyOnlineStream))>(API(SherpaOnnxSpeakerEmbeddingExtractorCreateStream)(extractor),API(SherpaOnnxDestroyOnlineStream));
    if (!stream) return 3;
    API(SherpaOnnxOnlineStreamAcceptWaveform)(stream.get(),16000,samples.data(),samples.size()); API(SherpaOnnxOnlineStreamInputFinished)(stream.get());
    int dimension=API(SherpaOnnxSpeakerEmbeddingExtractorDim)(extractor); if (dimension<1 || dimension>4096) return 3;
    mon_json bits=nullptr, reason="insufficient_audio"; std::string status="unavailable";
    if (API(SherpaOnnxSpeakerEmbeddingExtractorIsReady)(extractor,stream.get())) {
        auto vector=std::unique_ptr<const float,decltype(API(SherpaOnnxSpeakerEmbeddingExtractorDestroyEmbedding))>(API(SherpaOnnxSpeakerEmbeddingExtractorComputeEmbedding)(extractor,stream.get()),API(SherpaOnnxSpeakerEmbeddingExtractorDestroyEmbedding));
        if (!vector) return 3;
        double norm=0; bool finite=true; bits=mon_json::array();
        for (int i=0;i<dimension;++i) { float v=vector.get()[i]; finite &= std::isfinite(v); norm+=double(v)*v; uint32_t raw; std::memcpy(&raw,&v,4); char hex[9]; std::snprintf(hex,sizeof(hex),"%08x",raw); bits.push_back(std::string(hex)); }
        if (!finite) reason="non_finite_embedding";
        else if (!std::isfinite(norm) || std::sqrt(norm)<=1e-12) reason="zero_embedding";
        else { status="ready"; reason=nullptr; }
        if (status!="ready") bits=nullptr;
    }
    std::cout << mon_json{{"schemaVersion",1},{"engine","sherpa-onnx"},{"engineVersion","1.13.8"},{"sampleRate",16000},{"firstFrame",first},{"endFrame",end},{"dimension",dimension},{"status",status},{"reason",reason},{"vectorBits",bits}}.dump();
    return 0;
}
int main(int argc,char **argv) {
    int result=mon_loop(argc,argv,[](const std::string &kind,int n,char **a) { auto o=options(n,a); if (kind=="sherpa") return diarize(o); if (kind=="embedding") return embedding(o); return 2; });
    if (extractor) API(SherpaOnnxDestroySpeakerEmbeddingExtractor)(extractor);
    if (diarizer) API(SherpaOnnxDestroyOfflineSpeakerDiarization)(diarizer);
    if (library) dlclose(library);
    return result;
}
