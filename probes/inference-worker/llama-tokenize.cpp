// The exact pinned completion tokenizer/template, without loading tensors.
#include "common.h"
#include "arg.h"
#include "chat.h"
#include "llama.h"
#include "json.hpp"
#include <cstdio>
#include <string>
#include <vector>
static llama_model * vocabulary_model = nullptr;
static std::string vocabulary_path;
void mon_tokenizer_shutdown() { if (vocabulary_model) llama_model_free(vocabulary_model); vocabulary_model=nullptr; }
static std::vector<llama_token> tokens(const llama_vocab *vocab,const std::string &text) {
    int n=llama_tokenize(vocab,text.data(),text.size(),nullptr,0,true,true);
    if (n>=0) return {};
    std::vector<llama_token> out(-n);
    n=llama_tokenize(vocab,text.data(),text.size(),out.data(),out.size(),true,true);
    if(n<0) throw std::runtime_error("tokenization failed");out.resize(n);return out;
}
int mon_llama_tokenize(int argc,char **argv) {
    common_params params;
    if (!common_params_parse(argc,argv,params,LLAMA_EXAMPLE_COMPLETION)) return 2;
    if(params.prompt.size()+params.system_prompt.size()>65536) return 2;
    if(!vocabulary_model || vocabulary_path!=params.model.path) {
        mon_tokenizer_shutdown();auto settings=llama_model_default_params();settings.vocab_only=true;
        vocabulary_model=llama_model_load_from_file(params.model.path.c_str(),settings);vocabulary_path=params.model.path;
        if(!vocabulary_model)return 3;
    }
    auto templates=common_chat_templates_init(vocabulary_model,params.chat_template);
    common_chat_templates_inputs input;input.use_jinja=params.use_jinja;input.force_pure_content=params.force_pure_content_parser;
    if(!params.system_prompt.empty()){common_chat_msg m;m.role="system";m.content=params.system_prompt;input.messages.push_back(m);}
    input.add_generation_prompt=false;
    auto prefix=tokens(llama_model_get_vocab(vocabulary_model),common_chat_templates_apply(templates.get(),input).prompt);
    if(!params.prompt.empty()){common_chat_msg m;m.role="user";m.content=params.prompt;input.messages.push_back(m);}
    input.add_generation_prompt=!params.prompt.empty();
    auto all=tokens(llama_model_get_vocab(vocabulary_model),common_chat_templates_apply(templates.get(),input).prompt);
    size_t match=0;while(match<prefix.size() && match<all.size() && prefix[match]==all[match])++match;
    std::printf("{\"schemaVersion\":1,\"tokens\":%zu,\"systemPrefixTokens\":%zu}\n",all.size(),match);return 0;
}
