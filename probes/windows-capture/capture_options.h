#pragma once
// Included inside namespace product after native metadata adapters.
struct Settings {fs::path output;std::string owner,mode="system",uid,scenario,identity;DWORD pid=0,parent=0,synthetic_startup_delay_ms=0;bool test=false,synthetic=false,synthetic_clock_overlap=false,synthetic_clock_cases=false,synthetic_clock_resampling=false,synthetic_clock_drift=false,lease_stdin=false;};
std::string selection_request(const fs::path& path){
  Handle file(CreateFileW(path.c_str(),GENERIC_READ,FILE_SHARE_READ|FILE_SHARE_DELETE,nullptr,OPEN_EXISTING,FILE_FLAG_OPEN_REPARSE_POINT|FILE_FLAG_SEQUENTIAL_SCAN,nullptr));
  if(file.value==INVALID_HANDLE_VALUE)throw std::runtime_error("microphone selection request cannot be opened");
  BY_HANDLE_FILE_INFORMATION before{},after{};LARGE_INTEGER size{};
  if(GetFileType(file.value)!=FILE_TYPE_DISK||!GetFileInformationByHandle(file.value,&before)
    ||(before.dwFileAttributes&(FILE_ATTRIBUTE_DIRECTORY|FILE_ATTRIBUTE_REPARSE_POINT))||before.nNumberOfLinks!=1
    ||!GetFileSizeEx(file.value,&size)||size.QuadPart<=0||size.QuadPart>4096)
    throw std::runtime_error("microphone selection request is not a bounded regular file");
  std::string bytes(static_cast<std::size_t>(size.QuadPart),0);DWORD count=0;
  if(!ReadFile(file.value,bytes.data(),static_cast<DWORD>(bytes.size()),&count,nullptr)||count!=bytes.size()
    ||!GetFileInformationByHandle(file.value,&after)||after.nNumberOfLinks!=1
    ||(after.dwFileAttributes&(FILE_ATTRIBUTE_DIRECTORY|FILE_ATTRIBUTE_REPARSE_POINT))
    ||before.nFileSizeHigh!=after.nFileSizeHigh||before.nFileSizeLow!=after.nFileSizeLow
    ||ticks(before.ftLastWriteTime)!=ticks(after.ftLastWriteTime))throw std::runtime_error("microphone selection request changed while reading");
  return bytes;
}
DWORD capture_integer(const std::wstring& value){
  if(value.empty()||value.find_first_not_of(L"0123456789")!=std::wstring::npos)throw std::runtime_error("invalid capture integer");
  std::size_t used=0;auto number=std::stoull(value,&used);
  if(used!=value.size()||number>std::numeric_limits<DWORD>::max())throw std::runtime_error("capture integer exceeds capacity");
  return static_cast<DWORD>(number);
}
Settings parse(int argc,wchar_t** argv) {
  Settings s;std::set<std::wstring> seen;
  for(int i=1;i<argc;++i) {
    std::wstring flag=argv[i];
    if(!seen.insert(flag).second)throw std::runtime_error("duplicate product capture argument");
    if(flag==L"--stop-on-file"||flag==L"--until-stopped")continue;
    if(flag==L"--synthetic"){s.synthetic=true;continue;}
    if(flag==L"--synthetic-clock-overlap"){s.synthetic_clock_overlap=true;continue;}
    if(flag==L"--synthetic-clock-cases"){s.synthetic_clock_cases=true;continue;}
    if(flag==L"--synthetic-clock-resampling"){s.synthetic_clock_resampling=true;continue;}
    if(flag==L"--synthetic-clock-drift"){s.synthetic_clock_drift=true;continue;}
    if(i+1>=argc)throw std::runtime_error("missing product capture argument");
    auto value=std::wstring(argv[++i]);
    if(flag==L"--output")s.output=value;
    else if(flag==L"--mode")s.mode=utf8(value);
    else if(flag==L"--microphone-uid")s.uid=utf8(value);
    else if(flag==L"--input-level-owner")s.owner=utf8(value);
    else if(flag==L"--five-second-test"){s.test=true;s.owner=utf8(value);}
    else if(flag==L"--pid")s.pid=capture_integer(value);
    else if(flag==L"--parent-pid")s.parent=capture_integer(value);
    else if(flag==L"--application-identity")s.identity=utf8(value);
    else if(flag==L"--scenario")s.scenario=utf8(value);
    else if(flag==L"--synthetic-startup-delay-ms")s.synthetic_startup_delay_ms=capture_integer(value);
    else if(flag==L"--capture-test-lease-stdin"){if(value!=L"1")throw std::runtime_error("invalid test lease");s.lease_stdin=true;}
    else if(flag==L"--backend"){if(value!=L"wasapi")throw std::runtime_error("unsupported capture backend");}
    else throw std::runtime_error("unknown product capture argument");
  }
  if(!s.output.is_absolute()||!mon_capture_valid_owner(reinterpret_cast<const std::uint8_t*>(s.owner.data()),s.owner.size())||!s.parent||(s.mode!="system"&&s.mode!="application")||(s.mode=="application")!=bool(s.pid)
    ||(!s.uid.empty()&&!mon_capture_valid_uid(reinterpret_cast<const std::uint8_t*>(s.uid.data()),s.uid.size()))
    ||(seen.count(L"--microphone-uid")&&s.uid.empty())
    ||(seen.count(L"--input-level-owner")&&seen.count(L"--five-second-test"))
    ||s.synthetic_startup_delay_ms>5000||(!s.synthetic&&(s.synthetic_startup_delay_ms||s.synthetic_clock_overlap||s.synthetic_clock_cases||s.synthetic_clock_resampling||s.synthetic_clock_drift)))
    throw std::runtime_error("invalid product capture ownership or scope");
  return s;
}
