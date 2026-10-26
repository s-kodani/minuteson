#pragma once
#include <atomic>
#include <condition_variable>
#include <cstdint>
#include <exception>
#include <memory>
#include <mutex>
#include <optional>
#include <stdexcept>
#include <thread>
#include <utility>

namespace product {
struct EmptyPreparationContext {};
struct PreparationSchedule {
  static constexpr std::uint64_t timeout_ms=5000,retry_ms=1000;
  std::uint64_t started_at=0,retry_at=0;
  bool outstanding=false,cancelled=false,initial_resolved=false;
  bool may_start(std::uint64_t now)const{return !outstanding&&now>=retry_at;}
  void begin(std::uint64_t now){if(!may_start(now))throw std::logic_error("overlapping input preparation");outstanding=true;cancelled=false;started_at=now;}
  bool timed_out(std::uint64_t now)const{return outstanding&&!cancelled&&now-started_at>=timeout_ms;}
  void cancel(){if(outstanding)cancelled=true;}
  void finish(std::uint64_t now){outstanding=false;initial_resolved=true;retry_at=now+retry_ms;}
  void accept(){outstanding=false;initial_resolved=true;}
};

// A native acquisition may outlive a timeout: cancelling the UI's interest
// cannot cancel ActivateAudioInterfaceAsync. Keep its one worker and native
// objects alive until the actual operation returns. Only an explicit take()
// transfers ownership; rejected results are destroyed on the worker, with its
// thread context still alive. No worker ever owns a Lane or an output file.
template<class Value,class ThreadContext=EmptyPreparationContext>
class InputPreparation {
  struct State {
    std::mutex mutex;
    std::condition_variable changed;
    std::optional<Value> value;
    std::exception_ptr error;
    std::atomic<bool> cancelled{false},finished{false};
    bool accepted=false;
  };
  std::shared_ptr<State> state_;
 public:
  class Cancellation {
    std::shared_ptr<State> state_;
   public:
    explicit Cancellation(std::shared_ptr<State> state):state_(std::move(state)){}
    bool operator()() const {return state_->cancelled.load(std::memory_order_acquire);}
  };
  template<class Factory>
  explicit InputPreparation(Factory factory):state_(std::make_shared<State>()) {
    std::thread([state=state_,factory=std::move(factory)]() mutable {
      try {
        [[maybe_unused]] ThreadContext context;
        auto value=factory(Cancellation(state));
        std::unique_lock lock(state->mutex);
        if(!state->cancelled.load(std::memory_order_acquire)) {
          state->value.emplace(std::move(value));
          state->changed.wait(lock,[&]{return state->accepted||state->cancelled.load(std::memory_order_acquire);});
          // Moving out under the lock lets cancellation dispose of a result
          // without blocking the control thread in a native Stop or Release.
          if(state->value){value=std::move(*state->value);state->value.reset();}
        }
        lock.unlock();
      }catch(...) {
        std::lock_guard lock(state->mutex);state->error=std::current_exception();
      }
      // This includes disposal of a cancelled native stream and its COM
      // apartment. A caller must not start another attempt before this point.
      state->finished.store(true,std::memory_order_release);
    }).detach();
  }
  InputPreparation(const InputPreparation&)=delete;
  InputPreparation& operator=(const InputPreparation&)=delete;
  ~InputPreparation(){cancel();}
  void cancel(){{std::lock_guard lock(state_->mutex);state_->cancelled.store(true,std::memory_order_release);}state_->changed.notify_all();}
  bool finished() const {return state_->finished.load(std::memory_order_acquire);}
  std::exception_ptr error() const {std::lock_guard lock(state_->mutex);return state_->error;}
  std::optional<Value> take() {
    std::lock_guard lock(state_->mutex);
    if(state_->cancelled.load(std::memory_order_acquire)||!state_->value)return std::nullopt;
    auto result=std::move(state_->value);state_->value.reset();state_->accepted=true;
    state_->changed.notify_all();return result;
  }
};
} // namespace product
