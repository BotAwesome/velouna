// The app is a NativeActivity around the Rust library libvelour_android.so,
// which android/build.ps1 builds with cargo-ndk into src/main/jniLibs.
plugins {
    id("com.android.application")
}

android {
    namespace = "app.velour"
    compileSdk = 36

    defaultConfig {
        applicationId = "app.velour"
        minSdk = 26
        targetSdk = 35
        // build.ps1 passes the version from Cargo.toml; the code must grow
        // with every release for updates to install (1.2.3 -> 10203).
        val ver = (project.findProperty("velourVersion") as String?) ?: "0.1.0"
        val parts = ver.substringBefore('-').split(".").map { it.toIntOrNull() ?: 0 } + listOf(0, 0, 0)
        versionCode = maxOf(1, parts[0] * 10000 + parts[1] * 100 + parts[2])
        versionName = ver
        ndk {
            // build.ps1 passes -Pabis=… so a phone-only build also leaves out
            // the emulator (x86_64) copies of mpv/FFmpeg.
            val abis = (project.findProperty("abis") as String?) ?: "arm64-v8a,x86_64"
            abiFilters += abis.split(",").map { it.trim() }.filter { it.isNotEmpty() }
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            // Until a release key exists, release builds use the debug key.
            signingConfig = signingConfigs.getByName("debug")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

dependencies {
    // libmpv + FFmpeg for Android (MIT, github.com/jarnedemeulemeester/libmpv-android)
    implementation("dev.jdtech.mpv:libmpv:1.0.0")
}
