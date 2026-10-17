// MON's private, bounded sequential protocol. Never opens a network listener.
#pragma once
#include "json.hpp"
#include <chrono>
#include <cstdio>
#include <fstream>
#include <iostream>
#include <string>
#include <vector>
#include <unistd.h>
#include <fcntl.h>
#include <stdexcept>
using mon_json = nlohmann::ordered_json;
static int mon_protocol_fd = -1;
static void mon_reply(uint64_t id, const char *op, const std::string &message = "") {
    std::string line = mon_json{{"protocol",1},{"id",id},{"op",op},{"message",message}}.dump() + "\n";
    const char *data = line.data(); size_t remaining = line.size();
    while (remaining) { auto n = write(mon_protocol_fd,data,remaining); if (n <= 0) _exit(125); data += n; remaining -= n; }
}
static void mon_redirect(const std::string &name, int fd) {
    // Parent creates each unit's files exclusively before sending a request.
    if (name.find('/') != std::string::npos) throw std::runtime_error("invalid log filename");
    int opened = open(name.c_str(), O_WRONLY | O_APPEND | O_NOFOLLOW);
    if (opened < 0 || dup2(opened, fd) < 0) throw std::runtime_error("cannot redirect unit output");
    close(opened);
}
static int mon_loop(int argc, char **argv, int (*infer)(const std::string &,int,char **)) {
    if (argc != 2 || std::string(argv[1]) != "--mon-resident-v1") return 2;
    mon_protocol_fd = dup(STDOUT_FILENO);
    fcntl(mon_protocol_fd,F_SETFD,FD_CLOEXEC);
    setvbuf(stdout,nullptr,_IONBF,0); setvbuf(stderr,nullptr,_IONBF,0);
    uint64_t previous = 0; bool initialized = false; std::string evidence;
    for (;;) {
        // std::getline alone permits unbounded allocation on a malformed peer.
        std::string line; char c;
        while (std::cin.get(c) && c != '\n') { if (line.size() == 65536) return 2; line += c; }
        if (line.empty() && !std::cin) return 0;
        uint64_t id = 0;
        try {
            const auto request = mon_json::parse(line);
            if (request.at("protocol") != 1) throw std::runtime_error("unsupported protocol");
            id = request.at("id").get<uint64_t>();
            const std::string op = request.at("op");
            if (op == "shutdown") return 0;
            if (op == "init" && !initialized) { initialized = true; mon_reply(id,"ready"); continue; }
            if (!initialized || op != "infer" || id <= previous) throw std::runtime_error("invalid request order");
            previous = id;
            std::string directory = request.at("directory");
            if (directory.empty() || directory[0] != '/' || chdir(directory.c_str())) throw std::runtime_error("invalid unit directory");
            mon_redirect(request.at("stdout"),STDOUT_FILENO);
            mon_redirect(request.at("stderr"),STDERR_FILENO);
            auto args = request.at("args").get<std::vector<std::string>>();
            args.insert(args.begin(),"mon-resident-engine");
            std::vector<char *> pointers; for (auto &arg:args) pointers.push_back(arg.data()); pointers.push_back(nullptr);
            fprintf(stderr,"mon_session: pid=%d request=%llu %s\n",getpid(),(unsigned long long)id,id == 1 ? "initializing model" : "reusing model");
            // This is evidence from the successful model initialization, not a
            // request to re-select a GPU on each unit.
            if (!evidence.empty()) fprintf(stderr,"%s",evidence.c_str());
            mon_reply(id,"progress");
            const int result = infer(request.at("kind"),int(args.size()),pointers.data());
            fflush(nullptr);
            if (result != 0) { mon_reply(id,"error","native inference exit " + std::to_string(result)); return result; }
            if (evidence.empty()) {
                std::ifstream logs(request.at("stderr").get<std::string>()); std::string row;
                while (std::getline(logs,row)) if (row.find("using device ") != std::string::npos || row.find("offloaded ") != std::string::npos) evidence += row + "\n";
            }
            mon_reply(id,"done");
        } catch (const std::exception &error) { mon_reply(id,"error",error.what()); return 2; }
    }
}
