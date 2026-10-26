#include "pcm-input.hpp"
#include <cassert>
#include <chrono>
static void le32(std::ofstream &f,uint32_t v){for(int i=0;i<4;++i)f.put(char(v>>(8*i)));}
static void le64(std::ofstream &f,uint64_t v){le32(f,uint32_t(v));le32(f,uint32_t(v>>32));}
static void fixture(const std::filesystem::path &p,bool rf64) {
    std::ofstream f(p,std::ios::binary);
    f.write(rf64?"RF64":"RIFF",4);le32(f,rf64?UINT32_MAX:44);f.write("WAVE",4);
    if(rf64){f.write("ds64",4);le32(f,28);le64(f,80);le64(f,8);le64(f,4);le32(f,0);}
    f.write("fmt ",4);le32(f,16);f.put(1);f.put(0);f.put(1);f.put(0);le32(f,16000);le32(f,32000);f.put(2);f.put(0);f.put(16);f.put(0);
    f.write("data",4);le32(f,rf64?UINT32_MAX:8);
    for(int16_t sample:{int16_t(0),int16_t(8192),int16_t(-16384),int16_t(32767)}) {f.put(char(uint16_t(sample)));f.put(char(uint16_t(sample)>>8));}
}
int main() {
    const auto directory=std::filesystem::temp_directory_path()/std::filesystem::path("mon-pcm-"+std::to_string(std::chrono::steady_clock::now().time_since_epoch().count()));
    std::filesystem::create_directory(directory);
    for(bool extended:{false,true}) {
        const auto file=directory/(extended?"rf64.wav":"riff.wav");fixture(file,extended);
        MonWave wave(file.u8string());assert(wave.frames==4);assert(wave.offset==(extended?80:44));
        const auto pcm=wave.read(1,3);assert(pcm.size()==2&&pcm[0]==0.25f&&pcm[1]==-0.5f);
        bool rejected=false;try{wave.read(3,5);}catch(...){rejected=true;}assert(rejected);
        mon_pcm_first=1;mon_pcm_end=3;std::vector<float> decoded;std::vector<std::vector<float>> channels;
        assert(mon_read_audio_data(file.u8string(),decoded,channels,false));assert(decoded==pcm);
        std::filesystem::resize_file(file,extended?87:51);
        rejected=false;try{MonWave invalid(file.u8string());}catch(...){rejected=true;}assert(rejected);
    }
    std::filesystem::remove(directory/"riff.wav");std::filesystem::remove(directory/"rf64.wav");std::filesystem::remove(directory);
}
