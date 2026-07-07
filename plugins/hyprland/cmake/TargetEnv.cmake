# 1. Load Toolchain if provided
if(CMAKE_TOOLCHAIN_FILE)
    include(${CMAKE_TOOLCHAIN_FILE})
endif()

# 2. Manual Compiler Detection (Fallback for Host builds)
if(NOT CMAKE_C_COMPILER)
    find_program(CMAKE_C_COMPILER NAMES clang gcc cc)
    find_program(CMAKE_CXX_COMPILER NAMES clang++ g++ c++)
endif()

if(NOT CMAKE_AR)
    find_program(CMAKE_AR NAMES llvm-ar ar)
endif()

if(NOT CMAKE_RANLIB)
    find_program(CMAKE_RANLIB NAMES llvm-ranlib ranlib)
endif()

# 3. Detect Architecture & System Name
if(NOT CMAKE_SYSTEM_NAME)
    set(CMAKE_SYSTEM_NAME ${CMAKE_HOST_SYSTEM_NAME})
endif()

if(NOT CMAKE_SYSTEM_PROCESSOR)
    set(CMAKE_SYSTEM_PROCESSOR ${CMAKE_HOST_SYSTEM_PROCESSOR})
endif()

# 4. Normalize TARGET_HOST for Autotools
if(NOT TARGET_HOST)
    execute_process(COMMAND ${CMAKE_C_COMPILER} -dumpmachine
                    OUTPUT_VARIABLE TARGET_HOST
                    OUTPUT_STRIP_TRAILING_WHITESPACE)
endif()

# 6. Setup Staging and Parallelism
set(CMAKE_STAGING_PREFIX "${CMAKE_BINARY_DIR}/staging" CACHE PATH "Staging Area")
file(MAKE_DIRECTORY "${CMAKE_STAGING_PREFIX}/include" "${CMAKE_STAGING_PREFIX}/lib")

include(ProcessorCount)
processorcount(NUM_CORES)
if(NUM_CORES EQUAL 0)
    set(NUM_CORES 2)
endif()
set(PARALLEL_LEVEL ${NUM_CORES})

# 7. Define the Universal Arguments
set(COMMON_CMAKE_ARGS
    # The final destination for headers/libs on the "target" or staging area.
    -DCMAKE_INSTALL_PREFIX:PATH=${CMAKE_STAGING_PREFIX}

    # Used in cross-compilation to tell CMake where the 'staged' files live on the host.
    # Often matches INSTALL_PREFIX in Superbuilds to ensure internal consistency.
    -DCMAKE_STAGING_PREFIX:PATH=${CMAKE_STAGING_PREFIX}

    # Passes the platform/compiler configuration to child projects.
    -DCMAKE_TOOLCHAIN_FILE:FILEPATH=${CMAKE_TOOLCHAIN_FILE}

    # Defines the 'sysroot' or 'jail'. Combined with 'ONLY' modes, it forces
    # child projects to ignore system libraries and only see what's in staging.
    -DCMAKE_FIND_ROOT_PATH:PATH=${CMAKE_STAGING_PREFIX}

    # Explicitly adds the staging folder to the search list. While FIND_ROOT_PATH
    # acts as a filter, this tells find_package() specifically where to look first.
    -DCMAKE_PREFIX_PATH:PATH=${CMAKE_STAGING_PREFIX}

    # The bridge for pkg-config. Tells CMake to pass the paths in CMAKE_PREFIX_PATH
    # to the pkg-config tool, helping it find .pc files in staging/lib/pkgconfig.
    -DPKG_CONFIG_USE_CMAKE_PREFIX_PATH:BOOL=ON

    # Ensures child projects match the debug/release state of the superbuild.
    -DCMAKE_BUILD_TYPE:STRING=${CMAKE_BUILD_TYPE}

    # Required for static libraries that will eventually be linked into shared objects
    # or position-independent executables (prevents 'relocation R_X86_64_32' errors).
    -DCMAKE_POSITION_INDEPENDENT_CODE:BOOL=ON

    -DCMAKE_MODULE_PATH:PATH=${CMAKE_MODULE_PATH}
    -DCMAKE_GENERATOR:STRING=${CMAKE_GENERATOR}
    -DCMAKE_C_COMPILER:STRING=${CMAKE_C_COMPILER}
    -DCMAKE_CXX_COMPILER:STRING=${CMAKE_CXX_COMPILER}
    -DCMAKE_AR:STRING=${CMAKE_AR}
    -DCMAKE_RANLIB:STRING=${CMAKE_RANLIB}
    -DCMAKE_SYSTEM_NAME:STRING=${CMAKE_SYSTEM_NAME}
    -DCMAKE_SYSTEM_PROCESSOR:STRING=${CMAKE_SYSTEM_PROCESSOR}
    -DTARGET_HOST:STRING=${TARGET_HOST}
    -DPARALLEL_LEVEL:STRING=${PARALLEL_LEVEL}
    -DCMAKE_EXPORT_COMPILE_COMMANDS:BOOL=ON

    -DCMAKE_FIND_ROOT_PATH_MODE_PACKAGE:STRING=ONLY
    -DCMAKE_FIND_ROOT_PATH_MODE_LIBRARY:STRING=ONLY
    -DCMAKE_FIND_ROOT_PATH_MODE_INCLUDE:STRING=ONLY
    -DCMAKE_FIND_ROOT_PATH_MODE_PROGRAM:STRING=NEVER  # Use host tools (compilers/sh), not target ones
)
