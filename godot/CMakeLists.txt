cmake_minimum_required(VERSION 3.17)

project(mapcraft LANGUAGES C CXX)

set(LIBNAME "mapcraft" CACHE STRING "Library name")

# Point FETCHCONTENT_SOURCE_DIR_GODOT-CPP at a local godot-cpp checkout to build offline.
include(FetchContent)
FetchContent_Declare(
    godot-cpp
    GIT_REPOSITORY https://github.com/godotengine/godot-cpp.git
    GIT_TAG b0e3b1e4b78a606f48d162898afb5eeda533d2a9
    SYSTEM
)
FetchContent_MakeAvailable(godot-cpp)

get_target_property(GODOTCPP_SUFFIX godot::cpp GODOTCPP_SUFFIX)
get_target_property(GODOTCPP_PLATFORM godot::cpp GODOTCPP_PLATFORM)

set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

add_library(${LIBNAME} SHARED)
file(GLOB MAPCRAFT_SOURCES CONFIGURE_DEPENDS "${CMAKE_CURRENT_LIST_DIR}/src/*.cpp" "${CMAKE_CURRENT_LIST_DIR}/src/*.h")
target_sources(${LIBNAME} PRIVATE ${MAPCRAFT_SOURCES})
target_link_libraries(${LIBNAME} PRIVATE godot-cpp)

set_target_properties(${LIBNAME}
    PROPERTIES
    RUNTIME_OUTPUT_DIRECTORY "$<1:${CMAKE_CURRENT_LIST_DIR}/addons/mapcraft/bin/${GODOTCPP_PLATFORM}>"
    LIBRARY_OUTPUT_DIRECTORY "$<1:${CMAKE_CURRENT_LIST_DIR}/addons/mapcraft/bin/${GODOTCPP_PLATFORM}>"
    PREFIX ""
    OUTPUT_NAME "${LIBNAME}${GODOTCPP_SUFFIX}"
)
