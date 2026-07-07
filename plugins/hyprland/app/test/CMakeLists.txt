cmake_minimum_required(VERSION 3.27)
project(wellbeing-lockdown-test LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 23)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

include(FetchContent)

fetchcontent_declare(
    googletest
    GIT_REPOSITORY https://github.com/google/googletest.git
    GIT_TAG        v1.17.0
    EXCLUDE_FROM_ALL
)

fetchcontent_makeavailable(googletest)

# lockdown.cpp is standalone (types.hpp + std only; Hyprland/sdbus live in
# main.cpp, which is intentionally excluded from the unit-test build).
add_executable(${PROJECT_NAME} test.cpp ../src/lockdown.cpp)

target_include_directories(${PROJECT_NAME} PRIVATE
  ${CMAKE_CURRENT_SOURCE_DIR}/../include
  ${CMAKE_CURRENT_SOURCE_DIR}/../src)

target_link_libraries(${PROJECT_NAME} PRIVATE GTest::gtest)

# Register the executable with CTest. `enable_testing()` is called in the parent
# (app/) CMakeLists, so this test is discoverable by the superbuild's delegated
# CTest invocation (`ctest --preset test-host`).
add_test(NAME ${PROJECT_NAME} COMMAND ${PROJECT_NAME})
