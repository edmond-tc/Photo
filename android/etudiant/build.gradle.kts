plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

// « Amphi » : la petite application des étudiants. Elle se connecte au
// Wi-Fi de l'amphi sans couper les données mobiles et ouvre le portail.
android {
    namespace = "bj.photocopie.amphi"
    compileSdk = 35

    defaultConfig {
        applicationId = "bj.photocopie.amphi"
        // Android 10 : première version où une application peut demander
        // un Wi-Fi local (WifiNetworkSpecifier) sans toucher à Internet.
        minSdk = 29
        targetSdk = 35
        versionCode = 1
        versionName = "1.0.0"
    }

    signingConfigs {
        // Même clé que l'envoyeur (voir app/build.gradle.kts).
        val clePublication = System.getenv("ENVOYEUR_KEYSTORE")
        if (!clePublication.isNullOrBlank() && file(clePublication).exists()) {
            create("publication") {
                storeFile = file(clePublication)
                storePassword = System.getenv("ENVOYEUR_KEYSTORE_MDP")
                keyAlias = "envoyeur"
                keyPassword = System.getenv("ENVOYEUR_KEYSTORE_MDP")
            }
        }
        create("essai") {
            storeFile = file("../envoyeur-essai.jks")
            storePassword = "envoyeur-essai"
            keyAlias = "essai"
            keyPassword = "envoyeur-essai"
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            signingConfig = signingConfigs.findByName("publication") ?: signingConfigs.getByName("essai")
        }
    }

    lint {
        checkReleaseBuilds = false
        abortOnError = false
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
    kotlinOptions {
        jvmTarget = "17"
    }
}
