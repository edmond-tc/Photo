package bj.photocopie.envoyeur

import android.Manifest
import android.app.Activity
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Typeface
import android.net.Uri
import android.net.wifi.WifiManager
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import android.text.InputType
import android.view.WindowManager
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/**
 * Un seul écran, construit sans fichier de mise en page : autoriser une
 * fois, choisir ou recevoir (« Partager ») des documents, suivre l'envoi.
 */
class MainActivity : Activity() {
    companion object {
        const val ACTION_CHOISIR = "bj.photocopie.envoyeur.CHOISIR"
        private const val DEMANDE_FICHIERS = 10
        private const val DEMANDE_AUTORISATIONS = 11
    }

    private lateinit var journal: TextView
    private lateinit var etatEcoute: TextView
    private lateinit var boutonChoisir: Button
    @Volatile private var envoiEnCours = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        construireEcran()
        traiter(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        traiter(intent)
    }

    override fun onResume() {
        super.onResume()
        rafraichirEcoute()
    }

    // ───────────────────────────── Écran ─────────────────────────────

    private fun dp(v: Int) = (v * resources.displayMetrics.density).toInt()

    private fun construireEcran() {
        val colonne = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(20), dp(24), dp(20), dp(24))
        }
        colonne.addView(TextView(this).apply {
            text = "Envoyeur Kiosque"
            textSize = 24f
            setTypeface(typeface, Typeface.BOLD)
        })
        colonne.addView(TextView(this).apply {
            text = "Envoie vos documents au PC du kiosque, sans internet et sans forfait. " +
                "Près du guichet, une notification vous le propose toute seule."
            textSize = 15f
            setPadding(0, dp(8), 0, dp(16))
        })

        boutonChoisir = Button(this).apply {
            text = "Choisir des documents à envoyer"
            textSize = 18f
            setOnClickListener { choisirFichiers() }
        }
        colonne.addView(boutonChoisir)

        colonne.addView(Button(this).apply {
            text = "Autoriser (une seule fois)"
            setOnClickListener { demanderAutorisations() }
        })

        etatEcoute = TextView(this).apply {
            textSize = 13f
            setPadding(0, dp(8), 0, dp(8))
        }
        colonne.addView(etatEcoute)

        // Réglages d'essai : le mot de passe doit être celui du PC ; le seuil
        // sépare « au guichet » de « cage voisine ».
        colonne.addView(TextView(this).apply {
            text = "Essai — mot de passe du kiosque :"
            textSize = 13f
            setPadding(0, dp(12), 0, 0)
        })
        val champMdp = EditText(this).apply {
            setText(Reglages.motDePasse(this@MainActivity))
            inputType = InputType.TYPE_CLASS_TEXT
            isSingleLine = true
        }
        colonne.addView(champMdp)
        colonne.addView(TextView(this).apply {
            text = "Essai — signal minimal de la balise (dBm, ex. -75 ; plus près de 0 = plus près du guichet) :"
            textSize = 13f
        })
        val champSeuil = EditText(this).apply {
            setText(Reglages.seuilBle(this@MainActivity).toString())
            inputType = InputType.TYPE_CLASS_NUMBER or InputType.TYPE_NUMBER_FLAG_SIGNED
            isSingleLine = true
        }
        colonne.addView(champSeuil)
        colonne.addView(Button(this).apply {
            text = "Enregistrer les réglages d'essai"
            setOnClickListener {
                val mdp = champMdp.text.toString().trim()
                if (mdp.length >= 8) Reglages.definirMotDePasse(this@MainActivity, mdp)
                champSeuil.text.toString().toIntOrNull()?.let { Reglages.definirSeuilBle(this@MainActivity, it) }
                Reglages.noterNotification(this@MainActivity, 0L)
                dire("Réglages enregistrés.")
            }
        })

        colonne.addView(TextView(this).apply {
            text = "Journal"
            textSize = 16f
            setTypeface(typeface, Typeface.BOLD)
            setPadding(0, dp(16), 0, dp(4))
        })
        journal = TextView(this).apply {
            textSize = 13f
            typeface = Typeface.MONOSPACE
        }
        colonne.addView(journal)

        setContentView(ScrollView(this).apply { addView(colonne) })
    }

    private fun rafraichirEcoute() {
        val etat = Balayage.demarrer(this)
        val signal = Reglages.dernierSignal(this)?.let { (rssi, quand) ->
            "\nDernière balise entendue : $rssi dBm à " +
                SimpleDateFormat("HH:mm:ss", Locale.FRANCE).format(Date(quand))
        }.orEmpty()
        etatEcoute.text = etat + signal
    }

    private fun dire(texte: String) {
        val heure = SimpleDateFormat("HH:mm:ss", Locale.FRANCE).format(Date())
        runOnUiThread { journal.text = "$heure  $texte\n" + journal.text }
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
            dire("Tout est déjà autorisé.")
            rafraichirEcoute()
        } else {
            requestPermissions(manquantes.toTypedArray(), DEMANDE_AUTORISATIONS)
        }
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<out String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        val refusees = autorisationsManquantes()
        dire(if (refusees.isEmpty()) "✅ Autorisations accordées." else "⚠️ Refusées : ${refusees.joinToString { it.substringAfterLast('.') }}")
        rafraichirEcoute()
    }

    // ───────────────────────────── Documents ─────────────────────────────

    private fun traiter(intent: Intent?) {
        when (intent?.action) {
            Intent.ACTION_SEND -> {
                val uri: Uri? = if (Build.VERSION.SDK_INT >= 33) {
                    intent.getParcelableExtra(Intent.EXTRA_STREAM, Uri::class.java)
                } else {
                    @Suppress("DEPRECATION")
                    intent.getParcelableExtra(Intent.EXTRA_STREAM)
                }
                uri?.let { envoyer(listOf(it)) } ?: dire("Rien à envoyer dans ce partage.")
            }
            Intent.ACTION_SEND_MULTIPLE -> {
                val uris: List<Uri> = if (Build.VERSION.SDK_INT >= 33) {
                    intent.getParcelableArrayListExtra(Intent.EXTRA_STREAM, Uri::class.java)
                } else {
                    @Suppress("DEPRECATION")
                    intent.getParcelableArrayListExtra(Intent.EXTRA_STREAM)
                }.orEmpty()
                if (uris.isNotEmpty()) envoyer(uris) else dire("Rien à envoyer dans ce partage.")
            }
            ACTION_CHOISIR -> choisirFichiers()
        }
    }

    private fun choisirFichiers() {
        if (envoiEnCours) return
        val choix = Intent(Intent.ACTION_OPEN_DOCUMENT)
            .addCategory(Intent.CATEGORY_OPENABLE)
            .setType("*/*")
            .putExtra(Intent.EXTRA_ALLOW_MULTIPLE, true)
        @Suppress("DEPRECATION")
        startActivityForResult(choix, DEMANDE_FICHIERS)
    }

    @Deprecated("API Activity simple, sans bibliothèque")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        @Suppress("DEPRECATION")
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode != DEMANDE_FICHIERS || resultCode != RESULT_OK || data == null) return
        val uris = buildList {
            data.clipData?.let { clip -> for (i in 0 until clip.itemCount) add(clip.getItemAt(i).uri) }
            if (isEmpty()) data.data?.let { add(it) }
        }
        if (uris.isNotEmpty()) envoyer(uris)
    }

    private fun envoyer(uris: List<Uri>) {
        if (envoiEnCours) {
            dire("Un envoi est déjà en cours.")
            return
        }
        val manquantes = autorisationsManquantes()
        if (manquantes.isNotEmpty()) {
            dire("Touchez d'abord « Autoriser (une seule fois) ».")
            demanderAutorisations()
            return
        }
        val wifi = applicationContext.getSystemService(WifiManager::class.java)
        if (wifi?.isWifiEnabled == false) startActivity(Intent(Settings.Panel.ACTION_WIFI))

        envoiEnCours = true
        boutonChoisir.isEnabled = false
        window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        dire("${uris.size} document(s) à envoyer.")
        Thread {
            try {
                Envoi(this, uris, ::dire).lancer()
            } catch (e: Exception) {
                dire("❌ ${e.message}")
            } finally {
                envoiEnCours = false
                runOnUiThread {
                    boutonChoisir.isEnabled = true
                    window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
                }
            }
        }.start()
    }
}
