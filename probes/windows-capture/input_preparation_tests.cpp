#include "input_preparation.h"
#include "loopback_scope.h"
#include <chrono>
#include <iostream>
#include <stdexcept>

using namespace std::chrono_literals;
void require(bool ok,const char* message){if(!ok)throw std::runtime_error(message);}
template<class Predicate>void until(Predicate predicate){auto end=std::chrono::steady_clock::now()+5s;while(!predicate()){if(std::chrono::steady_clock::now()>=end)throw std::runtime_error("worker did not complete");std::this_thread::yield();}}
struct Gate {
  std::mutex mutex;std::condition_variable changed;bool open=false;
  void wait(){std::unique_lock lock(mutex);changed.wait(lock,[&]{return open;});}
  void release(){{std::lock_guard lock(mutex);open=true;}changed.notify_all();}
};
thread_local bool context_alive=false;
struct Context{Context(){context_alive=true;}~Context(){context_alive=false;}};
struct Counts{std::atomic<int> created{0},disposed{0},outside_context{0},active{0};std::thread::id disposed_on;};
struct Value {
  std::shared_ptr<Counts> counts;
  explicit Value(std::shared_ptr<Counts> c):counts(std::move(c)){++counts->created;++counts->active;}
  Value(Value&&)=default;
  Value& operator=(Value&&)=default;
  ~Value(){if(counts){if(!context_alive)++counts->outside_context;counts->disposed_on=std::this_thread::get_id();++counts->disposed;--counts->active;}}
};
int main(){
  try{
    using Preparation=product::InputPreparation<Value,Context>;
    auto global=product::loopback_target(0,123);
    auto application=product::loopback_target(456,123);
    require(global.pid==123&&global.exclude_tree,"system capture must exclude the helper tree across endpoints");
    require(application.pid==456&&!application.exclude_tree,"application scope changed or fell back to global");
    // Exercise the actual producer's schedule with a virtual control clock:
    // timeout is observable after five seconds, but is not operation completion.
    product::PreparationSchedule schedule;
    schedule.begin(100);
    require(!schedule.timed_out(5099)&&schedule.timed_out(5100),"five-second timeout boundary changed");
    bool overlapped=false;try{schedule.begin(5100);}catch(const std::logic_error&){overlapped=true;}
    require(overlapped,"producer permitted overlapping preparation");
    schedule.cancel();
    require(!schedule.timed_out(5101)&&!schedule.may_start(90000),"timeout/cancellation allowed retry before native completion");
    schedule.finish(90000);
    require(!schedule.may_start(90999)&&schedule.may_start(91000),"producer did not wait one second after native cleanup");
    schedule.begin(91000);schedule.accept();
    require(!schedule.outstanding&&schedule.initial_resolved,"successful input handoff did not resolve initial preparation");
    // A delayed native operation cannot block the control/writer thread. A
    // timeout invalidates the result but does not pretend that it has closed.
    auto gate=std::make_shared<Gate>();auto counts=std::make_shared<Counts>();
    auto preparation=std::make_unique<Preparation>([=](auto cancelled){gate->wait();require(cancelled(),"cancel signal lost");return Value(counts);});
    unsigned microphone_writes=0,status_updates=0;
    for(unsigned i=0;i<1000;++i){++microphone_writes;if(i%25==0)++status_updates;require(!preparation->finished(),"native operation completed before release");}
    preparation->cancel();
    require(microphone_writes==1000&&status_updates==40&&!preparation->take(),"pending preparation blocked another lane or was accepted");
    require(!preparation->finished(),"timeout incorrectly proved native closure");
    gate->release();until([&]{return preparation->finished();});
    require(counts->created==1&&counts->disposed==1&&counts->active==0,"late result leaked or remained started");
    require(counts->disposed_on!=std::this_thread::get_id()&&counts->outside_context==0,"cancelled native object destroyed on control thread or outside its apartment");
    preparation.reset();
    // The owning Lane may disappear before the native operation returns. Its
    // worker owns only the operation state and disposes of the late result.
    gate=std::make_shared<Gate>();counts=std::make_shared<Counts>();
    {
      Preparation pending([=](auto){gate->wait();return Value(counts);});
      pending.cancel();
    }
    gate->release();until([&]{return counts->disposed==1;});
    require(counts->outside_context==0,"destruction invalidated a worker's native lifetime");
    // Ready results also require acceptance. A stop/selection change between
    // readiness and adoption must reject and close that result on the worker.
    counts=std::make_shared<Counts>();
    Preparation ready([=](auto){return Value(counts);});
    until([&]{return counts->created==1;});ready.cancel();
    until([&]{return ready.finished();});
    require(!ready.take()&&counts->disposed==1,"ready result was adopted after stop");
    // Successful handoff transfers ownership once; subsequent polls cannot
    // attach the same stream twice. Control thread now owns the accepted value.
    auto success=std::make_shared<Counts>();
    Preparation accepted([=](auto){return Value(success);});std::optional<Value> taken;
    until([&]{taken=accepted.take();return taken.has_value();});
    require(!accepted.take()&&success->active==1,"native stream accepted twice");
    until([&]{return accepted.finished();});
    taken.reset();require(success->disposed==1,"accepted stream ownership lost");
    product::InputPreparation<int> failure([](auto)->int{throw std::runtime_error("device unavailable");});
    until([&]{return failure.finished();});require(bool(failure.error())&&!failure.take(),"native failure hidden");
    std::cout<<"input preparation: independent control, cancellation, late/ready rejection, safe lifetime, one handoff and scope passed\n";return 0;
  }catch(const std::exception& error){std::cerr<<error.what()<<'\n';return 1;}
}
