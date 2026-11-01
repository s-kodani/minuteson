// Exercise the real log/protocol boundary without loading any model.
#include "worker.hpp"
static int infer(const std::string &, int, char **) {
    fprintf(stdout, "test unit completed\n");
    fprintf(stderr, "using device protocol-test\n");
    return 0;
}
int main(int argc, char **argv) { return mon_loop(argc, argv, infer); }
