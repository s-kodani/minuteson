cmake_minimum_required(VERSION 3.21)
project(mon_windows_capture LANGUAGES CXX)

add_executable(mon-windows-capture main.cpp segmented_wave_writer.h)
target_compile_features(mon-windows-capture PRIVATE cxx_std_20)
target_compile_definitions(mon-windows-capture PRIVATE UNICODE _UNICODE WIN32_LEAN_AND_MEAN)
target_link_libraries(mon-windows-capture PRIVATE ole32 uuid mmdevapi)
