#include "worker.hpp"
#include "log.h"
int llama_completion(int,char **);
void mon_llama_shutdown();
int main(int argc,char **argv) {
    const int result = mon_loop(argc,argv,[](const std::string &kind,int n,char **a) {
        if (kind != "llama") return 2;
        int result = llama_completion(n,a);
        common_log_flush(common_log_main());
        return result;
    });
    mon_llama_shutdown();
    common_log_flush(common_log_main());
    return result;
}
