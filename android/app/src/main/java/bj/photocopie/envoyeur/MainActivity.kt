package bj.photocopie.envoyeur

import android.Manifest
import android.annotation.SuppressLint
import android.app.Activity
import android.content.ActivityNotFoundException
import android.content.ContentValues
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.net.wifi.WifiManager
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
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
        private const val DEMANDE_FICHIERS = 10
        private const val DEMANDE_AUTORISATIONS = 11
        private const val DEMANDE_MICRO = 12
        private const val DEMANDE_PHOTO = 13

        /** Adresse imaginaire, servie par l'application elle-même (voir [ClientWeb]). */
        private const val HOTE = "envoyeur.kiosque"

        /** Sans envoi, le réseau est supprimé au bout de ce délai : le PC ne reste pas bloqué. */
        private const val INACTIVITE_MAX_MS = 10 * 60 * 1000L

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
        traiter(intent)
        web.loadUrl("https://$HOTE/index.html")
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        traiter(intent)
    }

    override fun onResume() {
        super.onResume()
        if (autorisationsManquantes().isEmpty()) Balayage.demarrer(this)
    }

    override fun onDestroy() {
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

        // L'interface ne quitte jamais son écran.
        override fun shouldOverrideUrlLoading(view: WebView, requete: WebResourceRequest) =
            requete.url.host != HOTE
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

        /** Envoi reçu par le PC : on le libère peu après pour le client suivant. */
        @JavascriptInterface
        fun envoiTermine(numero: String) {
            journal("🎉 Commande $numero reçue par le PC.")
            principal.post {
                principal.removeCallbacks(fermeture)
                principal.postDelayed(fermeture, APRES_ENVOI_MS)
            }
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
    }

    // ───────────────────────────── Liaison avec le PC ─────────────────────────────

    private fun ouvrirLiaison() {
        principal.removeCallbacks(fermeture)
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
        val wifi = applicationContext.getSystemService(WifiManager::class.java)
        if (wifi?.isWifiEnabled == false) {
            try { startActivity(Intent(Settings.Panel.ACTION_WIFI)) } catch (_: ActivityNotFoundException) {}
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
            DEMANDE_MICRO -> {
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
        val uris: List<Uri> = when (intent?.action) {
            Intent.ACTION_SEND -> listOfNotNull(documentPartage(intent))
            Intent.ACTION_SEND_MULTIPLE -> documentsPartages(intent)
            else -> emptyList()
        }
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
