cmake_minimum_required(VERSION 3.17)
project(rapidgzip-capi CXX)

set(CMAKE_CXX_STANDARD 17)

if(MSVC)
    add_compile_options(/W3 /utf-8 /EHsc)
endif()

set(LIBRAPIDARCHIVE_DIR "${CMAKE_CURRENT_SOURCE_DIR}/../../vendor/librapidarchive")

set(LIBRAPIDARCHIVE_BUILD_BENCHMARKS OFF CACHE BOOL "" FORCE)
set(LIBRAPIDARCHIVE_BUILD_TESTS OFF CACHE BOOL "" FORCE)
set(LIBRAPIDARCHIVE_BUILD_TOOLS OFF CACHE BOOL "" FORCE)
set(LIBRAPIDARCHIVE_USE_SYSTEM_ZLIB OFF CACHE BOOL "" FORCE)
set(LIBRAPIDARCHIVE_USE_ZLIB_NG ON CACHE BOOL "" FORCE)
set(LIBRAPIDARCHIVE_WITH_RPMALLOC ON CACHE BOOL "" FORCE)
# Keep the less portable ISA-L path off on the CMake targets that have shown
# unstable CI behavior in this repository. Apple and Windows/MSVC still build
# with zlib-ng and the rest of the native stack, but avoid the assembler-heavy
# ISA-L dependency for reliability.
if(APPLE OR MSVC)
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
