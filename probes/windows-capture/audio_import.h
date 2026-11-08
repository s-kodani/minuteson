#pragma once
// File decoding only: this entry point never opens a microphone or loopback.
#include <mfapi.h>
#include <mfidl.h>
#include <mfreadwrite.h>
#include <mferror.h>
#include <cstring>
#include "m4a_container.h"
#include "voice_pcm.h"
namespace audio_import {
using Json=nlohmann::json;
struct Output {
  HANDLE file=INVALID_HANDLE_VALUE;
  ~Output(){if(file!=INVALID_HANDLE_VALUE)CloseHandle(file);}
  void write(const void* data,std::size_t bytes){
    auto p=static_cast<const BYTE*>(data);
    while(bytes){DWORD done=0;DWORD chunk=static_cast<DWORD>(std::min<std::size_t>(bytes,1024*1024));
      if(!WriteFile(file,p,chunk,&done,nullptr)||!done)throw std::runtime_error("音声変換結果を書き込めません");p+=done;bytes-=done;}
  }
};
inline std::vector<BYTE> caf(UINT32 rate,UINT32 channels,std::uint64_t frames){
  std::vector<BYTE> h={'c','a','f','f',0,1,0,0,'d','e','s','c'};
  auto be=[&](std::uint64_t v,int bytes){for(int i=bytes-1;i>=0;--i)h.push_back(BYTE(v>>(8*i)));};
  be(32,8);double frequency=rate;std::uint64_t bits;std::memcpy(&bits,&frequency,8);be(bits,8);
  h.insert(h.end(),{'l','p','c','m'});for(auto n:{3u,channels*4,1u,channels,32u})be(n,4);
  h.insert(h.end(),{'d','a','t','a'});be(frames*channels*4+4,8);be(0,4);return h;
}
// Inspect ISO BMFF structure before allowing Media Foundation to decode it.
// Do not silently accept the audio track of a video or multi-track container.
inline void container(const fs::path& path){
  m4a_container::validate(path);
}
inline int run(int argc,wchar_t** argv){
  try{
    if(argc!=5)throw std::runtime_error("音声取り込みの引数が不正です");
    fs::path input=argv[2],target=argv[3];DWORD pid=DWORD(std::stoul(argv[4]));
    if(!input.is_absolute()||!pid||(target!=L"-"&&!target.is_absolute()))throw std::runtime_error("音声取り込みにはローカルの絶対パスが必要です");
    product::Handle parent(OpenProcess(SYNCHRONIZE,FALSE,pid));if(!parent.value)throw std::runtime_error("取り込みの親プロセスがありません");
    container(input);check(CoInitializeEx(nullptr,COINIT_MULTITHREADED),"initialize audio import COM");
    check(MFStartup(MF_VERSION,MFSTARTUP_NOSOCKET),"initialize Media Foundation");struct Shutdown{~Shutdown(){MFShutdown();CoUninitialize();}} shutdown;
    ComPtr<IMFAttributes> attributes;check(MFCreateAttributes(&attributes,2),"create decoder attributes");
    check(attributes->SetUINT32(MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING,FALSE),"disable video processing");
    // Source Reader treats the Win32 extended-path prefix as a network URL.
    // Its URL parser accepts the equivalent local DOS path, including Unicode.
    auto source=input.wstring();if(source.rfind(L"\\\\?\\",0)==0){source=source.rfind(L"\\\\?\\UNC\\",0)==0?L"\\\\"+source.substr(8):source.substr(4);}
    ComPtr<IMFSourceReader> reader;check(MFCreateSourceReaderFromURL(source.c_str(),attributes.Get(),&reader),"open M4A audio");
    ComPtr<IMFMediaType> native;check(reader->GetNativeMediaType(MF_SOURCE_READER_FIRST_AUDIO_STREAM,0,&native),"read audio format");
    GUID subtype;check(native->GetGUID(MF_MT_SUBTYPE,&subtype),"read audio codec");
    GUID alac_sample_entry=MFAudioFormat_ALAC;alac_sample_entry.Data1=0x616c6163;
    GUID alac_mpeg4={0x616c6163,0x767a,0x494d,{0xb4,0x78,0xf2,0x9d,0x25,0xdc,0x90,0x37}};
    const char* codec=nullptr;if(subtype==MFAudioFormat_AAC)codec="aac";else if(subtype==MFAudioFormat_ALAC||subtype==alac_sample_entry||subtype==alac_mpeg4)codec="alac";else throw std::runtime_error("M4AはAACまたはALAC音声にしてください");
    UINT32 rate=0,channels=0;check(native->GetUINT32(MF_MT_AUDIO_SAMPLES_PER_SECOND,&rate),"read sample rate");check(native->GetUINT32(MF_MT_AUDIO_NUM_CHANNELS,&channels),"read channel count");
    if(rate<8000||rate>384000||!channels||channels>8)throw std::runtime_error("M4Aの音声形式が対応範囲外です");
    check(reader->SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS,FALSE),"disable other streams");check(reader->SetStreamSelection(MF_SOURCE_READER_FIRST_AUDIO_STREAM,TRUE),"select audio stream");
    ComPtr<IMFMediaType> decoded;check(MFCreateMediaType(&decoded),"create decoded type");
    check(decoded->SetGUID(MF_MT_MAJOR_TYPE,MFMediaType_Audio),"select audio");check(decoded->SetGUID(MF_MT_SUBTYPE,MFAudioFormat_Float),"select float PCM");
    check(decoded->SetUINT32(MF_MT_AUDIO_SAMPLES_PER_SECOND,rate),"retain sample rate");check(decoded->SetUINT32(MF_MT_AUDIO_NUM_CHANNELS,channels),"retain channels");check(decoded->SetUINT32(MF_MT_AUDIO_BITS_PER_SAMPLE,32),"select PCM width");
    check(reader->SetCurrentMediaType(MF_SOURCE_READER_FIRST_AUDIO_STREAM,nullptr,decoded.Get()),"configure decoder");
    Output output;if(target!=L"-"){output.file=CreateFileW(target.c_str(),GENERIC_WRITE,FILE_SHARE_READ,nullptr,CREATE_NEW,FILE_ATTRIBUTE_NORMAL,nullptr);if(output.file==INVALID_HANDLE_VALUE)throw std::runtime_error("音声変換先を新規作成できません");auto h=VoicePCM::header(0);output.write(h.data(),h.size());}
    std::unique_ptr<VoicePCM> compact;if(output.file!=INVALID_HANDLE_VALUE)compact=std::make_unique<VoicePCM>(rate,channels);
    std::uint64_t saved_frames=0;
    auto save=[&](const std::int16_t* pcm,std::size_t count,std::int64_t,std::int64_t){output.write(pcm,count*2);saved_frames+=count;};
    std::uint64_t frames=0;auto started=GetTickCount64();
    while(true){
      if(WaitForSingleObject(parent.value,0)!=WAIT_TIMEOUT||GetTickCount64()-started>3600000)throw std::runtime_error("音声取り込みが中断されたか時間上限を超えました");
      DWORD flags=0;LONGLONG timestamp=0;ComPtr<IMFSample> sample;check(reader->ReadSample(MF_SOURCE_READER_FIRST_AUDIO_STREAM,0,nullptr,&flags,&timestamp,&sample),"decode M4A audio");
      if(flags&MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED)throw std::runtime_error("M4Aの音声形式が途中で変わりました");
      if(sample){ComPtr<IMFMediaBuffer> buffer;check(sample->ConvertToContiguousBuffer(&buffer),"read PCM buffer");BYTE* bytes=nullptr;DWORD count=0;check(buffer->Lock(&bytes,nullptr,&count),"lock PCM buffer");
        struct Unlock{IMFMediaBuffer* b;~Unlock(){b->Unlock();}} unlock{buffer.Get()};
        if(count>64*1024*1024||count%(channels*4))throw std::runtime_error("M4AのPCM区間が不正です");
        for(DWORD i=0;i<count;i+=4){float v;std::memcpy(&v,bytes+i,4);if(!std::isfinite(v))throw std::runtime_error("M4Aに非有限のPCM値があります");}
        const auto first=frames;frames+=count/(channels*4);if(frames>std::uint64_t(rate)*14400)throw std::runtime_error("音声は4時間以内にしてください");if(compact){
          const auto sample_count=count/4;
          // Native decoder blocks can be large; bound each shared conversion.
          for(std::size_t offset=0;offset<sample_count;){const auto n=std::min<std::size_t>(4096*channels,sample_count-offset);const auto from=first+offset/channels;const auto to=from+n/channels;
            std::vector<float> pcm(n);std::memcpy(pcm.data(),bytes+offset*4,n*4);
            compact->push(pcm.data(),n,static_cast<std::int64_t>(from*1000000000/rate),static_cast<std::int64_t>(to*1000000000/rate),save);offset+=n;}
        }
      }
      if(flags&MF_SOURCE_READERF_ENDOFSTREAM)break;
    }
    if(compact)compact->finish(save);
    if(!frames)throw std::runtime_error("M4Aに音声がありません");
    if(output.file!=INVALID_HANDLE_VALUE){LARGE_INTEGER zero={};if(!SetFilePointerEx(output.file,zero,nullptr,FILE_BEGIN))throw std::runtime_error("CAFの確定位置を設定できません");auto h=VoicePCM::header(saved_frames);output.write(h.data(),h.size());if(!FlushFileBuffers(output.file))throw std::runtime_error("CAFを同期できません");CloseHandle(output.file);output.file=INVALID_HANDLE_VALUE;}
    auto report=Json{{"schemaVersion",1},{"detectedFormat","m4a"},{"codec",codec},{"sampleRate",rate},{"channels",channels},{"frames",frames},{"durationUs",frames*1000000/rate}};
    if(compact){if(saved_frames!=(frames*16000+rate-1)/rate)throw std::runtime_error("compact import frame mismatch");report["savedAudio"]=Json{{"policy","voice_pcm16_16000_mono_v1"},{"sampleRate",16000},{"channels",1},{"frames",saved_frames}};}
    std::cout<<report.dump()<<std::endl;return 0;
  }catch(const std::exception& error){std::cout<<Json{{"errorMessage",error.what()}}.dump()<<std::endl;return 1;}
}
}
