package bj.photocopie.amphi

import android.Manifest
import android.annotation.SuppressLint
import android.app.Activity
import android.content.ContentValues
import android.content.Intent
import android.content.pm.PackageManager
import android.location.LocationManager
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.net.Uri
import android.net.wifi.WifiNetworkSpecifier
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.PatternMatcher
import android.provider.MediaStore
import android.provider.Settings
import android.util.Base64
import android.webkit.JavascriptInterface
import android.webkit.URLUtil
import android.webkit.ValueCallback
import android.webkit.WebChromeClient
import android.webkit.WebResourceError
import android.webkit.WebResourceRequest
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.Toast
import org.json.JSONObject
import java.io.InputStream
import java.net.HttpURLConnection
import java.net.Inet4Address
import java.net.URL

/**
 * « Amphi » : se connecte au Wi-Fi de l'amphi (réseau « DIRECT-AM-… »)
 * comme un réseau LOCAL, sans remplacer Internet : les données mobiles
 * restent allumées pour WhatsApp, les appels… Seule cette application
 * passe par le Wi-Fi de l'amphi, où elle ouvre le portail.
 */
class MainActivity : Activity() {
    companion object {
        private const val DEMANDE_FICHIERS = 1
        private const val DEMANDE_AUTORISATION = 2
        private const val ACCUEIL = "file:///android_asset/accueil.html"
        private const val PORT = 8080
    }

    private lateinit var vue: WebView
    private val cm by lazy { getSystemService(ConnectivityManager::class.java) }
    private val prefs by lazy { getSharedPreferences("amphi", 0) }
    private val principal = Handler(Looper.getMainLooper())
    private var rappel: ConnectivityManager.NetworkCallback? = null
    @Volatile private var reseau: Network? = null
    private var adresse = ""
    private var mdpEnAttente = ""
    private var rappelFichiers: ValueCallback<Array<Uri>>? = null

