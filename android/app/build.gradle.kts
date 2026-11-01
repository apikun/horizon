plugins {
    id("com.android.application")
}

android {
    namespace = "dev.playhorizon.android"
    compileSdk = 35

    defaultConfig {
        applicationId = "dev.playhorizon.android"
        minSdk = 31
        targetSdk = 35
        versionCode = 1
        versionName = "26.3"
        ndk {
            // The engine builds for arm64 (tools/build_android.py); a
            // 64-bit-only app keeps the APK small and the devices supported.
            abiFilters += "arm64-v8a"
        }
    }

    buildTypes {
        release {
            // Debug-signed so it installs without a keystore; sign it
            // properly before sharing (docs/android.md).
        }
    }

    // The engine's shared library, copied here by tools/build_android.py.
    sourceSets["main"].jniLibs.srcDir("src/main/jniLibs")
    packaging {
        jniLibs {
            // The .so loads straight out of the APK (no extraction), as
            // NativeActivity expects since API 23.
            useLegacyPackaging = false
        }
    }
}
