cmake_minimum_required(VERSION 3.22.1)
project(pd_core C)

# libmgba (the repo's submodule, core only) + pd_core.c, as static libraries for the Rust crate.
# mGBA's options match app/src/main/cpp/CMakeLists.txt - keep them in step.

get_filename_component(REPO_DIR "${CMAKE_CURRENT_SOURCE_DIR}/../../.." ABSOLUTE)
set(MGBA_DIR "${REPO_DIR}/third_party/mgba")
if(NOT EXISTS "${MGBA_DIR}/CMakeLists.txt")
    message(FATAL_ERROR "mGBA submodule missing at ${MGBA_DIR} - run: git submodule update --init third_party/mgba")
endif()

# mGBA 0.10.5 asks for CMake 3.1 policies, which CMake 4 no longer offers.
set(CMAKE_POLICY_VERSION_MINIMUM 3.5)

set(BUILD_STATIC        ON  CACHE BOOL "" FORCE)
set(BUILD_SHARED        OFF CACHE BOOL "" FORCE)
set(DISABLE_FRONTENDS   ON  CACHE BOOL "" FORCE)
set(DISABLE_DEPS        ON  CACHE BOOL "" FORCE)
set(SKIP_LIBRARY        OFF CACHE BOOL "" FORCE)
set(BUILD_GL            OFF CACHE BOOL "" FORCE)
set(BUILD_GLES2         OFF CACHE BOOL "" FORCE)
set(BUILD_GLES3         OFF CACHE BOOL "" FORCE)
set(BUILD_PERF          OFF CACHE BOOL "" FORCE)
set(BUILD_TEST          OFF CACHE BOOL "" FORCE)
set(BUILD_SUITE         OFF CACHE BOOL "" FORCE)
set(BUILD_LIBRETRO      OFF CACHE BOOL "" FORCE)
set(BUILD_PYTHON        OFF CACHE BOOL "" FORCE)
set(USE_DEBUGGERS       OFF CACHE BOOL "" FORCE)
set(USE_GDB_STUB        OFF CACHE BOOL "" FORCE)
set(USE_EDITLINE        OFF CACHE BOOL "" FORCE)
set(USE_FFMPEG          OFF CACHE BOOL "" FORCE)
set(USE_DISCORD_RPC     OFF CACHE BOOL "" FORCE)
set(USE_LZMA            OFF CACHE BOOL "" FORCE)
set(USE_LUA             OFF CACHE BOOL "" FORCE)
set(USE_ELF             OFF CACHE BOOL "" FORCE)
set(USE_SQLITE3         OFF CACHE BOOL "" FORCE)
set(USE_LIBZIP          OFF CACHE BOOL "" FORCE)
set(USE_MINIZIP         OFF CACHE BOOL "" FORCE)
set(USE_PNG             OFF CACHE BOOL "" FORCE)
set(USE_ZLIB            OFF CACHE BOOL "" FORCE)
set(M_CORE_GBA          ON  CACHE BOOL "" FORCE)
set(M_CORE_GB           ON  CACHE BOOL "" FORCE)
set(BUILD_LTO           OFF CACHE BOOL "" FORCE)
set(SKIP_INSTALL_ALL    ON  CACHE BOOL "" FORCE)

add_subdirectory("${MGBA_DIR}" "${CMAKE_CURRENT_BINARY_DIR}/mgba" EXCLUDE_FROM_ALL)

# EXPAND: the Android app's own pk_wide.c (the overworld drawn past the GBA's edges), compiled in as is.
set(APP_CPP_DIR "${REPO_DIR}/app/src/main/cpp")
add_library(pd_core STATIC csrc/pd_core.c "${APP_CPP_DIR}/pk_wide.c")
# The same defines as mGBA's own sources: some of its structs change shape with them.
target_compile_definitions(pd_core PRIVATE M_CORE_GBA M_CORE_GB)
# pk_wide.c's fault guard is POSIX signals (sigsetjmp / sigaction): not on Windows, where it's built without it.
if(WIN32)
    target_compile_definitions(pd_core PRIVATE PK_WIDE_NO_GUARD)
endif()
if(NOT WIN32)
    target_compile_definitions(pd_core PRIVATE USE_PTHREADS)
endif()
target_include_directories(pd_core PRIVATE
    "${APP_CPP_DIR}"
    "${MGBA_DIR}/include"
    "${MGBA_DIR}/src"
    "${CMAKE_CURRENT_BINARY_DIR}/mgba/include")
target_link_libraries(pd_core mgba)
