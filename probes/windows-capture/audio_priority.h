#pragma once
#include <windows.h>
#include <avrt.h>
namespace product {
// Best-effort MMCSS boost for threads that drain or persist captured audio.
// Acquisition threads already do this; the session loop and the WAV writer must
// not be scheduled behind ordinary work, or the acquisition queue overflows.
// Failure is ignored: running at normal priority is the previous behaviour.
class ScopedAudioPriority {
  HANDLE handle_=nullptr;
 public:
  ScopedAudioPriority(){
    DWORD index=0;handle_=AvSetMmThreadCharacteristicsW(L"Audio",&index);
    if(handle_)AvSetMmThreadPriority(handle_,AVRT_PRIORITY_HIGH);
  }
  ScopedAudioPriority(const ScopedAudioPriority&)=delete;
  ScopedAudioPriority& operator=(const ScopedAudioPriority&)=delete;
  // AVRT handles require AvRevertMmThreadCharacteristics, not CloseHandle.
  ~ScopedAudioPriority(){if(handle_)AvRevertMmThreadCharacteristics(handle_);}
};
} // namespace product
