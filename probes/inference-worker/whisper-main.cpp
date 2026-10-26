#include "worker.hpp"
#include "pcm-input.hpp"
int mon_whisper(int,char **);
int main(int argc,char **argv) {
    return mon_loop(argc,argv,[](const std::string &kind,int n,char **a) {
        if(kind!="whisper")return 2;
        mon_pcm_first=mon_pcm_end=0;
        std::vector<char*> args;
        for(int i=0;i<n;++i) {
            const std::string arg=a[i];
            if(arg=="--mon-first-frame"||arg=="--mon-end-frame") {
                if(i+1>=n)return 2;
                const auto value=std::stoull(a[++i]);
                if(arg=="--mon-first-frame")mon_pcm_first=value;else mon_pcm_end=value;
            } else args.push_back(a[i]);
        }
        args.push_back(nullptr);
        return mon_whisper(int(args.size()-1),args.data());
    });
}