    @SuppressLint("SetJavaScriptEnabled")
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        vue = WebView(this)
        setContentView(vue)
        vue.settings.javaScriptEnabled = true
        vue.settings.domStorageEnabled = true
        vue.settings.textZoom = 90
        vue.settings.mediaPlaybackRequiresUserGesture = false
        vue.addJavascriptInterface(Pont(), "AmphiApp")
        vue.webViewClient = object : WebViewClient() {
            override fun shouldOverrideUrlLoading(v: WebView, demande: WebResourceRequest): Boolean {
                val u = demande.url
                if (u.scheme == "http" || u.scheme == "file") return false
                // WhatsApp, appels… : l'application qui convient.
                try { startActivity(Intent(Intent.ACTION_VIEW, u)) } catch (_: Exception) {}
                return true
            }

            override fun onReceivedError(v: WebView, demande: WebResourceRequest, erreur: WebResourceError) {
                if (demande.isForMainFrame && demande.url.scheme == "http") accueil("La page de l'amphi ne répond pas. Vérifiez que vous êtes dans l'amphi, puis reconnectez-vous.")
            }
        }
        vue.webChromeClient = object : WebChromeClient() {
            override fun onShowFileChooser(v: WebView, rappel: ValueCallback<Array<Uri>>, p: FileChooserParams): Boolean {
                rappelFichiers?.onReceiveValue(null)
                rappelFichiers = rappel
                val choix = Intent(Intent.ACTION_GET_CONTENT).addCategory(Intent.CATEGORY_OPENABLE).setType("*/*")
                    .putExtra(Intent.EXTRA_ALLOW_MULTIPLE, p.mode == FileChooserParams.MODE_OPEN_MULTIPLE)
                p.acceptTypes.filter { it.isNotBlank() }.takeIf { it.isNotEmpty() }?.let {
                    if (it.size == 1) choix.type = it[0] else choix.putExtra(Intent.EXTRA_MIME_TYPES, it.toTypedArray())
                }
                return try {
                    @Suppress("DEPRECATION")
                    startActivityForResult(Intent.createChooser(choix, "Choisir"), DEMANDE_FICHIERS)
                    true
                } catch (_: Exception) { rappelFichiers = null; false }
            }
        }
        // Liens de téléchargement classiques : enregistrés dans Téléchargements.
        vue.setDownloadListener { url, _, disposition, type, _ ->
            if (url.startsWith("http")) telecharger(url, URLUtil.guessFileName(url, disposition, type), type ?: "application/octet-stream")
        }
        accueil("")
    }

    private fun accueil(message: String) = principal.post { vue.loadUrl(ACCUEIL + if (message.isNotEmpty()) "#" + Uri.encode(message) else "") }

    private fun js(code: String) = principal.post { vue.evaluateJavascript(code, null) }

    private fun etat(t: String) = js("window.etat && window.etat(" + JSONObject.quote(t) + ")")

    private fun autorisation(): String = if (Build.VERSION.SDK_INT >= 33) Manifest.permission.NEARBY_WIFI_DEVICES else Manifest.permission.ACCESS_FINE_LOCATION

    /** Demande le Wi-Fi « DIRECT-AM-… » comme réseau local : Android montre la liste, un geste suffit. */
    private fun connecter(mdp: String) {
        if (checkSelfPermission(autorisation()) != PackageManager.PERMISSION_GRANTED) {
            mdpEnAttente = mdp
            requestPermissions(arrayOf(autorisation()), DEMANDE_AUTORISATION)
            return
        }
        if (Build.VERSION.SDK_INT < 33 && getSystemService(LocationManager::class.java)?.isLocationEnabled == false) {
            etat("Allumez la localisation (Android en a besoin pour trouver le Wi-Fi), puis touchez à nouveau « Se connecter ».")
            try { startActivity(Intent(Settings.ACTION_LOCATION_SOURCE_SETTINGS)) } catch (_: Exception) {}
            return
        }
        liberer()
        val spec = WifiNetworkSpecifier.Builder()
            .setSsidPattern(PatternMatcher("DIRECT-AM-", PatternMatcher.PATTERN_PREFIX))
            .apply { if (mdp.length >= 8) setWpa2Passphrase(mdp) }
            .build()
        val demande = NetworkRequest.Builder()
            .addTransportType(NetworkCapabilities.TRANSPORT_WIFI)
            .removeCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
            .setNetworkSpecifier(spec)
            .build()
        val r = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(n: Network) { utiliser(n) }
            override fun onUnavailable() { etat("Wi-Fi de l'amphi introuvable. Approchez-vous du téléphone qui le crée, vérifiez le mot de passe écrit au tableau, puis réessayez.") }
            override fun onLost(n: Network) {
                if (n == reseau) { cm.bindProcessToNetwork(null); reseau = null; accueil("Connexion au Wi-Fi de l'amphi perdue.") }
            }
        }
        rappel = r
        etat("Choisissez le Wi-Fi « DIRECT-AM-… » dans la fenêtre qui s'ouvre.")
        try { cm.requestNetwork(demande, r, 60_000) } catch (e: Exception) { rappel = null; etat("Impossible de chercher le Wi-Fi : " + (e.message ?: "erreur")) }
    }

    /** Toute l'application (et la page) passe par ce Wi-Fi ; le reste du téléphone garde Internet. */
    private fun utiliser(n: Network) {
        reseau = n
        cm.bindProcessToNetwork(n)
        val gw = cm.getLinkProperties(n)?.routes?.firstOrNull { it.destination.prefixLength == 0 && it.gateway is Inet4Address }?.gateway?.hostAddress
        adresse = gw ?: "192.168.49.1"
        principal.post { vue.loadUrl("http://$adresse:$PORT/") }
    }

    /** Déjà connecté au Wi-Fi de l'amphi par les réglages du téléphone. */
    private fun dejaConnecte() {
        @Suppress("DEPRECATION")
        val n = cm.allNetworks.firstOrNull { cm.getNetworkCapabilities(it)?.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) == true }
        if (n == null) { etat("Ce téléphone n'est connecté à aucun Wi-Fi. Touchez « Se connecter au Wi-Fi de l'amphi »."); return }
        liberer()
        utiliser(n)
    }

    private fun liberer() {
        rappel?.let { try { cm.unregisterNetworkCallback(it) } catch (_: Exception) {} }
        rappel = null
        cm.bindProcessToNetwork(null)
        reseau = null
    }

    private fun ouvrir(url: String): InputStream {
        val c = (reseau?.openConnection(URL(url)) ?: URL(url).openConnection()) as HttpURLConnection
        c.connectTimeout = 10_000; c.readTimeout = 60_000
        if (c.responseCode != 200) throw Exception("code ${c.responseCode}")
        return c.inputStream
    }

    /** Enregistre dans « Téléchargements » (sans permission de stockage, Android 10+). */
    private fun ecrireTelechargement(nom: String, type: String, ecrire: (java.io.OutputStream) -> Unit) {
        val valeurs = ContentValues().apply {
            put(MediaStore.Downloads.DISPLAY_NAME, nom.replace(Regex("[\\\\/:*?\"<>|]"), "_"))
            put(MediaStore.Downloads.MIME_TYPE, type)
            put(MediaStore.Downloads.IS_PENDING, 1)
        }
        val uri = contentResolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, valeurs) ?: throw Exception("stockage")
        try {
            contentResolver.openOutputStream(uri)!!.use(ecrire)
            contentResolver.update(uri, ContentValues().apply { put(MediaStore.Downloads.IS_PENDING, 0) }, null, null)
        } catch (e: Exception) { contentResolver.delete(uri, null, null); throw e }
    }

    private fun telecharger(url: String, nom: String, type: String) {
        toast("Téléchargement de « $nom »…")
        Thread {
            try {
                ecrireTelechargement(nom, type) { o -> ouvrir(url).use { it.copyTo(o, 64 * 1024) } }
                toast("✅ « $nom » est dans Téléchargements")
            } catch (_: Exception) { toast("❌ Échec du téléchargement de « $nom »") }
        }.start()
    }

    private fun toast(t: String) = principal.post { Toast.makeText(this, t, Toast.LENGTH_LONG).show() }

    private inner class Pont {
        @JavascriptInterface fun motDePasse(): String = prefs.getString("mdp", "amphi2026") ?: "amphi2026"

        @JavascriptInterface fun connecter(mdp: String) {
            prefs.edit().putString("mdp", mdp.trim()).apply()
            principal.post { connecter(mdp.trim()) }
        }

        @JavascriptInterface fun dejaConnecte() { principal.post { dejaConnecte() } }

        @JavascriptInterface fun telecharger(url: String, nom: String, type: String) {
            if (url.startsWith("http")) telecharger(url, nom.ifBlank { "document" }, type.ifBlank { "application/octet-stream" })
        }

        @JavascriptInterface fun enregistrer(base64: String, nom: String, type: String) {
            Thread {
                try {
                    val octets = Base64.decode(base64.substringAfter(","), Base64.DEFAULT)
                    ecrireTelechargement(nom, type) { it.write(octets) }
                    toast("✅ « $nom » est dans Téléchargements")
                } catch (_: Exception) { toast("❌ Échec de l'enregistrement") }
            }.start()
        }

        @JavascriptInterface fun partagerApplication() {
            principal.post {
                val i = Intent(Intent.ACTION_SEND).setType(FournisseurApk.TYPE).putExtra(Intent.EXTRA_STREAM, FournisseurApk.ADRESSE)
                    .putExtra(Intent.EXTRA_TEXT, "L'application Amphi : la page de l'amphi sans couper vos données mobiles.")
                    .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
                try { startActivity(Intent.createChooser(i, "Donner l'application Amphi")) } catch (_: Exception) {}
            }
        }

        @JavascriptInterface fun accueil() = accueil("")

        @JavascriptInterface fun ouvrirWifi() {
            principal.post { try { startActivity(Intent(Settings.Panel.ACTION_WIFI)) } catch (_: Exception) { try { startActivity(Intent(Settings.ACTION_WIFI_SETTINGS)) } catch (_: Exception) {} } }
        }
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode != DEMANDE_AUTORISATION) return
        if (grantResults.isNotEmpty() && grantResults[0] == PackageManager.PERMISSION_GRANTED) connecter(mdpEnAttente)
        else etat("Sans cette autorisation, Android ne peut pas trouver le Wi-Fi de l'amphi.")
    }

    @Deprecated("API Activity")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        @Suppress("DEPRECATION")
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode != DEMANDE_FICHIERS) return
        val uris = if (resultCode != RESULT_OK || data == null) null else buildList {
            data.clipData?.let { c -> for (i in 0 until c.itemCount) add(c.getItemAt(i).uri) }
            if (isEmpty()) data.data?.let { add(it) }
        }.toTypedArray().takeIf { it.isNotEmpty() }
        rappelFichiers?.onReceiveValue(uris)
        rappelFichiers = null
    }

    @Deprecated("API Activity")
    override fun onBackPressed() {
        if (vue.canGoBack()) vue.goBack() else {
            @Suppress("DEPRECATION")
            super.onBackPressed()
        }
    }

    override fun onDestroy() {
        liberer()
        super.onDestroy()
    }
}
