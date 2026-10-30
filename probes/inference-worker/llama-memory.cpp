// Metadata-only allocation planning uses the exact pinned engine's model,
// cache and graph layouts. No tensors or KV buffers survive a request.
#include "common.h"
#include "arg.h"
#include "fit.h"
#include "ggml-backend.h"
#include "llama.h"
#include "log.h"
#include "json.hpp"
#include <algorithm>
#include <cstdint>
#include <cstdio>
#include <stdexcept>
#include <string>
#include <vector>

static uint64_t mon_memory_bytes(int64_t bytes) {
    if (bytes < 0) throw std::runtime_error("invalid device memory budget");
    return static_cast<uint64_t>(bytes);
}

int mon_llama_memory(int argc, char **argv) {
    try {
        common_params params;
        if (!common_params_parse(argc, argv, params, LLAMA_EXAMPLE_COMPLETION)) return 2;
        if (params.n_ctx < 2048 || params.n_ctx > 262144 || params.n_gpu_layers < 0 || params.n_gpu_layers > 256) {
            throw std::runtime_error("invalid memory estimate configuration");
        }
        // A CPU estimate must not implicitly choose an accelerator. The empty
        // device list also avoids Metal/Vulkan initialization by model loading.
        const bool cpu = params.n_gpu_layers == 0;
        static ggml_backend_dev_t no_devices[] = {nullptr};
        auto mparams = common_model_params_to_llama(params);
        auto cparams = common_context_params_to_llama(params);
        if (cpu) {
            mparams.devices = no_devices;
            mparams.n_gpu_layers = 0;
            cparams.offload_kqv = false;
            cparams.op_offload = false;
        } else if (params.devices.size() != 2 || params.devices[0] == nullptr || params.devices[1] != nullptr) {
            throw std::runtime_error("memory estimate requires one explicit GPU");
        }
        llama_backend_init();
        std::vector<ggml_backend_dev_t> devices;
        uint32_t layers = 0, trained_context = 0, experts = 0;
        // This engine API forces no_alloc and metadata-only model loading,
        // measures the full prospective model/context/compute allocations and
        // frees its temporary model/context before returning. It never touches
        // the completion worker's separately owned resident cache.
        auto memory = common_get_device_memory_data(params.model.path.c_str(), &mparams, &cparams,
                devices, layers, trained_context, experts, GGML_LOG_LEVEL_ERROR);
        if (devices.size() > 64 || memory.size() != devices.size() + 1 || (cpu && !devices.empty())) {
            throw std::runtime_error("invalid memory estimate device count");
        }
        nlohmann::ordered_json entries = nlohmann::ordered_json::array();
        for (size_t i = 0; i < memory.size(); ++i) {
            const bool host = i == devices.size();
            std::string name = host ? "CPU" : ggml_backend_dev_name(devices[i]);
            if (name.empty() || name.size() > 256) throw std::runtime_error("invalid memory estimate device name");
            const auto kind = host ? GGML_BACKEND_DEVICE_TYPE_CPU : ggml_backend_dev_type(devices[i]);
            const bool shared = !host && (kind == GGML_BACKEND_DEVICE_TYPE_IGPU ||
                std::string(ggml_backend_reg_name(ggml_backend_dev_backend_reg(devices[i]))) == "Metal");
            if (!host && kind != GGML_BACKEND_DEVICE_TYPE_GPU && kind != GGML_BACKEND_DEVICE_TYPE_IGPU) {
                throw std::runtime_error("unsupported memory estimate accelerator");
            }
            const auto &part = memory[i];
            const uint64_t total = mon_memory_bytes(part.total), free = mon_memory_bytes(part.free);
            if (free > total) throw std::runtime_error("invalid available device memory");
            entries.push_back({{"type", host ? "host" : kind == GGML_BACKEND_DEVICE_TYPE_IGPU ? "integratedGpu" : "gpu"},
                {"name", name}, {"sharedMemory", shared}, {"total", total}, {"free", free},
                {"modelBytes", static_cast<uint64_t>(part.model)},
                {"contextBytes", static_cast<uint64_t>(part.context)},
                {"computeBytes", static_cast<uint64_t>(part.compute)}});
        }
        const uint32_t effective_layers = cpu ? 0 : std::min(static_cast<uint32_t>(params.n_gpu_layers), layers + 1);
        const auto result = nlohmann::ordered_json({{"schemaVersion", 1},
            {"contextTokens", params.n_ctx}, {"gpuLayers", effective_layers}, {"devices", entries}}).dump() + "\n";
        if (result.size() > 32 * 1024) throw std::runtime_error("memory estimate output exceeds limit");
        common_log_flush(common_log_main());
        if (std::fwrite(result.data(), 1, result.size(), stdout) != result.size()) return 2;
        return 0;
    } catch (const std::exception &error) {
        std::fprintf(stderr, "mon_memory_estimate: %s\n", error.what());
        common_log_flush(common_log_main());
        return 2;
    }
}
