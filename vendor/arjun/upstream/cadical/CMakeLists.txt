cmake_minimum_required(VERSION 3.18)
project(cadical VERSION 2.1.3 LANGUAGES CXX)

# ---- Compiler flags (only added if the compiler accepts them) ----
include(CheckCXXCompilerFlag)
macro(add_cxx_flag_if_supported flagname)
  check_cxx_compiler_flag("${flagname}" HAVE_FLAG_${flagname})

  if(HAVE_FLAG_${flagname})
    set(CMAKE_CXX_FLAGS "${CMAKE_CXX_FLAGS} ${flagname}")
  endif()
endmacro()

if(NOT WIN32)
    if(NOT ENABLE_TESTING AND ${CMAKE_SYSTEM_NAME} MATCHES "Linux")
        add_cxx_flag_if_supported("-fvisibility=hidden")
    endif()

    add_cxx_flag_if_supported("-mpopcnt")
    add_cxx_flag_if_supported("-msse4.2")
    add_cxx_flag_if_supported("-mpclmul")
    if(CMAKE_BUILD_TYPE STREQUAL "Release")
        # add_cxx_flag_if_supported("-flto")
        # add_link_flag_if_supported("-flto")
    else()
        add_cxx_flag_if_supported("-Wall")
        add_cxx_flag_if_supported("-Wextra")
        add_cxx_flag_if_supported("-Wunused")
        add_cxx_flag_if_supported("-Wsign-compare")
        add_cxx_flag_if_supported("-fno-omit-frame-pointer")
        add_cxx_flag_if_supported("-ggdb3")
        add_cxx_flag_if_supported("-Wtype-limits")
        add_cxx_flag_if_supported("-Wuninitialized")
        add_cxx_flag_if_supported("-Wno-deprecated")
        add_cxx_flag_if_supported("-Wno-extra-semi")
        add_cxx_flag_if_supported("-Wstrict-aliasing")
        add_cxx_flag_if_supported("-Wpointer-arith")
        add_cxx_flag_if_supported("-Wheader-guard")
        add_cxx_flag_if_supported("-Wformat-nonliteral")
        add_cxx_flag_if_supported("-Winit-self")
        add_cxx_flag_if_supported("-Wparentheses")
        add_cxx_flag_if_supported("-Wunreachable-code")
    endif()
endif()

# Collect all library sources: everything in src/ except the two standalone
# application entry points (cadical.cpp = solver binary, mobical.cpp = tester).
file(GLOB cadical_sources CONFIGURE_DEPENDS
    "${CMAKE_CURRENT_SOURCE_DIR}/src/*.cpp"
    "${CMAKE_CURRENT_SOURCE_DIR}/contrib/*.cpp")
list(FILTER cadical_sources EXCLUDE REGEX "/(cadical|mobical)\\.cpp$")

add_library(cadical STATIC ${cadical_sources})

# Always build as position-independent code so the static lib can be linked
# into a shared library or Python extension module.
set_target_properties(cadical PROPERTIES POSITION_INDEPENDENT_CODE ON)

# Public headers are in src/: cadical.hpp plus internal headers that
# downstream projects (cadiback, CryptoMiniSat) need (resources.hpp, etc.).
target_include_directories(cadical PUBLIC
    $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/src>
    $<INSTALL_INTERFACE:include/cadical>)

target_compile_features(cadical PUBLIC cxx_std_17)

# Fixed feature set: no API contracts, no tracing, frame-pointer kept for
# profiling (-fno-omit-frame-pointer = --fp in the old configure script).
# NBUILD skips the build.hpp generation step (version.cpp handles this).
target_compile_definitions(cadical PRIVATE
    NCONTRACTS
    NTRACING
    NBUILD
    NCLOSEFROM
    NUNLOCKED
    $<$<NOT:$<CONFIG:Debug>>:NDEBUG>)

target_compile_options(cadical PRIVATE -fno-omit-frame-pointer)

# ---- Install / cmake package export ----
include(GNUInstallDirs)

install(TARGETS cadical EXPORT cadicalTargets
    ARCHIVE DESTINATION ${CMAKE_INSTALL_LIBDIR})

# Export for build-tree usage (lets dependents point CMAKE_PREFIX_PATH at build/)
# Use export(TARGETS ...) rather than export(EXPORT ...) so the file is written
# immediately at configure time (export(EXPORT) defers to the generate phase).
export(TARGETS cadical
    FILE "${CMAKE_CURRENT_BINARY_DIR}/cadicalTargets.cmake")
# Install all public headers flat into include/cadical/
install(DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}/src/"
    DESTINATION "${CMAKE_INSTALL_INCLUDEDIR}/cadical"
    FILES_MATCHING PATTERN "*.hpp")
install(EXPORT cadicalTargets
    FILE cadicalTargets.cmake
    DESTINATION "${CMAKE_INSTALL_LIBDIR}/cmake/cadical")
# Minimal Config file — just imports the targets
file(WRITE "${CMAKE_CURRENT_BINARY_DIR}/cadicalConfig.cmake"
    "include(\"\${CMAKE_CURRENT_LIST_DIR}/cadicalTargets.cmake\")\n")
install(FILES "${CMAKE_CURRENT_BINARY_DIR}/cadicalConfig.cmake"
    DESTINATION "${CMAKE_INSTALL_LIBDIR}/cmake/cadical")
