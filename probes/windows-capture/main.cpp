#include <windows.h>
#include <audioclient.h>
#include <audioclientactivationparams.h>
#include <mmdeviceapi.h>
#include <wrl/client.h>
#include <wrl/implements.h>

#include "segmented_wave_writer.h"

#include <atomic>
#include <chrono>
#include <cstdint>
#include <filesystem>
#include <fstream>
#include <iomanip>
#include <iostream>
#include <limits>
#include <memory>
#include <sstream>
#include <stdexcept>
#include <string>
#include <thread>
#include <vector>

using Microsoft::WRL::ComPtr;
namespace fs = std::filesystem;

namespace {

void check(HRESULT result, const char* operation) {
  if (FAILED(result)) {
    std::ostringstream message;
    message << operation << " failed: HRESULT 0x" << std::hex
            << static_cast<unsigned long>(result);
    throw std::runtime_error(message.str());
  }
}

std::string utf8(const std::wstring& value) {
  if (value.empty()) return {};
  int count = WideCharToMultiByte(CP_UTF8, 0, value.data(),
                                  static_cast<int>(value.size()), nullptr, 0, nullptr, nullptr);
  if (!count) throw std::runtime_error("could not encode UTF-8");
  std::string result(count, '\0');
  WideCharToMultiByte(CP_UTF8, 0, value.data(), static_cast<int>(value.size()),
                      result.data(), count, nullptr, nullptr);
  return result;
}

std::string json_escape(const std::string& value) {
  std::ostringstream output;
  for (unsigned char ch : value) {
    if (ch == '"' || ch == '\\') output << '\\' << ch;
    else if (ch < 0x20) output << "\\u" << std::hex << std::setw(4)
                               << std::setfill('0') << static_cast<int>(ch) << std::dec;
    else output << ch;
  }
  return output.str();
}

struct CoTaskMemFreeDeleter {
  void operator()(WAVEFORMATEX* value) const { CoTaskMemFree(value); }
};
using WaveFormat = std::unique_ptr<WAVEFORMATEX, CoTaskMemFreeDeleter>;

class ActivationHandler final : public Microsoft::WRL::RuntimeClass<
    Microsoft::WRL::RuntimeClassFlags<Microsoft::WRL::ClassicCom>,
    IActivateAudioInterfaceCompletionHandler,Microsoft::WRL::FtmBase> {
 public:
  ActivationHandler() : completed_(CreateEventW(nullptr, TRUE, FALSE, nullptr)) {
    if (!completed_) throw std::runtime_error("could not create activation event");
  }
  ~ActivationHandler() { CloseHandle(completed_); }

  HRESULT STDMETHODCALLTYPE ActivateCompleted(
      IActivateAudioInterfaceAsyncOperation* operation) override {
    HRESULT activation = E_FAIL;
    ComPtr<IUnknown> unknown;
    result_ = operation->GetActivateResult(&activation, unknown.GetAddressOf());
    if (SUCCEEDED(result_)) result_ = activation;
    if (SUCCEEDED(result_)) result_ = unknown.As(&client_);
    SetEvent(completed_);
    return S_OK;
  }

  HANDLE event() const { return completed_; }
  HRESULT result() const { return result_; }
  ComPtr<IAudioClient> client() const { return client_; }

 private:
  HANDLE completed_ = nullptr;
  HRESULT result_ = E_PENDING;
  ComPtr<IAudioClient> client_;
};

ComPtr<IAudioClient> activate_process_loopback(DWORD process_id,bool exclude_tree=false,DWORD wait_ms=15000) {
  AUDIOCLIENT_ACTIVATION_PARAMS activation = {};
  activation.ActivationType = AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK;
  activation.ProcessLoopbackParams.TargetProcessId = process_id;
  activation.ProcessLoopbackParams.ProcessLoopbackMode =
      exclude_tree?PROCESS_LOOPBACK_MODE_EXCLUDE_TARGET_PROCESS_TREE:
        PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE;
  PROPVARIANT parameters = {};
  parameters.vt = VT_BLOB;
  parameters.blob.cbSize = sizeof(activation);
  parameters.blob.pBlobData = reinterpret_cast<BYTE*>(&activation);
  auto handler = Microsoft::WRL::Make<ActivationHandler>();
  if (!handler) throw std::runtime_error("could not create activation handler");
  ComPtr<IActivateAudioInterfaceAsyncOperation> operation;
  check(ActivateAudioInterfaceAsync(VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
                                    __uuidof(IAudioClient), &parameters,
                                    handler.Get(), operation.GetAddressOf()),
        "ActivateAudioInterfaceAsync");
  if (WaitForSingleObject(handler->event(), wait_ms) != WAIT_OBJECT_0)
    throw std::runtime_error("process loopback activation timed out");
  check(handler->result(), "process loopback activation");
  return handler->client();
}

struct CaptureStats {
  std::uint64_t packets = 0;
  std::uint64_t frames = 0;
  std::uint64_t silent_frames = 0;
  std::uint64_t discontinuities = 0;
  std::uint64_t timestamp_errors = 0;
  std::uint64_t first_qpc_100ns = 0;
  std::uint64_t last_qpc_100ns = 0;
};

class CaptureStream {
 public:
  CaptureStream(fs::path directory, std::string name)
      : directory_(std::move(directory)), name_(std::move(name)) {}
  ~CaptureStream() {
    running_ = false;
    if (worker_.joinable()) worker_.join();
    if (client_) client_->Stop();
  }

