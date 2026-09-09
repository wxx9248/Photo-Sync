plugins {
    alias(libs.plugins.kotlin.jvm)
    alias(libs.plugins.protobuf)
}

// The wire schema, generated once and used by the application.
//
// This is a plain JVM module rather than part of `:app` because the protobuf Gradle plugin
// does not yet understand the Android plugin of AGP 9: it reaches for an extension type that
// version no longer has. Generating here and depending on the result costs nothing — the
// stubs are ordinary Java and Kotlin — and it keeps both plugins on versions that work.
//
// `protobuf-javalite` is what `STACK.md` §5.1 asks for: the full runtime carries reflection
// and descriptors an application this size never uses.

kotlin {
    jvmToolchain(25)
}

sourceSets {
    main {
        proto { srcDir("../../proto") }
    }
}

protobuf {
    protoc {
        artifact = libs.protoc.get().toString()
    }
    plugins {
        create("grpc") { artifact = libs.grpc.protoc.java.get().toString() }
        create("grpckt") { artifact = libs.grpc.protoc.kotlin.get().toString() + ":jdk8@jar" }
    }
    generateProtoTasks {
        all().forEach { task ->
            task.builtins {
                named("java") { option("lite") }
                create("kotlin") { option("lite") }
            }
            task.plugins {
                create("grpc") { option("lite") }
                create("grpckt") { option("lite") }
            }
        }
    }
}

dependencies {
    api(libs.protobuf.javalite)
    api(libs.protobuf.kotlin.lite)
    api(libs.grpc.protobuf.lite)
    api(libs.grpc.stub)
    api(libs.grpc.kotlin.stub)
    api(libs.coroutines.android)
    compileOnly(libs.javax.annotation)
}
