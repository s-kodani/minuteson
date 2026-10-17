#include "worker.hpp"
int mon_whisper(int,char **);
int main(int argc,char **argv) {
    return mon_loop(argc,argv,[](const std::string &kind,int n,char **a) { return kind == "whisper" ? mon_whisper(n,a) : 2; });
}