  void open_endpoint(EDataFlow flow, ERole role, bool loopback,
                     const std::wstring& endpoint_id) {
    ComPtr<IMMDeviceEnumerator> enumerator;
    check(CoCreateInstance(__uuidof(MMDeviceEnumerator), nullptr, CLSCTX_ALL,
                           IID_PPV_ARGS(enumerator.GetAddressOf())),
          "create endpoint enumerator");
    ComPtr<IMMDevice> endpoint;
    if (endpoint_id.empty()) {
      check(enumerator->GetDefaultAudioEndpoint(flow, role, endpoint.GetAddressOf()),
            "get default audio endpoint");
    } else {
      check(enumerator->GetDevice(endpoint_id.c_str(), endpoint.GetAddressOf()),
            "get selected audio endpoint");
    }
    LPWSTR raw_id = nullptr;
    check(endpoint->GetId(&raw_id), "get endpoint ID");
    endpoint_id_ = utf8(raw_id);
    CoTaskMemFree(raw_id);
    check(endpoint->Activate(__uuidof(IAudioClient), CLSCTX_ALL, nullptr,
                             reinterpret_cast<void**>(client_.GetAddressOf())),
          "activate endpoint");
    WAVEFORMATEX* raw_format = nullptr;
    check(client_->GetMixFormat(&raw_format), "get endpoint mix format");
    format_.reset(raw_format);
    initialize(loopback ? AUDCLNT_STREAMFLAGS_LOOPBACK : 0);
  }

  void open_process(DWORD process_id) {
    client_ = activate_process_loopback(process_id);
    auto* raw = static_cast<WAVEFORMATEX*>(CoTaskMemAlloc(sizeof(WAVEFORMATEX)));
    if (!raw) throw std::bad_alloc();
    *raw = {};
    raw->wFormatTag = WAVE_FORMAT_PCM;
    raw->nChannels = 2;
    raw->nSamplesPerSec = 48000;
    raw->wBitsPerSample = 16;
    raw->nBlockAlign = 4;
    raw->nAvgBytesPerSec = 192000;
    format_.reset(raw);
    initialize(AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM |
               AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY);
  }

  void start() {
    check(client_->Start(), "start audio capture");
    running_ = true;
    worker_ = std::thread([this] { capture_loop(); });
  }

  void stop() {
    running_ = false;
    if (worker_.joinable()) worker_.join();
    check(client_->Stop(), "stop audio capture");
    writer_->finish();
    if (!error_.empty()) throw std::runtime_error(error_);
    if (stats_.frames == 0) throw std::runtime_error(name_ + " received no audio frames");
  }

