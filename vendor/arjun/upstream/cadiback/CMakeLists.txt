cmake_minimum_required(VERSION 3.18)
project(cadiback LANGUAGES CXX)

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

# Generate config.hpp (VERSION, GITID, BUILD) into the build directory.
file(READ "${CMAKE_CURRENT_SOURCE_DIR}/VERSION" CADIBACK_VERSION)
string(STRIP "${CADIBACK_VERSION}" CADIBACK_VERSION)

find_package(Git QUIET)
if(Git_FOUND)
    execute_process(
        COMMAND "${GIT_EXECUTABLE}" rev-parse HEAD
        WORKING_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}"
        OUTPUT_VARIABLE CADIBACK_GITID
        OUTPUT_STRIP_TRAILING_WHITESPACE
        ERROR_QUIET
        RESULT_VARIABLE _git_result)
    if(NOT _git_result EQUAL 0)
        set(CADIBACK_GITID "unknown")
    endif()
else()
    set(CADIBACK_GITID "unknown")
endif()
set(CADIBACK_BUILD "${CMAKE_CXX_COMPILER}")

configure_file(
    "${CMAKE_CURRENT_SOURCE_DIR}/config.hpp.in"
    "${CMAKE_CURRENT_BINARY_DIR}/config.hpp")

# ---- CaDiCaL (cmake-based) ----
set(cadical_DIR "" CACHE PATH "cadical install/build prefix (contains lib/cmake/cadical/). Auto-resolved if empty.")

include(FetchContent)
if(NOT TARGET cadical)
    if(cadical_DIR)
        find_package(cadical CONFIG REQUIRED HINTS "${cadical_DIR}")
        message(STATUS "CaDiCaL: using pre-built at ${cadical_DIR}")
    else()
        FetchContent_Declare(cadical
            GIT_REPOSITORY https://github.com/meelgroup/cadical
            GIT_TAG        master
            GIT_SHALLOW    TRUE)
        FetchContent_MakeAvailable(cadical)
        message(STATUS "CaDiCaL: fetched from GitHub")
    endif()
endif()

add_library(cadiback STATIC cadiback.cpp)

set_target_properties(cadiback PROPERTIES POSITION_INDEPENDENT_CODE ON)

# Public API header is in include/; config.hpp is in the build directory.
target_include_directories(cadiback
    PUBLIC  $<BUILD_INTERFACE:${CMAKE_CURRENT_SOURCE_DIR}/include>
            $<INSTALL_INTERFACE:include/cadiback>
    PRIVATE "${CMAKE_CURRENT_BINARY_DIR}")

target_compile_features(cadiback PUBLIC cxx_std_17)

target_compile_definitions(cadiback PRIVATE
    $<$<NOT:$<CONFIG:Debug>>:NDEBUG>)

# Linking cadical PUBLIC gives cadiback (and its users) access to cadical's
# public headers (cadical.hpp, resources.hpp, version.hpp from src/).
target_link_libraries(cadiback PUBLIC cadical)

# ---- Install / cmake package export ----
include(GNUInstallDirs)
install(TARGETS cadiback EXPORT cadibackTargets
    ARCHIVE DESTINATION ${CMAKE_INSTALL_LIBDIR})

# Export for build-tree usage (lets dependents point CMAKE_PREFIX_PATH at build/)
# Use export(TARGETS ...) rather than export(EXPORT ...) so the file is written
# immediately at configure time (export(EXPORT) defers to the generate phase).
export(TARGETS cadiback
    FILE "${CMAKE_CURRENT_BINARY_DIR}/cadibackTargets.cmake")
install(DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}/include/"
    DESTINATION "${CMAKE_INSTALL_INCLUDEDIR}/cadiback"
    FILES_MATCHING PATTERN "*.h" PATTERN "*.hpp")
install(EXPORT cadibackTargets
    FILE cadibackTargets.cmake
    DESTINATION "${CMAKE_INSTALL_LIBDIR}/cmake/cadiback")
# Minimal Config file — just imports the targets
file(WRITE "${CMAKE_CURRENT_BINARY_DIR}/cadibackConfig.cmake"
    "include(\"\${CMAKE_CURRENT_LIST_DIR}/cadibackTargets.cmake\")\n")
install(FILES "${CMAKE_CURRENT_BINARY_DIR}/cadibackConfig.cmake"
    DESTINATION "${CMAKE_INSTALL_LIBDIR}/cmake/cadiback")
