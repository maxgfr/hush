plugins {
    alias(libs.plugins.android.application) apply false
    // AGP compiles Kotlin itself; declaring the Kotlin plugin here only picks its version.
    alias(libs.plugins.kotlin.android) apply false
    alias(libs.plugins.kotlin.compose) apply false
}
