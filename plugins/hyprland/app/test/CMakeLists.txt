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

fetchcontent_declare(
    sdbus-cpp
    GIT_REPOSITORY https://github.com/Kistler-Group/sdbus-cpp.git
    GIT_TAG v2.3.1
    GIT_SHALLOW True
    GIT_PROGRESS True
    CMAKE_ARGS
        -DCMAKE_POSITION_INDEPENDENT_CODE:BOOL=ON
        -DBUILD_SHARED_LIBS:BOOL=OFF
        -DSDBUSCPP_BUILD_DOCS:BOOL=OFF
        -DSDBUSCPP_BUILD_CODEGEN:BOOL=OFF
        -DSDBUSCPP_BUILD_TESTS:BOOL=OFF
        -DSDBUSCPP_BUILD_LIBSYSTEMD:BOOL=OFF
)
fetchcontent_makeavailable(sdbus-cpp)

# LockManager tests — standalone (types.hpp + std only; no D-Bus).
add_executable(${PROJECT_NAME}
    test.cpp
    ../src/lockdown.cpp
)

target_include_directories(${PROJECT_NAME} PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}/../include
    ${CMAKE_CURRENT_SOURCE_DIR}/../src)

target_link_libraries(${PROJECT_NAME} PRIVATE GTest::gtest)
target_link_libraries(${PROJECT_NAME} PRIVATE sdbus-c++)

# ── D-Bus serialization contract tests ──────────────────────────────────────
# sdbus-c++ target is available from the parent's FetchContent (v2.3.1, static).
add_executable(wellbeing-lockdown-dbus-test
    dbus_serialization_test.cpp
)

target_include_directories(wellbeing-lockdown-dbus-test PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}/../include)

target_link_libraries(wellbeing-lockdown-dbus-test
    PRIVATE
        GTest::gtest
        sdbus-c++
)

# Register both tests with CTest.
add_test(NAME wellbeing-lockdown-test COMMAND ${PROJECT_NAME})
add_test(NAME wellbeing-lockdown-dbus-test COMMAND wellbeing-lockdown-dbus-test)
