cmake_minimum_required(VERSION 3.21)
project(mon_windows_capture LANGUAGES CXX)

add_executable(mon-windows-capture main.cpp segmented_wave_writer.h)
target_compile_features(mon-windows-capture PRIVATE cxx_std_20)
target_compile_definitions(mon-windows-capture PRIVATE UNICODE _UNICODE WIN32_LEAN_AND_MEAN)
target_link_libraries(mon-windows-capture PRIVATE ole32 uuid mmdevapi)

if(MSVC)
  # The QA installer must run on PCs without the Visual Studio development CRT.
  set_property(TARGET mon-windows-capture PROPERTY MSVC_RUNTIME_LIBRARY "MultiThreaded$<$<CONFIG:Debug>:Debug>")
  target_compile_options(mon-windows-capture PRIVATE /utf-8 /W4 /permissive-)
endif()
