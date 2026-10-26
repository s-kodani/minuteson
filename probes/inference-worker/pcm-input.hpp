#pragma once
// Canonical PCM range reader, shared by ASR, diarization and embeddings.
#include <cstdint>
#include <algorithm>
#include <cstdio>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <limits>
#include <stdexcept>
#include <string>
#include <vector>
inline uint64_t mon_pcm_first=0, mon_pcm_end=0;
static uint32_t mon_le32(const unsigned char *p) {
    return uint32_t(p[0])|(uint32_t(p[1])<<8)|(uint32_t(p[2])<<16)|(uint32_t(p[3])<<24);
}
static uint64_t mon_le64(const unsigned char *p) {return mon_le32(p)|(uint64_t(mon_le32(p+4))<<32);}
struct MonWave {
    std::ifstream file;
    uint64_t offset=0,frames=0;
    explicit MonWave(const std::string &path):file(std::filesystem::u8path(path),std::ios::binary) {
        if(!file)throw std::runtime_error("cannot open canonical WAV");
        file.seekg(0,std::ios::end); const auto n=file.tellg();
        if(n<44)throw std::runtime_error("short canonical WAV");
        const uint64_t size=uint64_t(n);file.seekg(0);
        unsigned char root[12];file.read(reinterpret_cast<char*>(root),12);
        const bool rf64=!std::memcmp(root,"RF64",4);
        if((!rf64&&std::memcmp(root,"RIFF",4))||std::memcmp(root+8,"WAVE",4)
            ||(rf64?mon_le32(root+4)!=UINT32_MAX:uint64_t(mon_le32(root+4))+8!=size))throw std::runtime_error("invalid WAV container");
        uint64_t position=12,extended=0;bool fmt=false,ds=false,data=false;
        for(unsigned i=0;position<size&&i<4096;++i) {
            if(size-position<8)throw std::runtime_error("invalid WAV chunk");
            file.seekg(position);unsigned char h[8];file.read(reinterpret_cast<char*>(h),8);
            uint64_t length=mon_le32(h+4); const uint64_t payload=position+8;
            if(!std::memcmp(h,"ds64",4)) {
                if(!rf64||ds||length!=28)throw std::runtime_error("invalid RF64 sizes");
                unsigned char d[28];file.read(reinterpret_cast<char*>(d),28);
                extended=mon_le64(d+8);
                if(mon_le64(d)!=size-8||mon_le64(d+16)!=extended/2||extended%2||mon_le32(d+24))throw std::runtime_error("invalid RF64 sizes");
                ds=true;
            } else if(!std::memcmp(h,"fmt ",4)) {
                if(fmt||length!=16)throw std::runtime_error("invalid WAV format");
                unsigned char f[16];file.read(reinterpret_cast<char*>(f),16);
                if(f[0]!=1||f[1]||f[2]!=1||f[3]||mon_le32(f+4)!=16000||mon_le32(f+8)!=32000||f[12]!=2||f[13]||f[14]!=16||f[15])throw std::runtime_error("expected mono PCM16 16kHz");
                fmt=true;
            } else if(!std::memcmp(h,"data",4)) {
                if(data||!fmt)throw std::runtime_error("invalid WAV data");
                if(rf64){if(!ds||length!=UINT32_MAX)throw std::runtime_error("invalid RF64 data");length=extended;}
                if(!length||length%2)throw std::runtime_error("invalid PCM size");
                offset=payload;frames=length/2;data=true;
            }
            if(!file||length>size-payload)throw std::runtime_error("truncated WAV");
            position=payload+length+(length&1);
        }
        if(position!=size||!fmt||!data||(rf64&&!ds))throw std::runtime_error("incomplete WAV");
    }
    std::vector<float> read(uint64_t first,uint64_t end) {
        if(end<=first||end>frames||end-first>uint64_t(INT32_MAX))throw std::runtime_error("PCM range out of bounds");
        std::vector<float> output(size_t(end-first));
        file.clear();file.seekg(offset+first*2);
        unsigned char bytes[8192];uint64_t done=0;
        while(done<end-first) {
            const size_t count=size_t(std::min<uint64_t>(4096,end-first-done));
            file.read(reinterpret_cast<char*>(bytes),std::streamsize(count*2));
            if(!file)throw std::runtime_error("short PCM read");
            for(size_t i=0;i<count;++i)output[size_t(done)+i]=float(int16_t(uint16_t(bytes[2*i])|(uint16_t(bytes[2*i+1])<<8)))/32768.0f;
            done+=count;
        }
        return output;
    }
};
static bool mon_read_audio_data(const std::string &path,std::vector<float> &pcm,std::vector<std::vector<float>> &channels,bool diarize) {
    try {
        if(diarize)return false;
        MonWave wave(path);const auto end=mon_pcm_end?mon_pcm_end:wave.frames;
        if(end-mon_pcm_first>124ULL*16000)return false;
        pcm=wave.read(mon_pcm_first,end);channels.clear();return true;
    } catch(const std::exception &e) {fprintf(stderr,"canonical PCM: %s\n",e.what());return false;}
}
