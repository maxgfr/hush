import org.jetbrains.kotlin.gradle.dsl.JvmTarget

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.compose)
}

// Release builds take their version from the release (HUSH_VERSION, e.g. 2.1.0) and are signed
// with the key CI holds as a secret (HUSH_ANDROID_KEYSTORE, a PKCS#12 file, and its password).
val releaseVersion: String? = System.getenv("HUSH_VERSION")
val keystore: String? = System.getenv("HUSH_ANDROID_KEYSTORE")

android {
    namespace = "io.github.maxgfr.hush"
    compileSdk = 37

    defaultConfig {
        applicationId = "io.github.maxgfr.hush"
        minSdk = 29
        targetSdk = 35
        versionName = releaseVersion ?: "0.1.0"
        versionCode = releaseVersion
            ?.split(".")
            ?.map { it.takeWhile(Char::isDigit).toIntOrNull() ?: 0 }
            ?.let { (major, minor, patch) -> major * 10_000 + minor * 100 + patch }
            ?: 1
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        // The engine is built for these two (mobile/scripts/build-android.sh): phones and the emulator.
        ndk { abiFilters += listOf("arm64-v8a", "x86_64") }
    }

    if (keystore != null) {
        signingConfigs {
            create("release") {
                storeFile = file(keystore)
                storeType = "pkcs12"
                storePassword = System.getenv("HUSH_ANDROID_KEYSTORE_PASSWORD")
                keyAlias = System.getenv("HUSH_ANDROID_KEY_ALIAS") ?: "hush"
                keyPassword = System.getenv("HUSH_ANDROID_KEYSTORE_PASSWORD")
            }
        }
    }

    buildTypes {
        // The test build CI publishes on every run: its own id, so it installs next to the release.
        debug {
            applicationIdSuffix = ".debug"
            versionNameSuffix = "-debug"
        }
        release {
            if (keystore != null) signingConfig = signingConfigs.getByName("release")
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures {
        compose = true
    }

    testOptions {
        unitTests.isReturnDefaultValues = true
    }

    lint {
        lintConfig = file("lint.xml")
        abortOnError = true
        checkReleaseBuilds = false
    }

    packaging {
        resources.excludes += "/META-INF/{AL2.0,LGPL2.1}"
    }
}

kotlin {
    compilerOptions {
        jvmTarget.set(JvmTarget.JVM_17)
    }
}

dependencies {
    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.lifecycle.runtime.compose)
    implementation(libs.androidx.lifecycle.viewmodel.compose)
    implementation(platform(libs.compose.bom))
    implementation(libs.compose.ui)
    implementation(libs.compose.foundation)
    implementation(libs.compose.material3)
    implementation(libs.kotlinx.coroutines.android)
    implementation(libs.camera.camera2)
    implementation(libs.camera.lifecycle)
    implementation(libs.camera.view)
    implementation(libs.zxing.core)
    // The generated bindings (uniffi.hush_ffi) load the engine through JNA.
    implementation(libs.jna) { artifact { type = "aar" } }

    testImplementation(libs.junit)

    androidTestImplementation(platform(libs.compose.bom))
    androidTestImplementation(libs.compose.ui.test.junit4)
    androidTestImplementation(libs.androidx.test.runner)
    androidTestImplementation(libs.androidx.test.rules)
    androidTestImplementation(libs.androidx.test.ext.junit)
    debugImplementation(libs.compose.ui.test.manifest)
}
