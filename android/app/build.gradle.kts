plugins {
    // Kotlin support is built into the Android plugin from AGP 9 on, so the separate Kotlin
    // plugin is not applied here — applying it is an error.
    alias(libs.plugins.android.application)
    alias(libs.plugins.compose.compiler)
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

        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    buildFeatures {
        compose = true
    }

    kotlin {
        jvmToolchain(25)
    }

    // A device the build can create and drive itself, so the platform surface is checked
    // without anybody plugging a phone in. docs/ROADMAP.md M8.
    testOptions {
        managedDevices {
            localDevices {
                create("phone") {
                    device = "Pixel 6"
                    apiLevel = 34
                    systemImageSource = "aosp-atd"
                }
            }
        }
    }
}

dependencies {
    implementation(project(":session"))
    implementation(project(":protocol"))
    implementation(libs.grpc.okhttp)
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.lifecycle.service)
    implementation(libs.androidx.lifecycle.viewmodel.compose)
    implementation(platform(libs.compose.bom))
    implementation(libs.compose.material3)
    implementation(libs.compose.ui)
    implementation(libs.compose.ui.tooling.preview)

    androidTestImplementation(libs.androidx.test.runner)
    androidTestImplementation(libs.androidx.test.ext.junit)
}
