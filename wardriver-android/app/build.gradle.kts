plugins {
    id("com.android.application")
    // Built-in Kotlin from AGP 9 — no org.jetbrains.kotlin.android plugin.
    id("org.jetbrains.kotlin.plugin.compose")
    id("com.google.devtools.ksp")
}

android {
    namespace = "net.wardriver.mobile"
    // API 36 (Android 16) — matches AGP 9.2 / mid-2026 toolchain targets.
    compileSdk = 36

    defaultConfig {
        applicationId = "net.wardriver.mobile"
        minSdk = 29
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0"

        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"

        // Rust cdylib MVP: arm64-v8a only (see mobile-core/CMakeLists.txt).
        ndk {
            abiFilters += "arm64-v8a"
        }

        externalNativeBuild {
            cmake {
                // IMPORTED prebuilt from cargo-ndk output (mobile-core/prebuilt/).
                arguments += listOf("-DANDROID_STL=c++_shared")
            }
        }
    }

    // CMake packages libwardriver_mobile_core.so built by cargo-ndk.
    externalNativeBuild {
        cmake {
            path = file("mobile-core/CMakeLists.txt")
        }
    }

    // IMPORTED CMake target alone does not copy the prebuilt .so into the APK.
    sourceSets {
        getByName("main") {
            jniLibs.srcDir("mobile-core/prebuilt")
        }
    }

    buildFeatures {
        compose = true
    }

    compileOptions {
        // AGP 9 minimum / recommended JDK for compile and runtime classpaths.
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    packaging {
        resources {
            excludes += "/META-INF/{AL2.0,LGPL2.1}"
        }
    }
}

// kotlinOptions{} moved out of android{} for AGP 9 built-in Kotlin.
kotlin {
    compilerOptions {
        jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)
    }
}

// Build Rust cdylib before CMake packages the prebuilt .so into the APK.
// Requires: rustup target add aarch64-linux-android && cargo install cargo-ndk
val repoRoot = rootProject.projectDir.parentFile

tasks.register<Exec>("cargoBuildMobileCore") {
    group = "build"
    description = "Build libwardriver_mobile_core.so via cargo-ndk (arm64-v8a)"
    workingDir = repoRoot
    commandLine(
        "cargo", "ndk",
        "-t", "arm64-v8a",
        "-o", "${projectDir}/mobile-core/prebuilt",
        "build",
        "-p", "wardriver-mobile-core",
        "--features", "jni",
        "--release",
    )
    // Only rebuild when Rust sources change.
    inputs.dir(repoRoot.resolve("wardriver-mobile-core/src"))
    inputs.file(repoRoot.resolve("wardriver-mobile-core/Cargo.toml"))
    outputs.dir("${projectDir}/mobile-core/prebuilt/arm64-v8a")
}

tasks.named("preBuild") {
    dependsOn("cargoBuildMobileCore")
}

dependencies {
    // Compose BOM pins compatible ui/material3/runtime versions (2026 stable train).
    val composeBom = platform("androidx.compose:compose-bom:2026.06.00")
    implementation(composeBom)
    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.ui:ui-text-google-fonts")
    implementation("androidx.compose.ui:ui-tooling-preview")
    debugImplementation("androidx.compose.ui:ui-tooling")

    implementation("androidx.activity:activity-compose:1.10.1")
    implementation("androidx.core:core-ktx:1.16.0")
    implementation("androidx.lifecycle:lifecycle-runtime-ktx:2.9.0")
    implementation("androidx.lifecycle:lifecycle-process:2.9.0")
    implementation("androidx.lifecycle:lifecycle-service:2.9.0")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.9.0")

    // Bottom-nav shell, persisted geofence prefs, extended Material icons.
    implementation("androidx.navigation:navigation-compose:2.9.0")
    implementation("androidx.datastore:datastore-preferences:1.1.7")
    implementation("androidx.compose.material:material-icons-extended")

    // Room 2.7.x + KSP (generateKotlin default true in 2.7+).
    val roomVersion = "2.7.2"
    implementation("androidx.room:room-runtime:$roomVersion")
    implementation("androidx.room:room-ktx:$roomVersion")
    ksp("androidx.room:room-compiler:$roomVersion")

    implementation("androidx.work:work-runtime-ktx:2.10.2")
    implementation("com.squareup.okhttp3:okhttp:4.12.0")
    implementation("androidx.security:security-crypto:1.1.0-alpha06")

    implementation("com.google.android.gms:play-services-location:21.3.0")
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-play-services:1.10.2")
    implementation("org.osmdroid:osmdroid-android:6.1.20")

    val cameraVersion = "1.4.2"
    implementation("androidx.camera:camera-camera2:$cameraVersion")
    implementation("androidx.camera:camera-lifecycle:$cameraVersion")
    implementation("androidx.camera:camera-view:$cameraVersion")
    implementation("com.google.mlkit:barcode-scanning:17.3.0")

    androidTestImplementation("androidx.test.ext:junit:1.2.1")
    androidTestImplementation("androidx.test:runner:1.6.1")
    androidTestImplementation("androidx.room:room-testing:$roomVersion")

    testImplementation("junit:junit:4.13.2")
    testImplementation("org.json:json:20240303")
}
