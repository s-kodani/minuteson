#include "worker.hpp"
#include "log.h"
int llama_completion(int,char **);
void mon_llama_shutdown();
void mon_llama_mode(bool);
int mon_llama_tokenize(int,char **);
int mon_llama_memory(int,char **);
void mon_tokenizer_shutdown();
int main(int argc,char **argv) {
    const int result = mon_loop(argc,argv,[](const std::string &kind,int n,char **a) {
        if (kind == "llama_tokenize") return mon_llama_tokenize(n,a);
        if (kind == "llama_memory") return mon_llama_memory(n,a);
        if (kind != "llama" && kind != "llama_prefix") return 2;
        mon_llama_mode(kind == "llama_prefix");
        int result = llama_completion(n,a);
        common_log_flush(common_log_main());
        return result;
    });
    mon_tokenizer_shutdown();
    mon_llama_shutdown();
    common_log_flush(common_log_main());
    return result;
}
