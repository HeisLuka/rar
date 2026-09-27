plugins {
    id("com.android.application")
}

android {
    namespace = "com.chaptera.reader"
    compileSdk = 37

    defaultConfig {
        applicationId = "com.chaptera.reader"
        minSdk = 26
        targetSdk = 37
        versionCode = 1
        versionName = "0.1.0"

        ndk {
            abiFilters += "arm64-v8a"
        }
    }

    sourceSets {
        getByName("main") {
            jniLibs.srcDir("src/main/jniLibs")
        }
    }
}
