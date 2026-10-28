#pragma once
#include "c-api.h"
#ifndef _WIN32
#include <dlfcn.h>
#endif
#include <map>
#include <memory>
#include <cmath>
#include <cstring>
#include <filesystem>
static void *library = nullptr;
template <typename T> static T symbol(const char *name) {
    #ifdef _WIN32
    auto pointer=GetProcAddress(reinterpret_cast<HMODULE>(library),name);
    #else
    auto pointer = dlsym(library,name);
    #endif
    if (!pointer) throw std::runtime_error(std::string("missing ABI symbol: ")+name);
    return reinterpret_cast<T>(pointer);
}
#define API(name) symbol<decltype(&name)>(#name)
static std::string library_path;
static std::map<std::string,std::string> options(int argc,char **argv) {
    std::map<std::string,std::string> result;
    for (int i=1;i<argc;++i) {
        std::string arg=argv[i]; auto equal=arg.find('=');
        if (equal!=std::string::npos) result[arg.substr(0,equal)]=arg.substr(equal+1);
        else if (arg.rfind("--",0)==0) { if (i+1>=argc) throw std::runtime_error("missing option"); result[arg]=argv[++i]; }
        else result["wave"]=arg;
    }
    return result;
}
static void load(const std::string &path) {
    if (library) { if (path!=library_path) throw std::runtime_error("runtime changed"); return; }
    #ifdef _WIN32
    auto native=mon_wide(path);
    if(!std::filesystem::path(native).is_absolute()||std::filesystem::path(native).filename()!=L"sherpa-onnx-c-api.dll")throw std::runtime_error("invalid pinned sherpa library");
    library=LoadLibraryExW(native.c_str(),nullptr,LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR|LOAD_LIBRARY_SEARCH_DEFAULT_DIRS);
    #else
    library = dlopen(path.c_str(),RTLD_NOW|RTLD_LOCAL);
    #endif
    if (!library) throw std::runtime_error("cannot load sherpa C API");
    library_path=path;
    if (std::string(API(SherpaOnnxGetVersionStr)()) != "1.13.8") throw std::runtime_error("unsupported sherpa ABI");
}
