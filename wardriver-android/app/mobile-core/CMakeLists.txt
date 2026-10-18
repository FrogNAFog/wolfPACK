# CMake hook for AGP externalNativeBuild — real logic is Rust via cargo-ndk.
#
# Prerequisites (host machine):
#   rustup target add aarch64-linux-android
#   cargo install cargo-ndk
#   Android NDK via SDK Manager (AGP supplies android.ndkDirectory)
#
# Gradle task `cargoBuildMobileCore` (app/build.gradle.kts) runs:
#   cargo ndk -t arm64-v8a -o mobile-core/prebuilt build -p wardriver-mobile-core --features jni --release
#
# This file imports the prebuilt libwardriver_mobile_core.so so AGP packages it in the APK.

cmake_minimum_required(VERSION 3.22.1)
project(wardriver_mobile_core)

# arm64-v8a only for MVP (matches abiFilters in Gradle).
set(RUST_PREBUILT "${CMAKE_CURRENT_SOURCE_DIR}/prebuilt/${ANDROID_ABI}/libwardriver_mobile_core.so")

if(NOT EXISTS "${RUST_PREBUILT}")
    message(FATAL_ERROR
        "Missing ${RUST_PREBUILT}. Run ./gradlew :app:cargoBuildMobileCore first "
        "(requires cargo-ndk and aarch64-linux-android Rust target).")
endif()

# IMPORTED shared lib — AGP copies into APK jniLibs.
add_library(wardriver_mobile_core SHARED IMPORTED GLOBAL)
set_target_properties(wardriver_mobile_core PROPERTIES
    IMPORTED_LOCATION "${RUST_PREBUILT}"
)
