// Chat Computer for Android: chatcomputer.ai as an app (a Trusted Web Activity: Chrome shows the site full screen,
// with no address bar, once the site says this app is its own: docs/.well-known/assetlinks.json).
pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}
dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}
rootProject.name = "Chat Computer"
include(":app")