  bool failed() const { return failed_.load(); }
  const CaptureStats& stats() const { return stats_; }
  const std::vector<std::string>& files() const { return writer_->files(); }
  const std::string& endpoint_id() const { return endpoint_id_; }
  const WAVEFORMATEX* format() const { return format_.get(); }

 private:
  fs::path directory_;
  std::string name_;
  std::string endpoint_id_;
  ComPtr<IAudioClient> client_;
  ComPtr<IAudioCaptureClient> capture_;
  WaveFormat format_;
  std::unique_ptr<mon::SegmentedWaveWriter> writer_;
  std::atomic<bool> running_ = false;
  std::atomic<bool> failed_ = false;
  std::thread worker_;
  CaptureStats stats_;
  std::string error_;

  void initialize(DWORD flags) {
    check(client_->Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 0, 0, format_.get(), nullptr),
          "initialize audio client");
    check(client_->GetService(IID_PPV_ARGS(capture_.GetAddressOf())),
          "get audio capture client");
    const std::uint32_t format_size = format_->wFormatTag == WAVE_FORMAT_PCM
                                          ? 16
                                          : 18 + format_->cbSize;
    writer_ = std::make_unique<mon::SegmentedWaveWriter>(
        directory_, name_, reinterpret_cast<const std::uint8_t*>(format_.get()),
        format_size);
  }

  void capture_loop() {
    HRESULT apartment = CoInitializeEx(nullptr, COINIT_MULTITHREADED);
    try {
      check(apartment, "initialize capture thread COM");
      while (true) {
        UINT32 available = 0;
        UINT32 drain_packets = 0;
        check(capture_->GetNextPacketSize(&available), "get next packet size");
        while (available && (running_ || drain_packets < 64)) {
          BYTE* data = nullptr;
          UINT32 frames = 0;
          DWORD flags = 0;
          UINT64 device_position = 0;
          UINT64 qpc_position = 0;
          check(capture_->GetBuffer(&data, &frames, &flags, &device_position, &qpc_position),
                "get audio packet");
          try {
            std::uint64_t bytes = static_cast<std::uint64_t>(frames) * format_->nBlockAlign;
            if (bytes > std::numeric_limits<std::uint32_t>::max())
              throw std::runtime_error("audio packet byte count exceeds WAV limit");
            writer_->append(data, static_cast<std::uint32_t>(bytes),
                            (flags & AUDCLNT_BUFFERFLAGS_SILENT) != 0);
          } catch (...) {
            capture_->ReleaseBuffer(frames);
            throw;
          }
          check(capture_->ReleaseBuffer(frames), "release audio packet");
          if (!stats_.packets) stats_.first_qpc_100ns = qpc_position;
          stats_.last_qpc_100ns = qpc_position;
          ++stats_.packets;
          stats_.frames += frames;
          if (flags & AUDCLNT_BUFFERFLAGS_SILENT) stats_.silent_frames += frames;
          if (flags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY) ++stats_.discontinuities;
          if (flags & AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR) ++stats_.timestamp_errors;
          if (!running_) ++drain_packets;
          check(capture_->GetNextPacketSize(&available), "get next packet size");
        }
        if (!running_) break;
        Sleep(5);
      }
    } catch (const std::exception& error) {
      error_ = error.what();
      running_ = false;
      failed_ = true;
    }
    if (SUCCEEDED(apartment)) CoUninitialize();
  }
};

struct Options {
  fs::path output;
  std::wstring mode = L"system";
  std::wstring scenario = L"manual";
  std::wstring render_device;
  std::wstring microphone_device;
  DWORD process_id = 0;
  int duration = 30;
};

std::uint64_t parse_decimal(const std::wstring& text, const char* name) {
  if (text.empty() || text.find_first_not_of(L"0123456789") != std::wstring::npos)
    throw std::runtime_error(std::string(name) + " must be an unsigned decimal integer");
  try {
    return std::stoull(text);
  } catch (const std::out_of_range&) {
    throw std::runtime_error(std::string(name) + " is out of range");
  }
}

