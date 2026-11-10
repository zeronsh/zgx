# The Filament arm, against a Filament release's prebuilt libraries
# (fetch.py unpacks one; -DFILAMENT_DIR=<its filament/ folder>).
cmake_minimum_required(VERSION 3.22)
project(filament-compare-3d CXX OBJCXX)
set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
if(NOT FILAMENT_DIR)
  message(FATAL_ERROR "-DFILAMENT_DIR=<the release's filament/ folder> is needed")
endif()
add_executable(filament-compare-3d main.cpp)
target_include_directories(filament-compare-3d PRIVATE ${FILAMENT_DIR}/include)
foreach(lib gltfio_core filament backend filabridge filaflat utils geometry smol-v ibl
            ktxreader image stb basis_transcoder uberarchive uberzlib zstd dracodec
            meshoptimizer mikktspace abseil bluevk bluegl perfetto shaders)
  target_link_libraries(filament-compare-3d PRIVATE ${FILAMENT_DIR}/lib/arm64/lib${lib}.a)
endforeach()
target_link_libraries(filament-compare-3d PRIVATE
  "-framework Metal" "-framework QuartzCore" "-framework Cocoa" "-framework CoreVideo"
  "-framework IOKit" "-framework IOSurface" "-framework OpenGL")
