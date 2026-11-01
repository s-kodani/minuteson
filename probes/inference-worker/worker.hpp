// MON's private, bounded sequential protocol. Never opens a network listener.
#pragma once
#include "json.hpp"
#include <chrono>
#include <cstdio>
#include <fstream>
#include <iostream>
#include <string>
#include <vector>
#ifdef _WIN32
#include <windows.h>
#include <io.h>
#include <process.h>
#include <filesystem>
#define STDOUT_FILENO 1
#define STDERR_FILENO 2
#define dup _dup
#define dup2 _dup2
#define close _close
#define write _write
#define getpid _getpid
static std::wstring mon_wide(const std::string &text) {
    int n=MultiByteToWideChar(CP_UTF8,MB_ERR_INVALID_CHARS,text.data(),int(text.size()),nullptr,0);
    if(!n)throw std::runtime_error("invalid UTF-8 path");std::wstring result(n,0);
    MultiByteToWideChar(CP_UTF8,MB_ERR_INVALID_CHARS,text.data(),int(text.size()),result.data(),n);return result;
}
#else
#include <unistd.h>
#endif
#include <fcntl.h>
#include <stdexcept>
using mon_json = nlohmann::ordered_json;
static int mon_protocol_fd = -1;
static void mon_reply(uint64_t id, const char *op, const std::string &message = "") {
    std::string line = mon_json{{"protocol",1},{"id",id},{"op",op},{"message",message}}.dump() + "\n";
    const char *data = line.data(); size_t remaining = line.size();
    while (remaining) { auto n = write(mon_protocol_fd,data,remaining); if (n <= 0) _exit(125); data += n; remaining -= n; }
}
static void mon_log_name(const std::string &name) {
    // Parent creates each unit's files exclusively before sending a request.
    if (name.empty() || name=="." || name==".." || name.find_first_of("/\\:") != std::string::npos) throw std::runtime_error("invalid log filename");
}
#ifdef _WIN32
static std::filesystem::path mon_log_path(const std::string &directory, const std::string &name) {
    mon_log_name(name);
    auto path=mon_wide(directory);
    if(!std::filesystem::path(path).is_absolute())throw std::runtime_error("invalid unit directory");
    // Relative Win32/CRT paths can still exceed MAX_PATH after resolving CWD.
    // Use the extended absolute namespace for opening and reading diagnostics.
    if(path.rfind(L"\\\\?\\",0)!=0) {
        if(path.rfind(L"\\\\",0)==0)path=L"\\\\?\\UNC\\"+path.substr(2);
        else path=L"\\\\?\\"+path;
    }
    return std::filesystem::path(path)/mon_wide(name);
}
#endif
static void mon_redirect(const std::string &directory, const std::string &name, int fd) {
    mon_log_name(name);
    #ifdef _WIN32
    auto path=mon_log_path(directory,name);HANDLE file=CreateFileW(path.c_str(),FILE_APPEND_DATA|FILE_READ_ATTRIBUTES,FILE_SHARE_READ|FILE_SHARE_WRITE|FILE_SHARE_DELETE,nullptr,OPEN_EXISTING,FILE_FLAG_OPEN_REPARSE_POINT,nullptr);
    BY_HANDLE_FILE_INFORMATION info{};
    if(file==INVALID_HANDLE_VALUE) {
        const DWORD error=GetLastError();
        throw std::runtime_error("cannot open existing unit log: "+name+" win32="+std::to_string(error)+" path_length="+std::to_string(path.native().size()));
    }
    if(!GetFileInformationByHandle(file,&info)||(info.dwFileAttributes&(FILE_ATTRIBUTE_REPARSE_POINT|FILE_ATTRIBUTE_DIRECTORY))||info.nNumberOfLinks!=1){CloseHandle(file);throw std::runtime_error("invalid unit log identity");}
    int opened=_open_osfhandle(reinterpret_cast<intptr_t>(file),_O_APPEND|_O_WRONLY|_O_BINARY);
    if(opened<0)CloseHandle(file);
    #else
    int opened = open(name.c_str(), O_WRONLY | O_APPEND | O_NOFOLLOW);
    #endif
    if (opened < 0) throw std::runtime_error("cannot redirect unit output");
    if (dup2(opened, fd) < 0) { close(opened); throw std::runtime_error("cannot redirect unit output"); }
    close(opened);
}
static int mon_loop(int argc, char **argv, int (*infer)(const std::string &,int,char **)) {
    if (argc != 2 || std::string(argv[1]) != "--mon-resident-v1") return 2;
    mon_protocol_fd = dup(STDOUT_FILENO);
    #ifdef _WIN32
    _setmode(mon_protocol_fd,_O_BINARY);_setmode(0,_O_BINARY);
    SetHandleInformation(reinterpret_cast<HANDLE>(_get_osfhandle(mon_protocol_fd)),HANDLE_FLAG_INHERIT,0);
    // llama's console initialization changes stdin to UTF-16/UTF-8 CRT text
    // mode. Read the private protocol through its own raw handle so subsequent
    // requests never depend on an engine's console or C runtime settings.
    HANDLE protocol_input=nullptr;
    if(!DuplicateHandle(GetCurrentProcess(),GetStdHandle(STD_INPUT_HANDLE),GetCurrentProcess(),&protocol_input,0,FALSE,DUPLICATE_SAME_ACCESS))return 2;
    struct InputCloser{HANDLE file;~InputCloser(){CloseHandle(file);}} input_closer{protocol_input};
    auto get=[&](char& c){DWORD count=0;return ReadFile(protocol_input,&c,1,&count,nullptr)&&count==1;};
    #else
    fcntl(mon_protocol_fd,F_SETFD,FD_CLOEXEC);
    auto get=[](char& c){return bool(std::cin.get(c));};
    #endif
    setvbuf(stdout,nullptr,_IONBF,0); setvbuf(stderr,nullptr,_IONBF,0);
    uint64_t previous = 0; bool initialized = false; std::string evidence;
    for (;;) {
        // std::getline alone permits unbounded allocation on a malformed peer.
        std::string line; char c;bool available=true;
        while ((available=get(c)) && c != '\n') { if (line.size() == 65536) return 2; line += c; }
        if (line.empty() && !available) return 0;
        uint64_t id = 0;
        try {
            const auto request = mon_json::parse(line);
            if (request.at("protocol") != 1) throw std::runtime_error("unsupported protocol");
            id = request.at("id").get<uint64_t>();
            const std::string op = request.at("op");
            if (op == "shutdown") return 0;
            if (op == "init" && !initialized) { initialized = true; mon_reply(id,"ready","canonical_pcm_range_v1"); continue; }
            if (!initialized || op != "infer" || id <= previous) throw std::runtime_error("invalid request order");
            previous = id;
            std::string directory = request.at("directory");
            #ifdef _WIN32
            auto native=mon_wide(directory);
            if(!std::filesystem::path(native).is_absolute()||!SetCurrentDirectoryW(native.c_str()))throw std::runtime_error("invalid unit directory");
            #else
            if (directory.empty() || directory[0] != '/' || chdir(directory.c_str())) throw std::runtime_error("invalid unit directory");
            #endif
            mon_redirect(directory,request.at("stdout"),STDOUT_FILENO);
            mon_redirect(directory,request.at("stderr"),STDERR_FILENO);
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
                #ifdef _WIN32
                std::ifstream logs(mon_log_path(directory,request.at("stderr")));
                #else
                std::ifstream logs(request.at("stderr").get<std::string>());
                #endif
                std::string row;
                while (std::getline(logs,row)) if (row.find("using device ") != std::string::npos || row.find("offloaded ") != std::string::npos) evidence += row + "\n";
            }
            #ifdef _WIN32
            // A completed unit is sealed by the parent with a read-only share
            // that excludes writers. Release both writable unit handles before
            // acknowledging completion, while retaining the loaded model.
            int sink=_open("NUL",_O_WRONLY|_O_BINARY|_O_NOINHERIT);
            if(sink<0)throw std::runtime_error("cannot release unit logs");
            const bool detached=dup2(sink,STDOUT_FILENO)==0&&dup2(sink,STDERR_FILENO)==0;
            close(sink);
            if(!detached)throw std::runtime_error("cannot release unit logs");
            #endif
            mon_reply(id,"done");
        } catch (const std::exception &error) { mon_reply(id,"error",error.what()); return 2; }
    }
}
