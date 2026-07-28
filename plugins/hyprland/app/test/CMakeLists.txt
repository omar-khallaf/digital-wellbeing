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

# LockManager tests — standalone (types.hpp + messages.hpp + std only)
add_executable(${PROJECT_NAME}
    test.cpp
    ../src/lockdown.cpp
)

target_include_directories(${PROJECT_NAME} PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}/../include)

target_link_libraries(${PROJECT_NAME} PRIVATE GTest::gtest)

# Register test with CTest.
add_test(NAME wellbeing-lockdown-test COMMAND ${PROJECT_NAME})
