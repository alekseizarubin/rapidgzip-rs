cmake_minimum_required(VERSION 3.17)
project(rapidgzip-capi CXX)

set(CMAKE_CXX_STANDARD 17)

if(MSVC)
endif()

set(LIBRAPIDARCHIVE_DIR "${CMAKE_CURRENT_SOURCE_DIR}/../../vendor/librapidarchive")

set(LIBRAPIDARCHIVE_BUILD_BENCHMARKS OFF CACHE BOOL "" FORCE)
set(LIBRAPIDARCHIVE_BUILD_TESTS OFF CACHE BOOL "" FORCE)
set(LIBRAPIDARCHIVE_BUILD_TOOLS OFF CACHE BOOL "" FORCE)
set(LIBRAPIDARCHIVE_USE_SYSTEM_ZLIB OFF CACHE BOOL "" FORCE)
set(LIBRAPIDARCHIVE_USE_ZLIB_NG ON CACHE BOOL "" FORCE)
set(LIBRAPIDARCHIVE_WITH_RPMALLOC ON CACHE BOOL "" FORCE)
# Upstream currently disables ISA-L for macOS in release workflows because the
# macOS path is not stable enough yet. Keep the Apple build reliable and use the
# accelerated ISA-L path on the other targets.
if(APPLE)
    set(LIBRAPIDARCHIVE_WITH_ISAL OFF CACHE BOOL "" FORCE)
else()
    set(LIBRAPIDARCHIVE_WITH_ISAL ON CACHE BOOL "" FORCE)
endif()

add_subdirectory("${LIBRAPIDARCHIVE_DIR}" "${CMAKE_CURRENT_BINARY_DIR}/librapidarchive" EXCLUDE_FROM_ALL)

set(SOURCES src/rgz_capi.cpp)

add_library(rapidgzip-capi STATIC ${SOURCES})
set_target_properties(rapidgzip-capi PROPERTIES POSITION_INDEPENDENT_CODE ON)
target_link_libraries(rapidgzip-capi PRIVATE rapidgzip::rapidgzip)

if(TARGET rpmalloc)
    target_link_libraries(rapidgzip-capi PRIVATE rpmalloc)
    target_compile_definitions(rapidgzip-capi PRIVATE LIBRAPIDARCHIVE_WITH_RPMALLOC)
endif()

install(TARGETS rapidgzip-capi DESTINATION lib)
