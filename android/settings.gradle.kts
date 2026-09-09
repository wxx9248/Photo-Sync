pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "photo-sync"

// The session state machine holds no Android types, so it builds and tests on a plain JVM
// without the SDK. See docs/CONVENTIONS-KOTLIN.md rule K13.
include(":session")

// Generated wire stubs, kept out of the Android module: see protocol/build.gradle.kts.
include(":protocol")
include(":app")