Options parse(int argc, wchar_t** argv) {
  Options options;
  for (int index = 1; index < argc; ++index) {
    std::wstring option = argv[index];
    if (option == L"--help") {
      throw std::runtime_error("usage");
    }
    if (index + 1 >= argc) throw std::runtime_error("missing value for an option");
    std::wstring value = argv[++index];
    if (option == L"--output") options.output = value;
    else if (option == L"--mode") options.mode = value;
    else if (option == L"--scenario") options.scenario = value;
    else if (option == L"--render-device-id") options.render_device = value;
    else if (option == L"--microphone-device-id") options.microphone_device = value;
    else if (option == L"--pid") {
      std::uint64_t process_id = parse_decimal(value, "PID");
      if (process_id == 0 || process_id > MAXDWORD)
        throw std::runtime_error("PID is out of range");
      options.process_id = static_cast<DWORD>(process_id);
    } else if (option == L"--duration") {
      std::uint64_t duration = parse_decimal(value, "duration");
      if (duration < 1 || duration > 14400)
        throw std::runtime_error("duration must be between 1 and 14400 seconds");
      options.duration = static_cast<int>(duration);
    } else {
      throw std::runtime_error("unknown option: " + utf8(option));
    }
  }
  if (options.output.empty() || options.duration < 1 || options.duration > 14400 ||
      (options.mode != L"system" && options.mode != L"application") ||
      (options.mode == L"application" && options.process_id == 0) ||
      (options.mode == L"system" && options.process_id != 0) ||
      (options.mode == L"application" && !options.render_device.empty())) {
    throw std::runtime_error("invalid output, duration, mode, or PID");
  }
  return options;
}

void write_stats(std::ostream& output, const CaptureStream& stream) {
  const auto& stats = stream.stats();
  output << "{\"endpoint_id\":\"" << json_escape(stream.endpoint_id()) << "\","
         << "\"sample_rate\":" << stream.format()->nSamplesPerSec << ','
         << "\"channels\":" << stream.format()->nChannels << ','
         << "\"packets\":" << stats.packets << ','
         << "\"frames\":" << stats.frames << ','
         << "\"silent_frames\":" << stats.silent_frames << ','
         << "\"discontinuities\":" << stats.discontinuities << ','
         << "\"timestamp_errors\":" << stats.timestamp_errors << ','
         << "\"first_qpc_100ns\":" << stats.first_qpc_100ns << ','
         << "\"last_qpc_100ns\":" << stats.last_qpc_100ns << ','
         << "\"files\":[";
  for (std::size_t index = 0; index < stream.files().size(); ++index) {
    if (index) output << ',';
    output << '"' << json_escape(stream.files()[index]) << '"';
  }
  output << "]}";
}

}  // namespace

#include "product_capture.h"
#include "audio_import.h"

