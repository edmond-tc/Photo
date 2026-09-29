package bj.photocopie.envoyeur

import android.Manifest
import android.annotation.SuppressLint
import android.app.Activity
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothManager
import android.content.ActivityNotFoundException
import android.content.ClipData
import android.content.BroadcastReceiver
import android.content.Context
import android.content.IntentFilter
import android.content.ContentValues
import android.content.Intent
import android.content.pm.PackageManager
import android.hardware.Sensor
import android.hardware.SensorEvent
import android.hardware.SensorEventListener
import android.hardware.SensorManager
import android.media.AudioManager
import android.media.MediaRecorder
import android.speech.tts.TextToSpeech
import android.net.Uri
import android.net.wifi.WifiManager
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.provider.MediaStore
import android.provider.OpenableColumns
import android.provider.Settings
import android.util.Log
import android.view.WindowManager
import android.webkit.JavascriptInterface
import android.webkit.PermissionRequest
import android.webkit.ValueCallback
import android.webkit.WebChromeClient
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient
import org.json.JSONArray
import org.json.JSONObject
import java.util.concurrent.Executors

/**
 * L'écran de l'application est l'interface web du client
 * (`client-web/index.html`, la même que celle servie par le PC aux iPhone).
 * Cette activité lui donne les pouvoirs qu'une page web n'a pas :
 * créer le réseau et trouver le PC (voir [Liaison]), les autorisations,
 * l'appareil photo, le micro, et les documents reçus par « Partager ».
 */
class MainActivity : Activity() {
    companion object {
        const val ACTION_CHOISIR = "bj.photocopie.envoyeur.CHOISIR"

        /** Application à l'écran : inutile de proposer l'envoi par notification. */
        @Volatile var visible = false
        private const val DEMANDE_FICHIERS = 10
        private const val DEMANDE_AUTORISATIONS = 11
        private const val DEMANDE_MICRO = 12
        private const val DEMANDE_PHOTO = 13
        private const val DEMANDE_BLUETOOTH = 14
        private const val DEMANDE_ACTIVER_BLUETOOTH = 15
        private const val DEMANDE_REGLAGE_WIFI = 16
        private const val DEMANDE_REGLAGE_BLUETOOTH = 17
        private const val DEMANDE_MAM = 18
        private const val DEMANDE_MAM_FICHIERS = 19

        /** Adresse imaginaire, servie par l'application elle-même (voir [ClientWeb]). */
        private const val HOTE = "envoyeur.kiosque"

        /**
         * Sans signe de vie de l'interface (client parti), le réseau est
         * supprimé au bout de ce délai : le PC ne reste pas bloqué. Tant que
         * le client choisit ou envoie, l'interface le repousse
         * (`garderLien`), même pour un très gros fichier.
         */
        private const val INACTIVITE_MAX_MS = 3 * 60 * 1000L

        /** Après l'envoi, le temps de voir « En impression » avant de libérer le PC. */
        private const val APRES_ENVOI_MS = 30 * 1000L
    }

    private lateinit var web: WebView
    private val principal = Handler(Looper.getMainLooper())
    private val executeur = Executors.newSingleThreadExecutor()

    private var rappelFichiers: ValueCallback<Array<Uri>>? = null
    private var photo: Uri? = null
    private var demandeMicro: PermissionRequest? = null

    /** Documents reçus par « Partager », servis à l'interface sous /partage/<n>. */
    private val partages = mutableListOf<Uri>()
    private var partagesLus = 0

    /** Voix du téléphone (française), pour dire « C'est envoyé… ». */
    private var voix: TextToSpeech? = null
    private var voixPrete = false

    /** Message vocal en cours d'enregistrement, et ceux déjà faits (servis sous /vocal/<n>). */
    private var enregistreur: MediaRecorder? = null
    private val vocaux = mutableListOf<java.io.File>()

    /** Wi-Fi ou Bluetooth allumé ou éteint pendant que l'application est ouverte. */
    private val recepteurRadios = object : BroadcastReceiver() {
        override fun onReceive(c: Context, i: Intent) {
            // Le client vient d'allumer le Wi-Fi (ou le Bluetooth) dans le
            // panneau ouvert par l'application : on referme ce panneau, il
            // revient tout seul à l'application, sans toucher « retour ».
            if (wifiAllume()) finishActivity(DEMANDE_REGLAGE_WIFI)
            if (bluetoothAllume() == true) {
                finishActivity(DEMANDE_REGLAGE_BLUETOOTH)
                Balayage.demarrer(this@MainActivity)
            }
            signalerRadios()
        }
    }

    /** « Main à main » : fichiers choisis pour l'envoi, et action en attente d'autorisation. */
    private val mamFichiers = mutableListOf<MainAMain.FichierAEnvoyer>()
    private var mamEnAttente: (() -> Unit)? = null

