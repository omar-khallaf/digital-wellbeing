function(generate_clangd_config PROJECT_ROOT_DIR)
    set(CLANGD_CONFIG_FILE "${PROJECT_ROOT_DIR}/.clangd")
    set(CLANGD_FLAGS "")

    # 0. Language standard.
    # clangd has no compile DB on host builds and defaults to a pre-C++20 std,
    # which hides C++20+ symbols (e.g. std::bit_cast) used by Hyprland headers.
    # Pin it to match the project's CMAKE_CXX_STANDARD exactly.
    if(DEFINED CMAKE_CXX_STANDARD AND CMAKE_CXX_STANDARD)
        if(CMAKE_CXX_EXTENSIONS)
            list(APPEND CLANGD_FLAGS "-std=gnu++${CMAKE_CXX_STANDARD}")
        else()
            list(APPEND CLANGD_FLAGS "-std=c++${CMAKE_CXX_STANDARD}")
        endif()
    endif()

    # 1. Host vs Cross detection
    if(CMAKE_SYSTEM_NAME STREQUAL CMAKE_HOST_SYSTEM_NAME AND NOT ANDROID)
        set(IS_HOST TRUE)
    else()
        set(IS_HOST FALSE)
    endif()

    # 2. Target & Sysroot
    if(CMAKE_CXX_COMPILER_TARGET)
        list(APPEND CLANGD_FLAGS "--target=${CMAKE_CXX_COMPILER_TARGET}")
    endif()
    if(CMAKE_SYSROOT)
        list(APPEND CLANGD_FLAGS "--sysroot=${CMAKE_SYSROOT}")
    endif()

    # 3. Add ALL Implicit Includes (Unfiltered)
    foreach(dir ${CMAKE_CXX_IMPLICIT_INCLUDE_DIRECTORIES})
        if(IS_HOST)
            if(NOT "${dir}" MATCHES "gcc/.*/include")
                list(APPEND CLANGD_FLAGS "-isystem" "${dir}")
            endif()
        else()
            list(APPEND CLANGD_FLAGS "-isystem" "${dir}")
        endif()
    endforeach()

    # 4. Universal FetchContent Discovery
    # This finds ANY 'include' directory inside the _deps folder (GTest, fmt, etc.)
    set(FETCH_DEPS_DIR "${CMAKE_BINARY_DIR}/_deps")
    if(EXISTS "${FETCH_DEPS_DIR}")
        file(GLOB_RECURSE FOUND_INCLUDES LIST_DIRECTORIES true "${FETCH_DEPS_DIR}/*/include")
        foreach(inc_dir ${FOUND_INCLUDES})
            # Filter: 1. Must be a directory
            #         2. Must NOT be inside a hidden folder (like .git)
            #         3. Must end EXACTLY with /include
            if(IS_DIRECTORY "${inc_dir}" AND
               NOT "${inc_dir}" MATCHES "\\.git" AND
               "${inc_dir}" MATCHES "/include$")
                list(APPEND CLANGD_FLAGS "-I" "${inc_dir}")
            endif()
        endforeach()
    endif()

    # 5. Staging Prefix (For Boost/Sqlite from ExternalProject)
    if(EXISTS "${CMAKE_STAGING_PREFIX}/include")
        list(APPEND CLANGD_FLAGS "-I" "${CMAKE_STAGING_PREFIX}/include")
    endif()

    # 6. Build the YAML Structure
    set(YAML "CompileFlags:\n")
    # Shield against Nix host leakage
    if(NOT IS_HOST)
        string(APPEND YAML "  Remove: [\"-isystem=/nix/store/*gcc*\", \"-isystem=/nix/store/*glibc*\", \"-isystem=/usr/*\", \"-I/usr/*\"]\n")
    endif()

    string(APPEND YAML "  Add: [\n")
    foreach(flag ${CLANGD_FLAGS})
        string(APPEND YAML "    \"${flag}\",\n")
    endforeach()
    if(NOT IS_HOST)
        string(APPEND YAML "    \"-nostdinc++\",\n    \"-nostdlibinc\",\n")
    endif()
    string(APPEND YAML "  ]\n\n")

    string(APPEND YAML "Index:\n  StandardLibrary: Yes\n\n")

    file(WRITE "${CLANGD_CONFIG_FILE}" "${YAML}")
    message(STATUS "LSP: Generated .clangd at ${PROJECT_ROOT_DIR}")
endfunction()

function(link_compile_commands TARGET_NAME BINARY_DIR PROJECT_ROOT_DIR)
    add_custom_target(link_${TARGET_NAME}_commands ALL
        COMMAND ${CMAKE_COMMAND} -E create_symlink
                "${BINARY_DIR}/compile_commands.json"
                "${PROJECT_ROOT_DIR}/compile_commands.json"
        COMMENT "Linking ${TARGET_NAME} compile_commands.json to root"
    )
endfunction()
