package bj.photocopie.envoyeur

import android.annotation.SuppressLint
import android.app.Activity
import android.content.ComponentName
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.webkit.JavascriptInterface
import android.webkit.ValueCallback
import android.webkit.WebChromeClient
import android.webkit.WebResourceError
import android.webkit.WebResourceRequest
import android.webkit.WebView
import android.webkit.WebViewClient

/**
 * Écran « Portail Amphi » : démarre le serveur du téléphone et ouvre la page
 * de gestion (programme, annonces, documents, publicités, QR à montrer).
 */
class AmphiActivity : Activity() {
    companion object {
        private const val DEMANDE_FICHIERS = 7302
        private const val ADRESSE = "http://127.0.0.1:${AmphiService.PORT}/gerer"
    }

    private lateinit var vue: WebView
    private var rappelFichiers: ValueCallback<Array<Uri>>? = null
    private val principal = Handler(Looper.getMainLooper())
    private var essais = 0

    @SuppressLint("SetJavaScriptEnabled")
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        AmphiService.demarrer(this)
        vue = WebView(this)
        setContentView(vue)
        vue.settings.javaScriptEnabled = true
        vue.settings.domStorageEnabled = true
        vue.addJavascriptInterface(Pont(), "Amphi")
        vue.webViewClient = object : WebViewClient() {
            override fun onReceivedError(v: WebView, demande: WebResourceRequest, erreur: WebResourceError) {
                // Le serveur démarre en quelques dixièmes de seconde : on réessaie.
                if (demande.isForMainFrame && essais++ < 40) principal.postDelayed({ vue.loadUrl(ADRESSE) }, 250)
            }

            override fun shouldOverrideUrlLoading(v: WebView, demande: WebResourceRequest): Boolean {
                val url = demande.url
                // Aperçu du portail et liens externes (WhatsApp…) : dans le vrai navigateur.
                if (url.host == "127.0.0.1" && url.path?.startsWith("/gerer") == true) return false
                try { startActivity(Intent(Intent.ACTION_VIEW, url)) } catch (_: Exception) {}
                return true
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
                } catch (_: Exception) {
                    rappelFichiers = null
                    false
                }
            }
        }
        principal.postDelayed({ vue.loadUrl(ADRESSE) }, 300)
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

    private inner class Pont {
        @JavascriptInterface
        fun enMarche(): Boolean = AmphiService.enMarche

        @JavascriptInterface
        fun demarrer() = AmphiService.demarrer(this@AmphiActivity)

        @JavascriptInterface
        fun arreter() = AmphiService.arreter(this@AmphiActivity)

        /** Réglages du point d'accès mobile (Samsung et autres), sinon réglages réseau. */
        @JavascriptInterface
        fun ouvrirPointAcces() {
            principal.post {
                val essais = listOf(
                    Intent().setComponent(ComponentName("com.android.settings", "com.android.settings.TetherSettings")),
                    Intent().setClassName("com.android.settings", "com.android.settings.Settings\$TetherSettingsActivity"),
                    Intent(Settings.ACTION_WIRELESS_SETTINGS),
                    Intent(Settings.ACTION_SETTINGS),
                )
                for (i in essais) {
                    try { startActivity(i.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)); return@post } catch (_: Exception) {}
                }
            }
        }

        @JavascriptInterface
        fun partager(texte: String) {
            principal.post {
                try {
                    startActivity(Intent.createChooser(Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, texte), "Partager"))
                } catch (_: Exception) {}
            }
        }
    }
}