int diagnostic_main(int argc, wchar_t** argv) {
  Options options;
  bool parsed = false;
  bool output_ready = false;
  try {
    options = parse(argc, argv);
    parsed = true;
    check(CoInitializeEx(nullptr, COINIT_MULTITHREADED), "initialize COM");
    if (fs::exists(options.output) &&
        (!fs::is_directory(options.output) || !fs::is_empty(options.output))) {
      throw std::runtime_error("output directory already contains files");
    }
    fs::create_directories(options.output);
    output_ready = true;
    {
      std::ofstream starting(options.output / "manifest.json", std::ios::binary);
      starting << "{\"schema_version\":1,\"state\":\"starting\","
               << "\"scenario\":\"" << json_escape(utf8(options.scenario)) << "\","
               << "\"mode\":\"" << json_escape(utf8(options.mode)) << "\","
               << "\"duration_requested_seconds\":" << options.duration << "}\n";
      if (!starting) throw std::runtime_error("initial manifest write failed");
    }
    CaptureStream microphone(options.output, "microphone");
    CaptureStream system(options.output, "system");
    microphone.open_endpoint(eCapture, eCommunications, false,
                             options.microphone_device);
    if (options.mode == L"system") {
      system.open_endpoint(eRender, eConsole, true, options.render_device);
    } else {
      system.open_process(options.process_id);
    }
    microphone.start();
    system.start();
    for (int elapsed = 0; elapsed < options.duration * 10; ++elapsed) {
      if (microphone.failed() || system.failed()) break;
      std::this_thread::sleep_for(std::chrono::milliseconds(100));
    }
    std::vector<std::string> stop_errors;
    try { system.stop(); } catch (const std::exception& error) {
      stop_errors.emplace_back(std::string("system: ") + error.what());
    }
    try { microphone.stop(); } catch (const std::exception& error) {
      stop_errors.emplace_back(std::string("microphone: ") + error.what());
    }
    if (!stop_errors.empty()) {
      std::ostringstream combined;
      bool first = true;
      for (const auto& error : stop_errors) {
        if (!first) combined << "; ";
        combined << error;
        first = false;
      }
      throw std::runtime_error(combined.str());
    }
    std::ofstream manifest(options.output / "manifest.json", std::ios::binary);
    manifest << "{\"schema_version\":1,\"state\":\"saved\","
             << "\"mode\":\"" << json_escape(utf8(options.mode)) << "\","
             << "\"scenario\":\"" << json_escape(utf8(options.scenario)) << "\","
             << "\"duration_requested_seconds\":" << options.duration << ','
             << "\"process_id\":" << options.process_id << ','
             << "\"microphone\":";
    write_stats(manifest, microphone);
    manifest << ",\"system\":";
    write_stats(manifest, system);
    manifest << "}\n";
    if (!manifest) throw std::runtime_error("manifest write failed");
    CoUninitialize();
    std::wcout << (options.output / "manifest.json").wstring() << L'\n';
    return 0;
  } catch (const std::exception& error) {
    if (parsed && output_ready) {
      try {
        std::ofstream failed(options.output / "manifest.json", std::ios::binary);
        failed << "{\"schema_version\":1,\"state\":\"failed\","
               << "\"scenario\":\"" << json_escape(utf8(options.scenario)) << "\","
               << "\"mode\":\"" << json_escape(utf8(options.mode)) << "\","
               << "\"duration_requested_seconds\":" << options.duration << ','
               << "\"process_id\":" << options.process_id << ','
               << "\"error\":\""
               << json_escape(error.what()) << "\"}\n";
      } catch (...) {}
    }
    if (std::string(error.what()) != "usage") std::cerr << "error: " << error.what() << '\n';
    std::cerr << "Usage: mon-windows-capture --output DIR --duration SECONDS "
                 "--mode system|application [--pid PID] [--scenario ID] "
                 "[--render-device-id ID] [--microphone-device-id ID]\n";
    return 2;
  }
}

int wmain(int argc, wchar_t** argv) {
  try {
    if(argc==3&&std::wstring(argv[1])==L"--raw-four-hour-fixture") {
      fs::path directory=argv[2];if(!directory.is_absolute()||!fs::is_directory(directory)||!fs::is_empty(directory))throw std::runtime_error("fixture requires a new empty absolute directory");
      product::RawSegments raw(directory,"6ed98cb6-1658-4e35-ab1c-6c98d82abc58","microphone",1);
      std::vector<float> packet(48000,0.125f);std::size_t maximum=0;
      for(std::uint64_t second=0;second<14400;++second){raw.append(packet.data(),48000,90000000000ULL+second*10000000ULL,second%901==0);maximum=std::max(maximum,raw.pcm.capacity());}
      raw.finish();if(raw.total!=691200000||raw.sequence!=3600||maximum>192000*4)throw std::runtime_error("raw fixture exceeded duration or memory contract");
      std::cout<<product::Json{{"frames",raw.total},{"segments",raw.sequence},{"maximumPendingPcmBytes",maximum},{"durationSeconds",14400}}.dump()<<std::endl;return 0;
    }
    if(argc>1&&std::wstring(argv[1])==L"--import-audio")return audio_import::run(argc,argv);
    for (int i=1;i<argc;++i) {
      std::wstring flag=argv[i];
      if(flag==L"--backend"||flag==L"--list-input-devices"||flag==L"--list-capture-applications")return product::run(argc,argv);
    }
    return diagnostic_main(argc,argv);
  } catch(const std::exception& error) { std::cerr<<error.what()<<std::endl;return 1; }
}
