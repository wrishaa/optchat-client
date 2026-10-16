plugins { id("com.android.application") }

val optchatVersionCode = providers.gradleProperty("optchatVersionCode").orElse("1").get().toInt()
val cargoVersion = Regex("""(?m)^version = "(\d+\.\d+\.\d+)"$""")
    .find(rootProject.file("../../../Cargo.toml").readText())?.groupValues?.get(1)
    ?: error("Cannot read Cargo workspace version")
val optchatVersionName = providers.gradleProperty("optchatVersionName").orElse("$cargoVersion-dev.$optchatVersionCode").get()
require(optchatVersionCode in 1..2100000000) { "Invalid OptChat versionCode" }
val releaseKeystore = providers.environmentVariable("OPTCHAT_ANDROID_KEYSTORE").orNull

android {
    namespace = "com.reframe.optchat.gpui.android"
    compileSdk = 36
    buildToolsVersion = "36.0.0"
    ndkVersion = "29.0.14206865"
    defaultConfig {
        applicationId = "com.reframe.optchat.gpui.android"
        minSdk = 26
        targetSdk = 36
        versionCode = optchatVersionCode
        versionName = optchatVersionName
        testInstrumentationRunner = "com.reframe.optchat.gpui.android.OptChatDeviceChecks"
        ndk { abiFilters += "arm64-v8a" }
    }
    signingConfigs {
        if (releaseKeystore != null) {
            create("optchatRelease") {
                storeFile = file(releaseKeystore)
                storePassword = providers.environmentVariable("OPTCHAT_ANDROID_STORE_PASSWORD").get()
                keyAlias = providers.environmentVariable("OPTCHAT_ANDROID_KEY_ALIAS").get()
                keyPassword = providers.environmentVariable("OPTCHAT_ANDROID_KEY_PASSWORD").get()
            }
        }
    }
    buildTypes {
        debug { isDebuggable = true; isJniDebuggable = true }
        release {
            isDebuggable = false
            isJniDebuggable = false
            if (releaseKeystore != null) { signingConfig = signingConfigs.getByName("optchatRelease") }
        }
    }
    sourceSets["main"].jniLibs.srcDir("../../../../build/android/app/jniLibs")
    packaging { jniLibs { useLegacyPackaging = false } }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    lint { abortOnError = true }
}
