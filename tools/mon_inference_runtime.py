"""Build the three offline MON workers from reviewed, SHA-pinned upstreams.

The single-shot model packages are untouched. Sources and compiler products are
build-host caches; only signed workers, notices and their index ship in the app.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import shutil
import tarfile
import tempfile
if os.name != 'nt':
    import mon_whisper_runtime as asr
else:
    # Windows consumes the shared source transformations, not the Mac builder.
    asr = None

ROOT = Path(__file__).resolve().parents[1]
LLAMA_COMMIT = "edd6e2bbdad5930899a93db8fa73c3b61c7b9bcc"
LLAMA_SHA = "12f869f66555d409954d8d68d7fd85a308e4fa42e25e0631e9776f23cc898234"
LLAMA_BYTES = 37840022
SHERPA_COMMIT = "11afbd009a7f8c08f4bcf2fc1b265d0df4670fbf"
SHERPA_LICENSE_SHA = "cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30"
HEADER_SHA = "2a1b95084be8fd1deb3228fcad2fd3f7f0258b64582f7402281ec174c7b7f4ce"
NAMES = {"whisper": "mon-whisper-worker", "llama": "mon-llama-worker", "sherpa": "mon-sherpa-worker"}


def checked(path, sha, size=None):
    asr.private(path)
    if (size is not None and path.stat().st_size != size) or asr.digest(path) != sha:
        raise ValueError("pinned inference source differs")


def acquire(cache, name, url, sha, size=None, supplied=None):
    target = cache / name
    if not target.exists():
        pending = cache / (name + ".pending")
        if supplied:
            checked(supplied, sha, size)
            shutil.copyfile(supplied, pending)
        else:
            asr.command(["/usr/bin/curl", "--disable", "--silent", "--show-error", "--fail", "--location", "--max-filesize", str(size or 1024*1024), url, "--output", str(pending)], timeout=180)
        checked(pending, sha, size)
        os.replace(pending, target)
    checked(target, sha, size)
    return target


def extract(archive, destination):
    destination.mkdir(mode=0o700)
    total = 0
    with tarfile.open(archive) as source:
        members = source.getmembers()
        if len(members) > 40000:
            raise ValueError("too many source members")
        for item in members:
            p = PurePosixPath(item.name)
            if p.is_absolute() or ".." in p.parts or not (item.isdir() or item.isfile()):
                raise ValueError("unsafe source member")
            total += item.size
            if total > 512*1024*1024:
                raise ValueError("source expansion exceeded")
        source.extractall(destination, filter="data")
    return next(destination.iterdir())


def replace(text, old, new):
    if text.count(old) != 1:
        raise ValueError("upstream worker patch no longer matches exactly")
    return text.replace(old, new)


def patch_whisper(root):
    source = root / "examples/cli/cli.cpp"
    text = source.read_text(encoding="utf8")
    text = replace(text, "int main(int argc, char ** argv)", "int mon_whisper(int argc, char ** argv)")
    text = replace(text, "struct whisper_context * ctx = whisper_init_from_file_with_params(params.model.c_str(), cparams);", '''static whisper_context * cached = nullptr;
    static std::string model_key;
    const std::string key = params.model + (params.use_gpu ? ":gpu" : ":cpu");
    if (cached && model_key != key) return 3;
    if (!cached) {
        const auto start = std::chrono::steady_clock::now();
        cached = whisper_init_from_file_with_params(params.model.c_str(), cparams);
        model_key = key;
        fprintf(stderr, "mon_model_load: whisper=1 load_ms=%.3f\\n", std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now()-start).count());
        if (cached) atexit([] { whisper_free(cached); });
    }
    struct whisper_context * ctx = cached;
    if (ctx) whisper_reset_timings(ctx);''')
    text = replace(text, "    whisper_free(ctx);", "    // The worker owns the model until shutdown; whisper_full resets unit state.")
    text = text.replace('#include "whisper.h"', '#include "whisper.h"\n#include <chrono>')
    source.write_text(text,encoding="utf8")
    with (root / "CMakeLists.txt").open("a") as output:
        output.write('''\nadd_executable(mon-whisper-worker examples/cli/cli.cpp mon/whisper-main.cpp)
target_include_directories(mon-whisper-worker PRIVATE mon examples)
target_link_libraries(mon-whisper-worker PRIVATE common whisper)
target_compile_features(mon-whisper-worker PRIVATE cxx_std_17)
''')


def patch_llama(root):
    source = root / "tools/completion/completion.cpp"
    text = source.read_text(encoding="utf8")
    # Reuse the constructor's exact model-derived sampler preparation on every
    # subsequent request too (GGUF defaults, EOG biases), while keeping the new
    # request's grammar and seed. Do not approximate those defaults in MON.
    common = root / "common/common.cpp"
    body = common.read_text(encoding="utf8")
    first = body.index("    // updates params.sampling\n")
    last = body.index("    // init the backend samplers", first)
    sampling = body[first:last].replace("params.sampling", "sparams")
    body += "\nvoid mon_request_sampling_defaults(llama_model * model, common_params_sampling & sparams) {\n    const llama_vocab * vocab = llama_model_get_vocab(model);\n" + sampling + "}\n"
    common.write_text(body,encoding="utf8")
    text = replace(text, "int llama_completion(int argc, char ** argv);", "void mon_request_sampling_defaults(llama_model *, common_params_sampling &);\nint llama_completion(int argc, char ** argv);")
    text = replace(text, "int llama_completion(int argc, char ** argv) {", """// Destroy cached GPU resources explicitly before backend static destructors.
