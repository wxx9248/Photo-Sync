plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
}

android {
    namespace = "top.wxx9248.photosync"
    compileSdk = 36

    defaultConfig {
        applicationId = "top.wxx9248.photosync"

        // Android 12. The permission model in docs/SPEC.md section 3.1 starts here.
        minSdk = 31
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0"
    }

    kotlin {
        jvmToolchain(21)
    }
}

dependencies {
    implementation(project(":session"))
}
