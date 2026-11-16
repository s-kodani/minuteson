#pragma once

// Metadata only: never initialize or start an audio stream, or change volume,
// mute, routing or the default device. Used to correlate a live route failure.
namespace audio_inspection {
// PKEY_AudioEngine_DeviceFormat from the Windows SDK mmdeviceapi.h. Use a
// local key because that SDK declares the external key without a uuid.lib entry.
inline constexpr PROPERTYKEY engine_format{{0xf19f064d,0x082c,0x4e27,{0xbc,0x73,0x68,0x82,0xa1,0xbb,0x8e,0x4c}},0};
inline Json property(IMMDevice* device,const PROPERTYKEY& key){
  ComPtr<IPropertyStore> store;if(FAILED(device->OpenPropertyStore(STGM_READ,&store)))return nullptr;
  PROPVARIANT value;PropVariantInit(&value);Json result=nullptr;
  if(SUCCEEDED(store->GetValue(key,&value))){
    if(value.vt==VT_LPWSTR&&value.pwszVal)result=utf8(value.pwszVal);
    else if(value.vt==VT_CLSID&&value.puuid){wchar_t id[40]{};if(StringFromGUID2(*value.puuid,id,40))result=utf8(id);}
    else if(value.vt==VT_BLOB&&value.blob.pBlobData&&value.blob.cbSize>=sizeof(WAVEFORMATEX)){
      WAVEFORMATEX format{};std::memcpy(&format,value.blob.pBlobData,sizeof(format));
      result=Json{{"tag",format.wFormatTag},{"channels",format.nChannels},{"sampleRate",format.nSamplesPerSec},{"bits",format.wBitsPerSample},{"blockAlign",format.nBlockAlign}};
    }
  }
  PropVariantClear(&value);return result;
}
inline Json defaults(IMMDeviceEnumerator* devices,EDataFlow flow){
  Json result=Json::object();const char* names[]={"console","multimedia","communications"};
  for(unsigned i=0;i<3;++i){ComPtr<IMMDevice> device;result[names[i]]=SUCCEEDED(devices->GetDefaultAudioEndpoint(flow,static_cast<ERole>(i),&device))?Json(utf8(endpoint_id(device.Get()))):Json(nullptr);}
  return result;
}
inline Json sessions(IMMDevice* device){
  ComPtr<IAudioSessionManager2> manager;ComPtr<IAudioSessionEnumerator> list;
  auto hr=device->Activate(__uuidof(IAudioSessionManager2),CLSCTX_ALL,nullptr,reinterpret_cast<void**>(manager.GetAddressOf()));
  if(SUCCEEDED(hr))hr=manager->GetSessionEnumerator(&list);
  if(FAILED(hr))return Json{{"error",static_cast<unsigned long>(hr)}};
  int count=0;check(list->GetCount(&count),"inspect audio session count");Json result=Json::array();
  for(int i=0;i<std::min(count,128);++i){
    ComPtr<IAudioSessionControl> session;if(FAILED(list->GetSession(i,&session)))continue;
    Json row;AudioSessionState state;if(SUCCEEDED(session->GetState(&state)))row["state"]=int(state);
    ComPtr<IAudioSessionControl2> identity;
    if(SUCCEEDED(session.As(&identity))){
      DWORD pid=0;const auto identified=identity->GetProcessId(&pid);row["processResult"]=static_cast<unsigned long>(identified);row["pid"]=pid;
      LPWSTR id=nullptr;if(SUCCEEDED(identity->GetSessionInstanceIdentifier(&id))){row["instance"]=utf8(id);CoTaskMemFree(id);}
    }
    ComPtr<ISimpleAudioVolume> volume;
    if(SUCCEEDED(session.As(&volume))){float level=0;BOOL mute=FALSE;if(SUCCEEDED(volume->GetMasterVolume(&level)))row["volume"]=level;if(SUCCEEDED(volume->GetMute(&mute)))row["muted"]=mute!=FALSE;}
    result.push_back(std::move(row));
  }
  return result;
}
inline Json snapshot(){
  auto e=enumerator();FILETIME wall{};GetSystemTimePreciseAsFileTime(&wall);
  Json result{{"schemaVersion",1},{"observedAtMs",(ticks(wall)-EPOCH)/10000},{"inspectorPID",GetCurrentProcessId()},{"defaultInputs",defaults(e.Get(),eCapture)},{"defaultOutputs",defaults(e.Get(),eRender)},{"devices",Json::array()}};
  ComPtr<IMMDeviceCollection> devices;check(e->EnumAudioEndpoints(eAll,DEVICE_STATE_ACTIVE,&devices),"inspect active audio endpoints");
  UINT count=0;check(devices->GetCount(&count),"inspect audio endpoint count");
  for(UINT i=0;i<std::min(count,128u);++i){
    ComPtr<IMMDevice> device;check(devices->Item(i,&device),"inspect audio endpoint");
    ComPtr<IMMEndpoint> endpoint;EDataFlow flow=eAll;if(SUCCEEDED(device.As(&endpoint)))endpoint->GetDataFlow(&flow);
    Json row{{"uid",utf8(endpoint_id(device.Get()))},{"flow",flow==eCapture?"input":"output"},{"name",property(device.Get(),PKEY_Device_FriendlyName)},{"container",property(device.Get(),PKEY_Device_ContainerId)},{"format",property(device.Get(),engine_format)}};
    ComPtr<IAudioEndpointVolume> volume;
    if(SUCCEEDED(device->Activate(__uuidof(IAudioEndpointVolume),CLSCTX_ALL,nullptr,reinterpret_cast<void**>(volume.GetAddressOf())))){
      float level=0;BOOL mute=FALSE;if(SUCCEEDED(volume->GetMasterVolumeLevelScalar(&level)))row["volume"]=level;if(SUCCEEDED(volume->GetMute(&mute)))row["muted"]=mute!=FALSE;
    }
    row["sessions"]=sessions(device.Get());result["devices"].push_back(std::move(row));
  }
  return result;
}
} // namespace audio_inspection