static common_init_result_ptr mon_cached;
static bool mon_prefix_enabled=false;
static std::vector<llama_token> mon_prefix_tokens;
void mon_llama_mode(bool enabled) { mon_prefix_enabled=enabled; if(!enabled) mon_prefix_tokens.clear(); }
void mon_llama_shutdown() { mon_cached.reset(); llama_backend_free(); }

int llama_completion(int argc, char ** argv) {""")
    text = replace(text, "    common_init();", "    static const bool initialized = [] { common_init(); llama_backend_init(); return true; }();\n    (void) initialized;")
    text = replace(text, "    atexit([]() { console::cleanup(); });", "    static const bool cleanup = [] { atexit([] { console::cleanup(); }); return true; }();\n    (void) cleanup;")
    text = replace(text, '    LOG_INF("%s: load the model and apply lora adapter, if any\\n", __func__);', '    LOG_INF("%s: prepare job inference\\n", __func__);')
    text = replace(text, '    LOG_INF("%s: llama backend init\\n", __func__);', '    LOG_INF("%s: job inference backend\\n", __func__);')
    text = replace(text, "    llama_backend_init();", "    // Backend initialized once by the worker above.")
    text = replace(text, "    auto llama_init = common_init_from_params(params);", '''auto & cached = mon_cached;
    static std::string model_key;
    // The GPU probe requests an upper bound (256); admission records the
    // actual offloaded count. llama.cpp clamps both to n_layer + 1, so compare
    // effective placement rather than the original upper bound.
    const auto key_for = [&](llama_model * loaded) {
        const int layers = loaded ? std::min(params.n_gpu_layers, llama_model_n_layer(loaded) + 1) : params.n_gpu_layers;
        return params.model.path + ":" + std::to_string(params.n_ctx) + ":" + std::to_string(layers);
    };
    if (cached && model_key != key_for(cached->model())) {
        LOG_ERR("%s: resident model/context/GPU placement differs from initialization\\n", __func__);
        return 3;
    }
    const bool reused_model = bool(cached);
    if (!cached) {
        cached = common_init_from_params(params);
        if (cached) model_key = key_for(cached->model());
        fprintf(stderr, "mon_model_load: llama=1\\n");
    }
    auto & llama_init = cached;''')
    text = replace(text, "    smpl  = llama_init->sampler(0);", '''    // Each request gets fresh RNG, grammar and repetition history.
    if (!ctx || !model) return 1;
    if (reused_model) mon_request_sampling_defaults(model, sparams);
    common_sampler_ptr request_sampler(common_sampler_init(model, sparams));
    smpl = request_sampler.get();
    if (!smpl) return 1;
    is_interacting = false;
    need_insert_eot = false;
    if (ctx && !mon_prefix_enabled) llama_memory_clear(llama_get_memory(ctx), true);''')
    text = replace(text, "    bool session_do_save = false;", r'''    size_t mon_system_tokens = 0;
    if (mon_prefix_enabled) {
        common_chat_templates_inputs in;
        in.use_jinja=params.use_jinja; in.force_pure_content=params.force_pure_content_parser;
        if (!params.system_prompt.empty()) { common_chat_msg m; m.role="system"; m.content=params.system_prompt; in.messages.push_back(m); }
        in.add_generation_prompt=false;
        auto system_tokens=common_tokenize(ctx,common_chat_templates_apply(chat_templates.get(),in).prompt,true,true);
        while(mon_system_tokens<system_tokens.size() && mon_system_tokens<embd_inp.size() && system_tokens[mon_system_tokens]==embd_inp[mon_system_tokens]) ++mon_system_tokens;
        size_t keep=0;
        while(keep<mon_system_tokens && keep<mon_prefix_tokens.size() && mon_prefix_tokens[keep]==embd_inp[keep]) ++keep;
        if(!llama_memory_seq_rm(mem,-1,keep ? keep-1 : 0,-1)) { llama_memory_clear(mem,true); keep=0; }
        session_tokens.assign(mon_prefix_tokens.begin(),mon_prefix_tokens.begin()+keep);
        fprintf(stderr,"mon_prefix_cache: tokens=%zu\n",keep);
    }
    bool session_do_save = false;''')
    final_return=text.rfind("    return 0;\n}")
    if final_return < 0: raise ValueError("completion return missing")
    text=text[:final_return]+"    if(mon_prefix_enabled) mon_prefix_tokens.assign(embd_inp.begin(),embd_inp.begin()+mon_system_tokens);\n"+text[final_return:]
    text = replace(text, "    llama_backend_free();", "    // Model/context lifetime belongs to the worker.")
    source.write_text(text,encoding="utf8")
    with (root / "CMakeLists.txt").open("a") as output:
        output.write('''\nadd_executable(mon-llama-worker tools/completion/completion.cpp mon/llama-main.cpp mon/llama-tokenize.cpp)
target_include_directories(mon-llama-worker PRIVATE mon common)
target_link_libraries(mon-llama-worker PRIVATE llama-common llama)
target_compile_features(mon-llama-worker PRIVATE cxx_std_17)
add_executable(mon-sherpa-worker mon/sherpa.cpp)
target_include_directories(mon-sherpa-worker PRIVATE mon)
target_compile_features(mon-sherpa-worker PRIVATE cxx_std_17)
''')


def recipe_hash():
    h = hashlib.sha256(Path(__file__).read_bytes())
    for p in sorted((ROOT / "probes/inference-worker").iterdir()):
        if p.is_file() and p.suffix in (".cpp", ".hpp"):
            h.update(p.name.encode()); h.update(p.read_bytes())
    h.update((ROOT/"third-party/notices/spdx-MIT-template.txt").read_bytes())
    return h.hexdigest()


def prepare(cache, cmake, whisper_source=None, llama_source=None, sherpa_header=None, jobs=4, sherpa_license=None):
    cache.mkdir(parents=True, exist_ok=True)
    cache = cache.resolve()
    with asr.lock(cache):
        package = cache / "package"
        stamp = recipe_hash()
        if package.exists() and (package / "build-info.json").exists():
            info = json.loads((package / "build-info.json").read_text(encoding="utf8"))
            if info.get("recipeSha256") == stamp:
                verify(package)
                return package
        whisper = acquire(cache,"whisper-source.tar.gz",asr.SOURCE_URL,asr.SOURCE_SHA,asr.SOURCE_BYTES,whisper_source)
        llama = acquire(cache,"llama-source.tar.gz",f"https://codeload.github.com/ggml-org/llama.cpp/tar.gz/{LLAMA_COMMIT}",LLAMA_SHA,LLAMA_BYTES,llama_source)
        header = acquire(cache,"c-api.h",f"https://raw.githubusercontent.com/k2-fsa/sherpa-onnx/{SHERPA_COMMIT}/sherpa-onnx/c-api/c-api.h",HEADER_SHA,supplied=sherpa_header)
        license_file = acquire(cache,"sherpa-LICENSE",f"https://raw.githubusercontent.com/k2-fsa/sherpa-onnx/{SHERPA_COMMIT}/LICENSE",SHERPA_LICENSE_SHA,11358,sherpa_license)
        with tempfile.TemporaryDirectory(prefix="build-", dir=cache) as temporary:
            work = Path(temporary)
            roots = {"whisper": extract(whisper,work/"whisper"),"llama":extract(llama,work/"llama")}
            for root in roots.values():
                shutil.copytree(ROOT / "probes/inference-worker",root/"mon")
                shutil.copyfile(roots["llama"]/"vendor/nlohmann/json.hpp",root/"mon/json.hpp")
                shutil.copyfile(header,root/"mon/c-api.h")
            patch_whisper(roots["whisper"]); patch_llama(roots["llama"])
            pending = work / "package"; pending.mkdir(mode=0o700)
            env = asr.environment(cmake)
            env["TMPDIR"] = str(work)
            for engine, root in roots.items():
                build = work/(engine+"-build")
                options = list(asr.OPTIONS) if engine == "whisper" else [v for v in asr.OPTIONS if "WHISPER" not in v]
                if engine == "llama":
                    options += ["-DLLAMA_BUILD_COMMON=ON","-DLLAMA_BUILD_TESTS=OFF","-DLLAMA_BUILD_TOOLS=OFF","-DLLAMA_BUILD_EXAMPLES=OFF","-DLLAMA_BUILD_APP=OFF","-DLLAMA_BUILD_MTMD=OFF","-DLLAMA_OPENSSL=OFF"]
                asr.command([str(cmake),"-S",str(root),"-B",str(build),*options],env=env)
                targets = [NAMES[engine]] + ([NAMES["sherpa"]] if engine == "llama" else [])
                asr.command([str(cmake),"--build",str(build),"--target",*targets,"--parallel",str(jobs)],env=env,timeout=1800)
                for target in targets:
                    binary = build/"bin"/target
                    if not binary.exists(): binary = build/target
                    shutil.copyfile(binary,pending/target); (pending/target).chmod(0o700)
                    asr.command(["/usr/bin/codesign","--force","--sign","-",str(pending/target)])
                    asr.verify_binary(pending/target)
            index = [{"protocol":1,"engine":engine,"name":name,"bytes":(pending/name).stat().st_size,"sha256":asr.digest(pending/name)} for engine,name in NAMES.items()]
            (pending/"runtime-index.json").write_text(json.dumps(index,indent=2)+"\n")
            (pending/"build-info.json").write_text(json.dumps({"recipeSha256":stamp,"whisperCommit":asr.COMMIT,"llamaCommit":LLAMA_COMMIT,"sherpaCommit":SHERPA_COMMIT,"protocol":1,"licenseReview":"not_verified","distributionReady":False},indent=2)+"\n")
            # Include the original source notices alongside the derived workers.
            for name, path in [("whisper-MIT.txt",roots["whisper"]/"LICENSE"),("llama-MIT.txt",roots["llama"]/"LICENSE"),("nlohmann-MIT.txt",roots["llama"]/"vendor/nlohmann/json.hpp")]:
                if name.startswith("nlohmann"):
                    header = path.read_text(encoding="utf8").split("#ifndef")[0]
                    license_text = (ROOT/"third-party/notices/spdx-MIT-template.txt").read_text(encoding="utf8").replace("<year> <copyright holders>", "2013 - 2025 Niels Lohmann")
                    (pending/name).write_text(header + "\n" + license_text)
                else: shutil.copyfile(path,pending/name)
            shutil.copyfile(license_file,pending/"sherpa-Apache-2.0.txt")
            # Build only publication: existing installed worker bytes live in
            # the application's hash cache and never get overwritten here.
            if package.exists(): shutil.rmtree(package)
            shutil.copytree(pending,package)
            verify(package)
            return package


def verify(package):
    index = json.loads((package / "runtime-index.json").read_text(encoding="utf8"))
    if len(index) != 3 or {i["engine"] for i in index} != set(NAMES):
        raise ValueError("invalid worker index")
    for item in index:
        if item["name"] != NAMES[item["engine"]] or item["protocol"] != 1:
            raise ValueError("invalid worker identity")
        checked(package/item["name"],item["sha256"],item["bytes"])
        asr.verify_binary(package/item["name"])
    return index


def main():
    parser=argparse.ArgumentParser()
    parser.add_argument("--cache-dir",type=Path,default=ROOT/".cache/inference-runtime")
    parser.add_argument("--cmake",type=Path,default=Path(shutil.which("cmake") or "/usr/bin/cmake"))
    parser.add_argument("--whisper-source",type=Path)
    parser.add_argument("--llama-source",type=Path)
    parser.add_argument("--sherpa-header",type=Path)
    parser.add_argument("--sherpa-license",type=Path)
    parser.add_argument("--jobs",type=int,default=4)
    args=parser.parse_args()
    print(prepare(args.cache_dir,args.cmake,args.whisper_source,args.llama_source,args.sherpa_header,args.jobs,args.sherpa_license))

if __name__ == "__main__": main()
