cmake_minimum_required(VERSION 3.16)
project(rusty_bullet_capture CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

if(NOT BAKKESMOD_SDK_PATH)
    if(DEFINED ENV{APPDATA})
        set(BAKKESMOD_SDK_PATH "$ENV{APPDATA}/bakkesmod/bakkesmod/bakkesmodsdk"
            CACHE PATH "Path to a BakkesModSDK checkout (include/ + lib/pluginsdk.lib)")
    else()
        set(BAKKESMOD_SDK_PATH ""
            CACHE PATH "Path to a BakkesModSDK checkout (include/ + lib/pluginsdk.lib)")
    endif()
endif()

if(NOT EXISTS "${BAKKESMOD_SDK_PATH}/include/bakkesmod/plugin/bakkesmodplugin.h")
    message(FATAL_ERROR
        "BAKKESMOD_SDK_PATH ('${BAKKESMOD_SDK_PATH}') doesn't look like a BakkesModSDK "
        "checkout. Pass -DBAKKESMOD_SDK_PATH=<path to a clone of "
        "https://github.com/bakkesmodorg/BakkesModSDK>, or install BakkesMod once so it "
        "lands in %appdata%/bakkesmod/bakkesmod/bakkesmodsdk.")
endif()

add_library(rusty_bullet_capture MODULE
    RustyBulletCapturePlugin.cpp
    RustyBulletCapturePlugin.h
)

target_include_directories(rusty_bullet_capture PRIVATE "${BAKKESMOD_SDK_PATH}/include")
target_link_libraries(rusty_bullet_capture PRIVATE "${BAKKESMOD_SDK_PATH}/lib/pluginsdk.lib")

set_target_properties(rusty_bullet_capture PROPERTIES
    PREFIX ""
    SUFFIX ".dll"
)
