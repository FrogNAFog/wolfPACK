plugins {
    // AGP 9.2.x enables built-in Kotlin (KGP 2.2.10) — do not apply org.jetbrains.kotlin.android.
    id("com.android.application") version "9.2.1" apply false
    // Compose compiler plugin; version must match built-in KGP (2.2.10) bundled with AGP 9.
    id("org.jetbrains.kotlin.plugin.compose") version "2.2.10" apply false
    // KSP aligned with AGP 9 built-in Kotlin toolchain.
    id("com.google.devtools.ksp") version "2.2.10-2.0.2" apply false
}
