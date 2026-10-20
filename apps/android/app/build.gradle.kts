plugins {
    id("com.android.application")
}

android {
    namespace = "ai.chatcomputer.app"
    compileSdk = 36
    defaultConfig {
        applicationId = "ai.chatcomputer.app"
        minSdk = 24
        targetSdk = 36
        versionCode = 1
        versionName = "1.0.0"
    }
    // Signed with Chat Computer's own key, kept outside the repository: where it is, and its password, come from
    // the environment (see README.md, "The apps").
    signingConfigs {
        create("release") {
            storeFile = file(System.getenv("CHATCOMPUTER_KEYSTORE") ?: "no-keystore-given")
            storePassword = System.getenv("CHATCOMPUTER_KEYSTORE_PASSWORD")
            keyAlias = "chatcomputer"
            keyPassword = System.getenv("CHATCOMPUTER_KEYSTORE_PASSWORD")
        }
    }
    buildTypes {
        release {
            isMinifyEnabled = false
            signingConfig = signingConfigs.getByName("release")
        }
    }
    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

dependencies {
    // Google's helper for Trusted Web Activities: its LauncherActivity opens the site in Chrome (or another browser
    // that can), full screen.
    implementation("com.google.androidbrowserhelper:androidbrowserhelper:2.7.4")
}
