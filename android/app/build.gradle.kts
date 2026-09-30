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
        versionCode = 25
        versionName = "0.3.9"
    }

    // L'écran de l'application est l'interface web du client, la même que
    // celle que le PC sert aux iPhone : un seul fichier pour les deux.
    sourceSets {
        getByName("main") {
            assets.srcDirs("../../client-web")
        }
    }

    signingConfigs {
        // Clé de PUBLICATION, secrète : jamais dans le dépôt. Le workflow
        // de construction la reçoit des secrets GitHub
        // (ENVOYEUR_KEYSTORE_B64, ENVOYEUR_KEYSTORE_MDP) et la dépose à
        // l'endroit indiqué par ENVOYEUR_KEYSTORE.
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
            signingConfig = signingConfigs.findByName("publication") ?: signingConfigs.getByName("essai")
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