    /** Geste « Tchin » : deux téléphones qu'on cogne doucement l'un contre l'autre. */
    private var tchinActif = false
    private val ecouteurChoc = object : SensorEventListener {
        private val gravite = FloatArray(3)
        private var pret = false
        private var dernier = 0L

        override fun onSensorChanged(e: SensorEvent) {
            if (!pret) {
                e.values.copyInto(gravite, 0, 0, 3)
                pret = true
                return
            }
            var force = 0f
            for (k in 0..2) {
                gravite[k] = 0.9f * gravite[k] + 0.1f * e.values[k]
                val lineaire = e.values[k] - gravite[k]
                force += lineaire * lineaire
            }
            val t = SystemClock.elapsedRealtime()
            // Un petit choc sec (plus de 1,3 g en un instant), pas plus d'un par seconde et demie.
            if (force > 13f * 13f && t - dernier > 1500) {
                dernier = t
                signaler(JSONObject().put("type", "tchin"))
            }
        }

        override fun onAccuracyChanged(capteur: Sensor?, precision: Int) {}
    }

    @Volatile private var liaison: Liaison? = null
    @Volatile private var adressePc: String? = null
    private val fermeture = Runnable { fermerLiaison() }

    @SuppressLint("SetJavaScriptEnabled")
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        web = WebView(this)
        web.settings.apply {
            javaScriptEnabled = true
            domStorageEnabled = true
            mediaPlaybackRequiresUserGesture = false
            allowFileAccess = false
            allowContentAccess = true
            // L'interface est servie en https (adresse imaginaire), le PC
            // répond en http sur le réseau du téléphone : à autoriser.
            mixedContentMode = WebSettings.MIXED_CONTENT_ALWAYS_ALLOW
        }
        web.webViewClient = ClientWeb()
        web.webChromeClient = ChromeWeb()
        web.addJavascriptInterface(Pont(), "KiosqueApp")
        setContentView(web)
        voix = TextToSpeech(this) { statut ->
            if (statut == TextToSpeech.SUCCESS) {
                val r = voix?.setLanguage(java.util.Locale.FRENCH)
                voixPrete = r != TextToSpeech.LANG_MISSING_DATA && r != TextToSpeech.LANG_NOT_SUPPORTED
                // Constaté à l'essai : au réglage par défaut du téléphone, la
                // voix parlait trop vite. Un peu plus lent, ton normal.
                voix?.setSpeechRate(0.8f)
                voix?.setPitch(1.0f)
            }
        }
        val filtre = IntentFilter().apply {
            addAction(WifiManager.WIFI_STATE_CHANGED_ACTION)
            addAction(BluetoothAdapter.ACTION_STATE_CHANGED)
        }
        if (Build.VERSION.SDK_INT >= 33) registerReceiver(recepteurRadios, filtre, Context.RECEIVER_NOT_EXPORTED)
        else registerReceiver(recepteurRadios, filtre)
        traiter(intent)
        MainAMain.ecouteur = { e -> signaler(e) }
        web.loadUrl("https://$HOTE/index.html")
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        traiter(intent)
    }

    override fun onPause() {
        visible = false
        getSystemService(SensorManager::class.java)?.unregisterListener(ecouteurChoc)
        super.onPause()
    }

    override fun onResume() {
        super.onResume()
        visible = true
        signalerRadios()
        if (autorisationsManquantes().isEmpty()) Balayage.demarrer(this)
        if (tchinActif) activerTchin(true)
    }

    override fun onDestroy() {
        voix?.shutdown()
        voix = null
        try { unregisterReceiver(recepteurRadios) } catch (_: Exception) {}
        enregistreur?.let { try { it.release() } catch (_: Exception) {} }
        enregistreur = null
        vocaux.forEach { it.delete() }
        MainAMain.ecouteur = null
        getSystemService(SensorManager::class.java)?.unregisterListener(ecouteurChoc)
        principal.removeCallbacks(fermeture)
        liaison?.let { l -> executeur.execute { l.fermer() } }
        liaison = null
        executeur.shutdown()
        web.destroy()
        super.onDestroy()
    }

    @Deprecated("API Activity simple, sans bibliothèque")
    override fun onBackPressed() {
        web.evaluateJavascript("window.kiosque && window.kiosque.retour ? window.kiosque.retour() : false") { r ->
            if (r != "true") finish()
        }
    }

    private fun journal(texte: String) {
        Log.i("Envoyeur", texte)
    }

    /** Événement pour l'interface (voir `window.kiosque.evenement`). */
    private fun signaler(evenement: JSONObject) {
        principal.post {
            web.evaluateJavascript("window.kiosque && window.kiosque.evenement($evenement)", null)
        }
    }

    // ───────────────────────────── Pages de l'interface ─────────────────────────────

    private inner class ClientWeb : WebViewClient() {
        override fun shouldInterceptRequest(view: WebView, requete: WebResourceRequest): WebResourceResponse? {
            if (requete.url.host != HOTE) return null
            val chemin = requete.url.path.orEmpty()
            return try {
                when {
                    chemin == "/" || chemin == "/index.html" ->
                        WebResourceResponse("text/html", "utf-8", assets.open("index.html"))
                    chemin.startsWith("/vocal/") -> {
                        val fichier = chemin.removePrefix("/vocal/").toIntOrNull()?.let { vocaux.getOrNull(it) }
                            ?: return WebResourceResponse("text/plain", "utf-8", 404, "Introuvable", null, null)
                        WebResourceResponse("audio/mp4", null, fichier.inputStream())
                    }
                    chemin.startsWith("/partage/") -> {
                        val uri = chemin.removePrefix("/partage/").toIntOrNull()?.let { partages.getOrNull(it) }
                            ?: return WebResourceResponse("text/plain", "utf-8", 404, "Introuvable", null, null)
                        val flux = contentResolver.openInputStream(uri)
                            ?: return WebResourceResponse("text/plain", "utf-8", 404, "Illisible", null, null)
                        WebResourceResponse(contentResolver.getType(uri) ?: "application/octet-stream", null, flux)
                    }
                    else -> WebResourceResponse("text/plain", "utf-8", 404, "Introuvable", null, null)
                }
            } catch (e: Exception) {
                WebResourceResponse("text/plain", "utf-8", 500, "Erreur", null, null)
            }
        }

        // L'interface ne quitte jamais son écran : un lien vers ailleurs
        // (WhatsApp, un site de l'espace « Découvrir », un appel) s'ouvre
        // dans l'application qui convient.
        override fun shouldOverrideUrlLoading(view: WebView, requete: WebResourceRequest): Boolean {
            if (requete.url.host == HOTE) return false
            val schema = requete.url.scheme.orEmpty()
            if (schema == "https" || schema == "tel") {
                try {
                    startActivity(Intent(Intent.ACTION_VIEW, requete.url).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                } catch (_: ActivityNotFoundException) {
                }
            }
            return true
        }
    }

    private inner class ChromeWeb : WebChromeClient() {
        override fun onShowFileChooser(
            vue: WebView,
            rappel: ValueCallback<Array<Uri>>,
            parametres: FileChooserParams,
        ): Boolean {
            rappelFichiers?.onReceiveValue(null)
            rappelFichiers = rappel
            if (parametres.isCaptureEnabled) prendrePhoto()
            else choisirFichiers(parametres.mode == FileChooserParams.MODE_OPEN_MULTIPLE)
            return true
        }

        // Le micro, pour le message vocal : demandé au moment où il sert.
        override fun onPermissionRequest(demande: PermissionRequest) {
            principal.post {
                if (PermissionRequest.RESOURCE_AUDIO_CAPTURE !in demande.resources) {
                    demande.deny()
                } else if (checkSelfPermission(Manifest.permission.RECORD_AUDIO) == PackageManager.PERMISSION_GRANTED) {
                    demande.grant(arrayOf(PermissionRequest.RESOURCE_AUDIO_CAPTURE))
                } else {
                    demandeMicro?.deny()
                    demandeMicro = demande
                    requestPermissions(arrayOf(Manifest.permission.RECORD_AUDIO), DEMANDE_MICRO)
                }
            }
        }
    }

    // ───────────────────────────── Pont avec l'interface ─────────────────────────────

    private inner class Pont {
        @JavascriptInterface
        fun autorise(): Boolean = autorisationsManquantes().isEmpty()

        @JavascriptInterface
        fun autoriser() {
            principal.post { demanderAutorisations() }
        }

        /** Crée le réseau et attend le PC ; le résultat arrive en événement `connexion`. */
        @JavascriptInterface
        fun demarrer() {
            principal.post { ouvrirLiaison() }
        }

        /** Le client est encore là (il choisit, ou un envoi avance) : garder le réseau. */
        @JavascriptInterface
        fun garderLien() {
            principal.post {
                if (adressePc == null) return@post
                principal.removeCallbacks(fermeture)
                principal.postDelayed(fermeture, INACTIVITE_MAX_MS)
            }
        }

        /** Envoi reçu par le PC : on le libère peu après pour le client suivant. */
        @JavascriptInterface
        fun envoiTermine(numero: String) {
            journal("🎉 Commande $numero reçue par le PC.")
            principal.post {
                principal.removeCallbacks(fermeture)
                principal.postDelayed(fermeture, APRES_ENVOI_MS)
            }
        }

        /**
         * Commence un message vocal. Faux si le micro n'est pas encore
         * autorisé : la demande s'affiche, le client touche de nouveau.
         */
        @JavascriptInterface
        fun vocalDemarrer(): Boolean {
            if (checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) {
                principal.post { requestPermissions(arrayOf(Manifest.permission.RECORD_AUDIO), DEMANDE_MICRO) }
                return false
            }
            return synchronized(vocaux) {
                enregistreur?.let { try { it.release() } catch (_: Exception) {} }
                val fichier = java.io.File(cacheDir, "vocal_${System.currentTimeMillis()}.m4a")
                val r = nouvelEnregistreur()
                try {
                    r.setAudioSource(MediaRecorder.AudioSource.MIC)
                    r.setOutputFormat(MediaRecorder.OutputFormat.MPEG_4)
                    r.setAudioEncoder(MediaRecorder.AudioEncoder.AAC)
                    r.setAudioSamplingRate(44100)
                    r.setAudioEncodingBitRate(64000)
                    r.setOutputFile(fichier.absolutePath)
                    r.prepare()
                    r.start()
                    enregistreur = r
                    vocaux.add(fichier)
                    true
                } catch (e: Exception) {
                    journal("❌ Micro : ${e.message}")
                    try { r.release() } catch (_: Exception) {}
                    enregistreur = null
                    false
                }
            }
        }

        /** Arrête le message vocal ; rend son adresse pour l'interface, ou "" en cas d'échec. */
        @JavascriptInterface
        fun vocalArreter(): String = synchronized(vocaux) {
            val r = enregistreur ?: return ""
            enregistreur = null
            try {
                r.stop()
                "/vocal/${vocaux.size - 1}"
            } catch (e: Exception) {
                journal("❌ Micro : ${e.message}")
                ""
            } finally {
                try { r.release() } catch (_: Exception) {}
            }
        }

        /** Wi-Fi et Bluetooth allumés ? (`bluetooth` : null si le téléphone n'en a pas.) */
        @JavascriptInterface
        fun radios(): String = etatRadios().toString()

        /**
         * Android ne laisse plus une application allumer le Wi-Fi elle-même :
         * on ouvre son panneau, le client n'a qu'un bouton à toucher.
         */
        @JavascriptInterface
        fun allumerWifi() {
            principal.post {
                try {
                    startActivityForResult(Intent(Settings.Panel.ACTION_WIFI), DEMANDE_REGLAGE_WIFI)
                } catch (_: ActivityNotFoundException) {
                    try { startActivityForResult(Intent(Settings.ACTION_WIFI_SETTINGS), DEMANDE_REGLAGE_WIFI) } catch (_: Exception) {}
                }
            }
        }

        /** Android demande « Autoriser l'activation du Bluetooth ? » : un seul geste. */
        @JavascriptInterface
        fun allumerBluetooth() {
            principal.post {
                if (Build.VERSION.SDK_INT >= 31 &&
                    checkSelfPermission(Manifest.permission.BLUETOOTH_CONNECT) != PackageManager.PERMISSION_GRANTED
                ) {
                    requestPermissions(arrayOf(Manifest.permission.BLUETOOTH_CONNECT), DEMANDE_BLUETOOTH)
                } else {
                    demanderBluetooth()
                }
            }
        }

        /**
         * Dit une phrase avec la voix française du téléphone. Rien en mode
         * silencieux ou vibreur : le client a choisi le silence.
         */
        @JavascriptInterface
        fun parler(texte: String) {
            val son = getSystemService(AudioManager::class.java)
            if (son?.ringerMode != AudioManager.RINGER_MODE_NORMAL) return
            if (!voixPrete) return
            voix?.speak(texte.take(300), TextToSpeech.QUEUE_FLUSH, null, "kiosque")
        }

        /** Documents reçus par « Partager » depuis la dernière demande. */
        @JavascriptInterface
        fun partages(): String {
            val liste = JSONArray()
            synchronized(partages) {
                for (i in partagesLus until partages.size) {
                    val uri = partages[i]
                    liste.put(
                        JSONObject()
                            .put("nom", nomFichier(uri))
                            .put("type", contentResolver.getType(uri) ?: "")
                            .put("url", "/partage/$i")
                    )
                }
                partagesLus = partages.size
            }
            return liste.toString()
        }

        // ─── Main à main ───

        /** Ce que ce téléphone sait faire (Bluetooth, balise, Wi-Fi Direct, 5 GHz). */
        @JavascriptInterface
        fun mamCapacites(): String = MainAMain.capacites(this@MainActivity).toString()

        /** Choisir des fichiers de tout genre (vidéos, .apk, musique…) ; la liste revient en « mam-choix ». */
        @JavascriptInterface
        fun mamChoisir() {
            principal.post { choisirPourMam() }
        }

        @JavascriptInterface
        fun mamRetirer(i: Int) {
            synchronized(mamFichiers) { if (i in mamFichiers.indices) mamFichiers.removeAt(i) }
            signalerChoixMam()
        }

        @JavascriptInterface
        fun mamEnvoyer(groupe: Boolean, prenom: String) {
            principal.post {
                lancerMam {
                    val liste = synchronized(mamFichiers) { mamFichiers.toList() }
                    MainAMain.envoyer(this@MainActivity, liste, groupe, prenom.take(40))
                }
            }
        }

        @JavascriptInterface
        fun mamRecevoir(prenom: String) {
            principal.post { lancerMam { MainAMain.recevoir(this@MainActivity, prenom.take(40)) } }
        }

        @JavascriptInterface
        fun mamArreter() = MainAMain.arreter()

        /** Ouvre un fichier reçu (ou l'installe, pour un .apk : Android demande confirmation). */
        @JavascriptInterface
        fun mamOuvrir(n: Int) {
            principal.post { ouvrirRecu(n) }
        }

        @JavascriptInterface
        fun tchin(actif: Boolean) {
            principal.post {
                tchinActif = actif
                activerTchin(actif)
            }
        }

        /** Partage le fichier d'installation de l'application (Quick Share, Bluetooth…). */
        @JavascriptInterface
        fun donnerApplication() {
            principal.post { partagerApk() }
        }

        /**
         * Télécharge un fichier du PC du kiosque (renvoyé par la boutique ou
         * déposé pour ce client) dans Téléchargements/Kiosque. Seulement
         * depuis le PC relié. L'avancée arrive en événements « telechargement ».
         */
        @JavascriptInterface
        fun telecharger(url: String, nom: String, cle: String): Boolean {
            val adresse = try { Uri.parse(url) } catch (_: Exception) { return false }
            if (adresse.scheme != "http" || adresse.host == null || adresse.host != adressePc) return false
            Thread({ telechargerDuPc(url, nom, cle) }, "telechargement").start()
            return true
        }
    }

    // ───────────────────────────── Main à main ─────────────────────────────

    private fun autorisationsMam(): List<String> = buildList {
        addAll(autorisationsNecessaires())
        if (Build.VERSION.SDK_INT >= 31) {
            add(Manifest.permission.BLUETOOTH_ADVERTISE)
            add(Manifest.permission.BLUETOOTH_CONNECT)
        }
    }.filter { checkSelfPermission(it) != PackageManager.PERMISSION_GRANTED }

    /**
     * Avant « Envoyer » ou « Recevoir » : autorisations, Wi-Fi et Bluetooth
     * allumés, et le lien avec le PC du kiosque refermé (un téléphone ne
     * tient qu'un réseau direct à la fois).
     */
    private fun lancerMam(action: () -> Unit) {
        if (MainAMain.enCours()) return
        val manquantes = autorisationsMam()
        if (manquantes.isNotEmpty()) {
            mamEnAttente = action
            requestPermissions(manquantes.toTypedArray(), DEMANDE_MAM)
            return
        }
        if (!wifiAllume()) {
            signaler(JSONObject().put("type", "mam").put("etat", "wifi"))
            return
        }
        if (bluetoothAllume() != true) {
            signaler(JSONObject().put("type", "mam").put("etat", "bluetooth"))
            return
        }
        fermerLiaison()
        // Même file que la fermeture du lien kiosque : elle est finie avant.
        executeur.execute { principal.post { action() } }
    }

    private fun choisirPourMam() {
        val choix = Intent(Intent.ACTION_OPEN_DOCUMENT)
            .addCategory(Intent.CATEGORY_OPENABLE)
            .setType("*/*")
            .putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true)
        try {
            @Suppress("DEPRECATION")
            startActivityForResult(choix, DEMANDE_MAM_FICHIERS)
        } catch (_: ActivityNotFoundException) {
        }
    }

    private fun signalerChoixMam() {
        val liste = JSONArray()
        synchronized(mamFichiers) {
            mamFichiers.forEach { f ->
                liste.put(JSONObject().put("nom", f.nom).put("taille", f.taille).put("type", f.type))
            }
        }
        signaler(JSONObject().put("type", "mam-choix").put("fichiers", liste))
    }

    private fun ouvrirRecu(n: Int) {
        val (uri, type) = synchronized(MainAMain.recus) { MainAMain.recus.getOrNull(n) } ?: return
        if (type == FournisseurApk.TYPE && !packageManager.canRequestPackageInstalls()) {
            // Une seule fois : autoriser cette application à installer ce qu'on lui envoie.
            try {
                startActivity(Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES, Uri.parse("package:$packageName")))
            } catch (_: Exception) {
            }
            return
        }
        val voir = Intent(Intent.ACTION_VIEW).setDataAndType(uri, type).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        try {
            startActivity(voir)
        } catch (_: ActivityNotFoundException) {
            signaler(JSONObject().put("type", "mam").put("etat", "ouverture")
                .put("message", "Aucune application de ce téléphone ne sait ouvrir ce fichier. Il est dans Téléchargements."))
        }
    }

    private fun activerTchin(actif: Boolean) {
        val capteurs = getSystemService(SensorManager::class.java) ?: return
        capteurs.unregisterListener(ecouteurChoc)
        if (actif) {
            capteurs.getDefaultSensor(Sensor.TYPE_ACCELEROMETER)?.let {
                capteurs.registerListener(ecouteurChoc, it, SensorManager.SENSOR_DELAY_GAME)
            }
        }
    }

    private fun partagerApk() {
        val envoi = Intent(Intent.ACTION_SEND)
            .setType(FournisseurApk.TYPE)
            .putExtra(Intent.EXTRA_STREAM, FournisseurApk.ADRESSE)
            .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        envoi.clipData = ClipData.newRawUri(FournisseurApk.NOM, FournisseurApk.ADRESSE)
        try {
            startActivity(Intent.createChooser(envoi, "Donner l'application").addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION))
        } catch (_: ActivityNotFoundException) {
        }
    }

    /** Fichier du PC → Téléchargements/Kiosque, par morceaux, avec l'avancée. */
    private fun telechargerDuPc(url: String, nom: String, cle: String) {
        fun dire(etat: String, remplir: JSONObject.() -> Unit = {}) =
            signaler(JSONObject().put("type", "telechargement").put("cle", cle).put("etat", etat).apply(remplir))
        var cible: Uri? = null
        try {
            val lien = java.net.URL(url).openConnection() as java.net.HttpURLConnection
            lien.connectTimeout = 8000
            lien.readTimeout = 60_000
            if (lien.responseCode != 200) throw java.io.IOException("Le PC répond ${lien.responseCode}.")
            val total = lien.contentLengthLong
            val type = lien.contentType?.substringBefore(';')?.trim() ?: "application/octet-stream"
            val valeurs = ContentValues().apply {
                put(MediaStore.Downloads.DISPLAY_NAME, nom.replace('/', '_').take(150).ifBlank { "fichier" })
                put(MediaStore.Downloads.MIME_TYPE, type)
                put(MediaStore.Downloads.RELATIVE_PATH, android.os.Environment.DIRECTORY_DOWNLOADS + "/Kiosque")
                put(MediaStore.Downloads.IS_PENDING, 1)
            }
            val destination = contentResolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, valeurs)
                ?: throw java.io.IOException("Impossible d'écrire dans Téléchargements.")
            cible = destination
            var fait = 0L
            var dernier = 0L
            lien.inputStream.use { entree ->
                contentResolver.openOutputStream(destination)!!.use { sortie ->
                    val tampon = ByteArray(256 * 1024)
                    while (true) {
                        val n = entree.read(tampon)
                        if (n < 0) break
                        sortie.write(tampon, 0, n)
                        fait += n
                        val t = SystemClock.elapsedRealtime()
                        if (t - dernier > 400) {
                            dernier = t
                            dire("encours") { put("fait", fait); put("total", total) }
                        }
                    }
                }
            }
            contentResolver.update(destination, ContentValues().apply { put(MediaStore.Downloads.IS_PENDING, 0) }, null, null)
            val n = synchronized(MainAMain.recus) { MainAMain.recus.add(destination to type); MainAMain.recus.size - 1 }
            dire("fini") { put("n", n); put("fait", fait) }
        } catch (e: Exception) {
            cible?.let { try { contentResolver.delete(it, null, null) } catch (_: Exception) {} }
            dire("echec") { put("message", e.message ?: "Téléchargement interrompu.") }
        }
    }

    @Suppress("DEPRECATION")
    private fun nouvelEnregistreur(): MediaRecorder =
        if (Build.VERSION.SDK_INT >= 31) MediaRecorder(this) else MediaRecorder()

    // ───────────────────────────── Liaison avec le PC ─────────────────────────────

    private fun ouvrirLiaison() {
        principal.removeCallbacks(fermeture)
        // Un envoi « Main à main » occupe le Wi-Fi Direct : le guichet attendra.
        if (MainAMain.enCours()) {
            signaler(JSONObject().put("type", "connexion").put("etat", "aucune"))
            return
        }
        adressePc?.let { pc ->
            signaler(JSONObject().put("type", "connexion").put("etat", "ok").put("pc", pc))
            principal.postDelayed(fermeture, INACTIVITE_MAX_MS)
            return
        }
        if (liaison != null) return // déjà en cours
        if (autorisationsManquantes().isNotEmpty()) {
            signaler(JSONObject().put("type", "connexion").put("etat", "echec")
                .put("message", "Autorisations manquantes."))
            return
        }
        // Wi-Fi éteint : l'interface le dit au client et lui propose de
        // l'allumer ; la connexion repart d'elle-même une fois allumé.
        if (!wifiAllume()) {
            signaler(JSONObject().put("type", "connexion").put("etat", "wifi"))
            signalerRadios()
            return
        }
        val l = Liaison(this, ::journal)
        liaison = l
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        signaler(JSONObject().put("type", "connexion").put("etat", "encours"))
        executeur.execute {
            val pc = try { l.ouvrir() } catch (e: Exception) { journal("❌ ${e.message}"); null }
            principal.post {
                if (liaison !== l) return@post
                if (pc != null) {
                    adressePc = pc.hostAddress
                    signaler(JSONObject().put("type", "connexion").put("etat", "ok").put("pc", pc.hostAddress))
                    principal.postDelayed(fermeture, INACTIVITE_MAX_MS)
                } else {
                    liaison = null
                    window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
                    signaler(JSONObject().put("type", "connexion").put("etat", "echec")
                        .put("message", l.raison ?: "Guichet introuvable."))
                }
            }
        }
    }

    private fun wifiAllume(): Boolean =
        applicationContext.getSystemService(WifiManager::class.java)?.isWifiEnabled ?: true

    private fun bluetoothAllume(): Boolean? =
        getSystemService(BluetoothManager::class.java)?.adapter?.isEnabled

    private fun etatRadios(): JSONObject =
        JSONObject().put("type", "radios").put("wifi", wifiAllume())
            .put("bluetooth", bluetoothAllume() ?: JSONObject.NULL)

    private fun signalerRadios() = signaler(etatRadios())

    private fun demanderBluetooth() {
        try {
            @Suppress("DEPRECATION")
            startActivityForResult(Intent(BluetoothAdapter.ACTION_REQUEST_ENABLE), DEMANDE_ACTIVER_BLUETOOTH)
        } catch (_: Exception) {
            ouvrirReglagesBluetooth()
        }
    }

    private fun ouvrirReglagesBluetooth() {
        try {
            @Suppress("DEPRECATION")
            startActivityForResult(Intent(Settings.ACTION_BLUETOOTH_SETTINGS), DEMANDE_REGLAGE_BLUETOOTH)
        } catch (_: Exception) {
        }
    }

    private fun fermerLiaison() {
        val l = liaison ?: return
        liaison = null
        adressePc = null
        window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        signaler(JSONObject().put("type", "connexion").put("etat", "aucune"))
        executeur.execute { l.fermer() }
    }

    // ───────────────────────────── Autorisations ─────────────────────────────

    private fun autorisationsNecessaires(): List<String> = buildList {
        if (Build.VERSION.SDK_INT >= 33) {
            add(Manifest.permission.NEARBY_WIFI_DEVICES)
            add(Manifest.permission.POST_NOTIFICATIONS)
        } else {
            add(Manifest.permission.ACCESS_FINE_LOCATION)
        }
        if (Build.VERSION.SDK_INT >= 31) add(Manifest.permission.BLUETOOTH_SCAN)
    }

    private fun autorisationsManquantes() =
        autorisationsNecessaires().filter { checkSelfPermission(it) != PackageManager.PERMISSION_GRANTED }

    private fun demanderAutorisations() {
        val manquantes = autorisationsManquantes()
        if (manquantes.isEmpty()) {
            Balayage.demarrer(this)
            signaler(JSONObject().put("type", "autorisations").put("ok", true))
        } else {
            requestPermissions(manquantes.toTypedArray(), DEMANDE_AUTORISATIONS)
        }
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        when (requestCode) {
            DEMANDE_AUTORISATIONS -> {
                val refusees = autorisationsManquantes()
                if (refusees.isEmpty()) Balayage.demarrer(this)
                signaler(
                    JSONObject().put("type", "autorisations").put("ok", refusees.isEmpty())
                        .put("message", if (refusees.isEmpty()) "" else
                            "Autorisation refusée. Sans elle, l'application ne peut pas joindre la boutique. " +
                                "Touchez « Autoriser » à nouveau, ou ouvrez les réglages de l'application.")
                )
            }
            DEMANDE_MAM -> {
                val action = mamEnAttente
                mamEnAttente = null
                if (autorisationsMam().isEmpty()) {
                    action?.let { lancerMam(it) }
                } else {
                    signaler(JSONObject().put("type", "mam").put("etat", "erreur")
                        .put("message", "Sans l'autorisation « Appareils à proximité », les deux téléphones ne peuvent pas se trouver."))
                }
            }
            DEMANDE_BLUETOOTH -> {
                if (grantResults.firstOrNull() == PackageManager.PERMISSION_GRANTED) demanderBluetooth()
                else ouvrirReglagesBluetooth()
            }
            DEMANDE_MICRO -> {
                // Demande venue du bouton vocal de l'application : le client touche de nouveau.
                val demande = demandeMicro ?: return
                demandeMicro = null
                if (grantResults.firstOrNull() == PackageManager.PERMISSION_GRANTED) {
                    demande.grant(arrayOf(PermissionRequest.RESOURCE_AUDIO_CAPTURE))
                } else {
                    demande.deny()
                }
            }
        }
    }

    // ───────────────────────────── Documents ─────────────────────────────

    private fun traiter(intent: Intent?) {
        // Notification « Kiosque à côté » : nouvelle visite, nouvelle commande.
        if (intent?.action == ACTION_CHOISIR) signaler(JSONObject().put("type", "nouvelle-visite"))
        val uris: List<Uri> = when (intent?.action) {
            Intent.ACTION_SEND -> listOfNotNull(documentPartage(intent))
            Intent.ACTION_SEND_MULTIPLE -> documentsPartages(intent)
            else -> emptyList()
        }.filter { it.scheme == "content" }
        // Seuls les documents confiés par une autre application (content://)
        // sont acceptés : une adresse file:// pourrait désigner les fichiers
        // privés de cette application (messages vocaux, réglages).
        if (uris.isEmpty()) return
        synchronized(partages) { partages.addAll(uris) }
        signaler(JSONObject().put("type", "partages"))
    }

    @Suppress("DEPRECATION")
    private fun documentPartage(intent: Intent): Uri? =
        if (Build.VERSION.SDK_INT >= 33) intent.getParcelableExtra(Intent.EXTRA_STREAM, Uri::class.java)
        else intent.getParcelableExtra<Uri>(Intent.EXTRA_STREAM)

    @Suppress("DEPRECATION")
    private fun documentsPartages(intent: Intent): List<Uri> =
        (if (Build.VERSION.SDK_INT >= 33) intent.getParcelableArrayListExtra(Intent.EXTRA_STREAM, Uri::class.java)
        else intent.getParcelableArrayListExtra<Uri>(Intent.EXTRA_STREAM)).orEmpty()

    private fun nomFichier(uri: Uri): String {
        try {
            contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { c ->
                if (c.moveToFirst()) {
                    val i = c.getColumnIndex(OpenableColumns.DISPLAY_NAME)
                    if (i >= 0) c.getString(i)?.let { if (it.isNotBlank()) return it }
                }
            }
        } catch (_: Exception) {
        }
        return uri.lastPathSegment?.substringAfterLast('/') ?: "document"
    }

    private fun choisirFichiers(plusieurs: Boolean) {
        val choix = Intent(Intent.ACTION_OPEN_DOCUMENT)
            .addCategory(Intent.CATEGORY_OPENABLE)
            .setType("*/*")
            .putExtra(Intent.EXTRA_ALLOW_MULTIPLE, plusieurs)
        try {
            @Suppress("DEPRECATION")
            startActivityForResult(choix, DEMANDE_FICHIERS)
        } catch (_: ActivityNotFoundException) {
            rendreFichiers(null)
        }
    }

    /** Photo d'un document : enregistrée dans Images/Kiosque, puis ajoutée à la commande. */
    private fun prendrePhoto() {
        val valeurs = ContentValues().apply {
            put(MediaStore.Images.Media.DISPLAY_NAME, "Document_${System.currentTimeMillis()}.jpg")
            put(MediaStore.Images.Media.MIME_TYPE, "image/jpeg")
            put(MediaStore.Images.Media.RELATIVE_PATH, "Pictures/Kiosque")
        }
        val cible = try {
            contentResolver.insert(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, valeurs)
        } catch (_: Exception) {
            null
        }
        if (cible == null) {
            choisirFichiers(true)
            return
        }
        photo = cible
        val appareil = Intent(MediaStore.ACTION_IMAGE_CAPTURE)
            .putExtra(MediaStore.EXTRA_OUTPUT, cible)
            .addFlags(Intent.FLAG_GRANT_WRITE_URI_PERMISSION)
        try {
            @Suppress("DEPRECATION")
            startActivityForResult(appareil, DEMANDE_PHOTO)
        } catch (_: ActivityNotFoundException) {
            contentResolver.delete(cible, null, null)
            photo = null
            choisirFichiers(true)
        }
    }

    private fun rendreFichiers(uris: Array<Uri>?) {
        rappelFichiers?.onReceiveValue(uris)
        rappelFichiers = null
    }

    @Deprecated("API Activity simple, sans bibliothèque")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        @Suppress("DEPRECATION")
        super.onActivityResult(requestCode, resultCode, data)
        when (requestCode) {
            DEMANDE_FICHIERS -> {
                if (resultCode != RESULT_OK || data == null) return rendreFichiers(null)
                val uris = buildList {
                    data.clipData?.let { clip -> for (i in 0 until clip.itemCount) add(clip.getItemAt(i).uri) }
                    if (isEmpty()) data.data?.let { add(it) }
                }
                rendreFichiers(uris.toTypedArray().takeIf { it.isNotEmpty() })
            }
            DEMANDE_MAM_FICHIERS -> {
                if (resultCode != RESULT_OK || data == null) return
                val uris = buildList {
                    data.clipData?.let { clip -> for (i in 0 until clip.itemCount) add(clip.getItemAt(i).uri) }
                    if (isEmpty()) data.data?.let { add(it) }
                }
                executeur.execute {
                    val decrits = uris.map { MainAMain.decrire(this, it) }
                    synchronized(mamFichiers) { mamFichiers.addAll(decrits) }
                    signalerChoixMam()
                }
            }
            DEMANDE_PHOTO -> {
                val cible = photo
                photo = null
                if (resultCode == RESULT_OK && cible != null) {
                    rendreFichiers(arrayOf(cible))
                } else {
                    cible?.let { try { contentResolver.delete(it, null, null) } catch (_: Exception) {} }
                    rendreFichiers(null)
                }
            }
        }
    }
}
