plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "bj.photocopie.envoyeur"
    compileSdk = 35

    defaultConfig {
        applicationId = "bj.photocopie.envoyeur"
        // Android 10 : première version où l'application peut créer son
        // propre réseau (Wi-Fi Direct) avec un nom et un mot de passe
        // choisis, sans fenêtre de confirmation.
        minSdk = 29
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0"
    }

    signingConfigs {
        create("essai") {
            // Clé d'ESSAI, publique dans le dépôt : elle permet d'installer
            // les versions successives l'une sur l'autre pendant les tests.
            // À remplacer par une clé secrète avant toute diffusion aux clients.
            storeFile = file("../envoyeur-essai.jks")
            storePassword = "envoyeur-essai"
            keyAlias = "essai"
            keyPassword = "envoyeur-essai"
        }
    }

    buildTypes {
        release {
            isMinifyEnabled = false
            signingConfig = signingConfigs.getByName("essai")
        }
    }

    lint {
        // Version d'essai : un avertissement de style ne doit pas bloquer
        // la construction de l'APK.
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
