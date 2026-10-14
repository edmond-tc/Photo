package bj.photocopie.envoyeur

import android.Manifest
import android.annotation.SuppressLint
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.wifi.WifiManager
import android.net.wifi.p2p.WifiP2pConfig
import android.net.wifi.p2p.WifiP2pGroup
import android.net.wifi.p2p.WifiP2pManager
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import android.provider.MediaStore
import org.json.JSONArray
import org.json.JSONObject
import java.io.BufferedInputStream
import java.io.File
import java.io.InputStream
import java.io.OutputStream
import java.net.Inet4Address
import java.net.HttpURLConnection
import java.net.InetSocketAddress
import java.net.NetworkInterface
import java.net.ServerSocket
import java.net.Socket
import java.net.URLDecoder
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference
import java.util.zip.ZipEntry
import java.util.zip.ZipInputStream
import java.util.zip.ZipOutputStream

/**
 * Portail de l'amphi : le téléphone partage sa connexion (point d'accès
 * mobile, données coupées) et sert une page aux étudiants connectés —
 * programme de la semaine, annonces, documents à télécharger, publicités —
 * sans Internet et sans forfait.
 *
 * Tout ce que voient les étudiants est en lecture seule. La gestion (/gerer)
 * ne répond qu'au téléphone lui-même (adresse 127.0.0.1), ouverte dans
 * l'application : un étudiant connecté ne peut rien modifier.
 */
class AmphiService : Service() {
    companion object {
        const val PORT = 8080
        private const val CANAL = "amphi"
        private const val ID = 7301
        private const val CONTENU_MAX = 2_000_000
        private const val FICHIER_MAX = 2_000_000_000L

        private const val PREFS = "amphi"

        @Volatile var enMarche = false
            private set

        fun demarrer(ctx: Context) {
            ctx.startForegroundService(Intent(ctx, AmphiService::class.java))
        }

        /** Arrêt voulu par l'éditeur : le portail ne redémarre plus tout seul. */
        fun arreter(ctx: Context) {
            ctx.getSharedPreferences(PREFS, 0).edit().putBoolean("actif", false).apply()
            ctx.stopService(Intent(ctx, AmphiService::class.java))
        }

        /** Le portail tournait avant l'extinction du téléphone : on le relance. */
        fun doitTourner(ctx: Context) = ctx.getSharedPreferences(PREFS, 0).getBoolean("actif", false)

        /** Adresses IPv4 du téléphone sur ses réseaux locaux (point d'accès compris). */
        fun adresses(): List<String> = try {
            NetworkInterface.getNetworkInterfaces().toList()
                .filter { it.isUp && !it.isLoopback }
                // Le point d'accès d'abord (swlan, ap, wlan), les données mobiles exclues.
                .filterNot { it.name.startsWith("rmnet") || it.name.startsWith("ccmni") || it.name.startsWith("dummy") || it.name.startsWith("tun") }
                // Le Wi-Fi de l'amphi (Wi-Fi Direct, adresse fixe) d'abord, puis le point d'accès.
                .sortedBy { if (it.name.startsWith("p2p")) 0 else if (it.name.startsWith("swlan") || it.name.startsWith("ap")) 1 else 2 }
                .flatMap { i -> i.inetAddresses.toList().filterIsInstance<Inet4Address>().map { it.hostAddress ?: "" } }
                .filter { it.isNotEmpty() && (it.startsWith("192.168.") || it.startsWith("10.") || it.matches(Regex("^172\\.(1[6-9]|2\\d|3[01])\\..*"))) }
                .distinct()
        } catch (_: Exception) {
            emptyList()
        }
    }

    private var serveur: ServerSocket? = null
    private val ouvriers = Executors.newFixedThreadPool(24)
    private var verrou: PowerManager.WakeLock? = null
    private var verrouWifi: WifiManager.WifiLock? = null
    private val dossier by lazy { File(filesDir, "amphi").apply { mkdirs() } }
    private val dossierFichiers by lazy { File(getExternalFilesDir(null) ?: filesDir, "amphi").apply { mkdirs() } }
    private val fichierContenu get() = File(dossier, "contenu.json")
    private val fichierStats get() = File(dossier, "stats.json")
    private val fichierCompteurs get() = File(dossier, "compteurs.json")
    private val visiteursDuJour = HashSet<String>()
    private var jourCourant = ""

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        val nm = getSystemService(NotificationManager::class.java)
        if (nm?.getNotificationChannel(CANAL) == null) {
            nm?.createNotificationChannel(NotificationChannel(CANAL, "Portail Amphi", NotificationManager.IMPORTANCE_LOW))
        }
        val ouvrir = PendingIntent.getActivity(
            this, 0, Intent(this, AmphiActivity::class.java), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
        )
        val n = Notification.Builder(this, CANAL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle("Portail Amphi en marche")
            .setContentText("Les étudiants connectés à votre point d'accès peuvent ouvrir la page.")
            .setContentIntent(ouvrir)
            .setOngoing(true)
            .build()
        // « specialUse » : pas de limite de 6 heures par jour (Android 15), une
        // journée de cours en dure 12.
        if (Build.VERSION.SDK_INT >= 34) startForeground(ID, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE)
        else startForeground(ID, n)

        // Écran éteint, le téléphone doit continuer à répondre.
        verrou = getSystemService(PowerManager::class.java)
            ?.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "photocopie:amphi")?.apply { setReferenceCounted(false); acquire() }
        @Suppress("DEPRECATION")
        verrouWifi = (applicationContext.getSystemService(Context.WIFI_SERVICE) as? WifiManager)
            ?.createWifiLock(WifiManager.WIFI_MODE_FULL_HIGH_PERF, "photocopie:amphi")?.apply { setReferenceCounted(false); acquire() }

        getSharedPreferences(PREFS, 0).edit().putBoolean("actif", true).apply()
        chargerStats()
        Thread({ ecouter() }, "amphi-serveur").start()
        // Le Wi-Fi de l'amphi était allumé (redémarrage, mise à jour) : on le
        // recrée, puis on veille à ce qu'il reste allumé.
        Thread({ veillerWifi() }, "amphi-wifi").start()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int = START_STICKY

    override fun onDestroy() {
        enMarche = false
        veille = false
        supprimerGroupe()
        try { serveur?.close() } catch (_: Exception) {}
        ouvriers.shutdownNow()
        try { verrou?.release() } catch (_: Exception) {}
        try { verrouWifi?.release() } catch (_: Exception) {}
        super.onDestroy()
    }

    private fun ecouter() {
        try {
            val s = ServerSocket()
            s.reuseAddress = true
            s.bind(InetSocketAddress(PORT), 64)
            serveur = s
            enMarche = true
            while (!s.isClosed) {
                val client = try { s.accept() } catch (_: Exception) { break }
                try {
                    ouvriers.execute { traiter(client) }
                } catch (_: Exception) {
                    try { client.close() } catch (_: Exception) {}
                }
            }
        } catch (_: Exception) {
        } finally {
            enMarche = false
        }
    }

    // ── HTTP minimal (une requête par connexion) ───────────────────────────

    private class Requete(val methode: String, val chemin: String, val query: Map<String, String>, val entetes: Map<String, String>, val corps: InputStream)

    private fun lireRequete(entree: InputStream): Requete? {
        val tete = StringBuilder()
        while (tete.length < 16_384) {
            val c = entree.read()
            if (c < 0) return null
            tete.append(c.toChar())
            if (tete.endsWith("\r\n\r\n") || tete.endsWith("\n\n")) break
        }
        val lignes = tete.toString().split("\n").map { it.trimEnd('\r') }
        val premiere = lignes.firstOrNull()?.split(" ") ?: return null
        if (premiere.size < 2) return null
        val cible = premiere[1]
        val chemin = URLDecoder.decode(cible.substringBefore('?'), "UTF-8")
        val query = cible.substringAfter('?', "").split('&').filter { it.contains('=') }.associate {
            URLDecoder.decode(it.substringBefore('='), "UTF-8") to URLDecoder.decode(it.substringAfter('='), "UTF-8")
        }
        val entetes = lignes.drop(1).filter { it.contains(':') }.associate { it.substringBefore(':').trim().lowercase() to it.substringAfter(':').trim() }
        return Requete(premiere[0].uppercase(), chemin, query, entetes, entree)
    }

    private fun repondre(sortie: OutputStream, code: Int, type: String, corps: ByteArray, extra: Map<String, String> = emptyMap(), cache: String = "no-store") {
        val texte = when (code) { 200 -> "OK"; 206 -> "Partial Content"; 403 -> "Forbidden"; 404 -> "Not Found"; 413 -> "Payload Too Large"; 416 -> "Range Not Satisfiable"; 507 -> "Insufficient Storage"; else -> "Error" }
        val tete = StringBuilder("HTTP/1.1 $code $texte\r\nContent-Type: $type\r\nContent-Length: ${corps.size}\r\nConnection: close\r\nCache-Control: $cache\r\n")
        extra.forEach { (k, v) -> tete.append("$k: $v\r\n") }
        tete.append("\r\n")
        sortie.write(tete.toString().toByteArray())
        sortie.write(corps)
        sortie.flush()
    }

    private fun json(sortie: OutputStream, o: Any, code: Int = 200) =
        repondre(sortie, code, "application/json; charset=utf-8", o.toString().toByteArray())

    private fun traiter(socket: Socket) {
        socket.use { s ->
            try {
                s.soTimeout = 60_000
                val entree = BufferedInputStream(s.getInputStream())
                val sortie = s.getOutputStream()
                val r = lireRequete(entree) ?: return
                val local = s.inetAddress.isLoopbackAddress
                when {
                    r.methode == "GET" && (r.chemin == "/" || r.chemin == "/index.html") -> {
                        compterVisite(s.inetAddress.hostAddress ?: "")
                        actif(sortie, "amphi/portail.html", "text/html; charset=utf-8")
                    }
                    r.methode == "GET" && r.chemin == "/donnees.json" -> json(sortie, contenuPublic())
                    // Lecture dans la page (PDF, images, vidéos, sons) : pas de
                    // téléchargement « non sécurisé » signalé par Chrome.
                    r.methode == "GET" && r.chemin == "/lire" -> actif(sortie, "amphi/lire.html", "text/html; charset=utf-8")
                    // UE et crédits officiels : intégrés à l'application, ils ne disparaissent jamais.
                    r.methode == "GET" && r.chemin == "/maquettes.json" -> actif(sortie, "amphi/maquettes.json", "application/json; charset=utf-8")
                    r.methode == "GET" && r.chemin == "/formulaire.json" -> actif(sortie, "amphi/formulaire.json", "application/json; charset=utf-8", "max-age=86400")
                    // Icône « Amphi » sur l'écran d'accueil des étudiants.
                    r.methode == "GET" && r.chemin == "/manifest.json" -> actif(sortie, "amphi/manifest.json", "application/manifest+json; charset=utf-8", "max-age=86400")
                    r.methode == "GET" && (r.chemin == "/icone-192.png" || r.chemin == "/icone-512.png") ->
                        actif(sortie, "amphi" + r.chemin, "image/png", "max-age=604800")
                    r.methode == "GET" && (r.chemin == "/lib/pdf.min.js" || r.chemin == "/lib/pdf.worker.min.js") ->
                        actif(sortie, "amphi" + r.chemin, "text/javascript; charset=utf-8", "max-age=604800")
                    r.methode == "GET" && r.chemin.startsWith("/f/") -> envoyerFichier(sortie, r, r.chemin.removePrefix("/f/"))
                    // L'application elle-même : un étudiant l'installe pour devenir relais.
                    r.methode == "GET" && r.chemin == "/application.apk" ->
                        envoyerChemin(sortie, r, File(applicationInfo.sourceDir), "application/vnd.android.package-archive", "Portail-Amphi.apk", null)
                    // ── Échanges entre étudiants (lecture et dépôt, sans compte) ──
                    r.methode == "GET" && r.chemin == "/echanges.json" -> json(sortie, echangesPublics(s.inetAddress.hostAddress ?: ""))
                    r.methode == "POST" && r.chemin == "/partage" -> deposerPartage(sortie, r, s.inetAddress.hostAddress ?: "")
                    r.methode == "GET" && r.chemin.matches(Regex("^/partage/\\d{4}$")) -> infoPartage(sortie, r.chemin.removePrefix("/partage/"))
                    r.methode == "GET" && r.chemin.matches(Regex("^/partage/\\d{4}/f$")) -> fichierPartage(sortie, r, r.chemin.removePrefix("/partage/").removeSuffix("/f"))
                    r.methode == "POST" && r.chemin == "/proposer" -> proposer(sortie, r, s.inetAddress.hostAddress ?: "")
                    r.methode == "POST" && r.chemin == "/question" -> poserQuestion(sortie, r, s.inetAddress.hostAddress ?: "")
                    r.methode == "POST" && r.chemin.matches(Regex("^/question/[a-z0-9]{6,30}/plus$")) -> plusUn(sortie, r.chemin.split("/")[2], s.inetAddress.hostAddress ?: "")
                    r.methode == "POST" && r.chemin == "/vote" -> voter(sortie, r, s.inetAddress.hostAddress ?: "")
                    // ── Vie de la promo ──
                    r.methode == "POST" && r.chemin == "/publier" -> publier(sortie, r, s.inetAddress.hostAddress ?: "")
                    r.methode == "POST" && r.chemin == "/alerte" -> alerter(sortie, r, s.inetAddress.hostAddress ?: "")
                    r.methode == "POST" && r.chemin.matches(Regex("^/alerte/[a-z0-9]{6,30}/plus$")) -> confirmerAlerte(sortie, r.chemin.split("/")[2], s.inetAddress.hostAddress ?: "")
                    r.methode == "POST" && r.chemin == "/devinette" -> repondreDevinette(sortie, r, s.inetAddress.hostAddress ?: "")
                    // ── Réviser : profil (points, niveau, série) et défi du jour ──
                    r.methode == "POST" && r.chemin == "/profil" -> profil(sortie, r, s.inetAddress.hostAddress ?: "")
                    r.methode == "POST" && r.chemin == "/defi" -> repondreDefi(sortie, r, s.inetAddress.hostAddress ?: "")
                    // ── Quiz en direct ──
                    r.methode == "GET" && r.chemin == "/quiz.json" -> json(sortie, quizPublic(r.query["pid"] ?: ""))
                    r.methode == "POST" && r.chemin == "/quiz/repondre" -> repondreQuiz(sortie, r, s.inetAddress.hostAddress ?: "")
                    // ── Travailler ensemble : salle d'étude, fiches flash ──
                    r.methode == "POST" && r.chemin == "/etude" -> etude(sortie, r, s.inetAddress.hostAddress ?: "")
                    r.methode == "POST" && r.chemin == "/etude/fini" -> etudeFinie(sortie, r, s.inetAddress.hostAddress ?: "")
                    r.methode == "POST" && r.chemin == "/profil/fiches" -> pointsFiches(sortie, r, s.inetAddress.hostAddress ?: "")
                    // ── Ressources : corrigés, merci, entraide ──
                    r.methode == "POST" && r.chemin == "/corrige" -> deposerCorrige(sortie, r, s.inetAddress.hostAddress ?: "")
                    r.methode == "POST" && r.chemin.matches(Regex("^/corrige/[a-z0-9]{20}/utile$")) -> voterCorrige(sortie, r, r.chemin.split("/")[2], s.inetAddress.hostAddress ?: "")
                    r.methode == "POST" && r.chemin.matches(Regex("^/merci/[a-z0-9]{8,40}$")) -> merci(sortie, r, r.chemin.split("/")[2], s.inetAddress.hostAddress ?: "")
                    r.methode == "POST" && r.chemin.matches(Regex("^/question/[a-z0-9]{6,30}/repondre$")) -> repondreQuestion(sortie, r, r.chemin.split("/")[2], s.inetAddress.hostAddress ?: "")
                    r.methode == "POST" && r.chemin.matches(Regex("^/question/[a-z0-9]{6,30}/reponse/[a-z0-9]{12}/utile$")) ->
                        voterReponse(sortie, r, r.chemin.split("/")[2], r.chemin.split("/")[4], s.inetAddress.hostAddress ?: "")
                    // ── Star du jour (anniversaires) et événements ──
                    r.methode == "POST" && r.chemin == "/star" -> creerStar(sortie, r, s.inetAddress.hostAddress ?: "")
                    r.methode == "POST" && r.chemin.matches(Regex("^/star/[a-z0-9]{16}/photo$")) -> photoStar(sortie, r, r.chemin.split("/")[2], s.inetAddress.hostAddress ?: "")
                    r.methode == "POST" && r.chemin.matches(Regex("^/star/[a-z0-9]{16}/voeu$")) -> voeuStar(sortie, r, r.chemin.split("/")[2], s.inetAddress.hostAddress ?: "")
                    r.methode == "POST" && r.chemin.matches(Regex("^/evenement/[a-z0-9]{6,30}/viens$")) -> participer(sortie, r.chemin.split("/")[2], s.inetAddress.hostAddress ?: "")
                    r.methode == "POST" && r.chemin.startsWith("/c/") -> {
                        // Clic sur une publicité (appel, WhatsApp) : la preuve pour l'annonceur.
                        r.chemin.removePrefix("/c/").takeIf { it.matches(Regex("^[a-z0-9]{4,40}$")) }?.let { compter("clics", it) }
                        repondre(sortie, 200, "text/plain", ByteArray(0))
                    }
                    r.chemin.startsWith("/gerer") && !local -> repondre(sortie, 403, "text/plain; charset=utf-8", "Réservé au téléphone de l'éditeur.".toByteArray())
                    r.methode == "GET" && r.chemin == "/gerer" -> actif(sortie, "amphi/gerer.html", "text/html; charset=utf-8")
                    r.methode == "GET" && r.chemin == "/gerer/qrcode.js" -> actif(sortie, "amphi/qrcode.js", "text/javascript; charset=utf-8")
                    r.methode == "GET" && r.chemin == "/gerer/etat" -> json(sortie, etat())
                    r.methode == "GET" && r.chemin == "/gerer/contenu" -> json(sortie, lireContenu())
                    r.methode == "POST" && r.chemin == "/gerer/contenu" -> enregistrerContenu(sortie, r)
                    r.methode == "POST" && r.chemin == "/gerer/fichier" -> recevoirFichier(sortie, r)
                    r.methode == "POST" && r.chemin == "/gerer/sauvegarder" -> json(sortie, sauvegarder())
                    r.methode == "GET" && r.chemin == "/gerer/echanges" -> json(sortie, echangesGestion())
                    r.methode == "GET" && r.chemin.matches(Regex("^/gerer/proposition/[a-z0-9]{8,40}/f$")) -> {
                        val f = File(dossierPropositions, r.chemin.split("/")[3])
                        if (f.isFile) envoyerChemin(sortie, r, f, r.query["type"], r.query["nom"], null)
                        else repondre(sortie, 404, "text/plain", "Introuvable".toByteArray())
                    }
                    r.methode == "POST" && r.chemin.matches(Regex("^/gerer/proposition/[a-z0-9]{8,40}/(accepter|refuser)$")) -> {
                        val morceaux = r.chemin.split("/")
                        json(sortie, deciderProposition(morceaux[3], morceaux[4] == "accepter"))
                    }
                    r.methode == "POST" && r.chemin.matches(Regex("^/gerer/publication/[a-z0-9]{6,30}$")) -> json(sortie, modererPublication(r.chemin.split("/")[3], lireJson(r).optString("action")))
                    r.methode == "POST" && r.chemin == "/gerer/defi" -> json(sortie, defiPerso(lireJson(r)))
                    r.methode == "GET" && r.chemin == "/gerer/quiz" -> json(sortie, quizGestion())
                    r.methode == "POST" && r.chemin.matches(Regex("^/gerer/quiz/(creer|suivant|terminer|fermer)$")) -> json(sortie, piloterQuiz(r.chemin.substringAfterLast("/"), lireJson(r)))
                    r.methode == "POST" && r.chemin.matches(Regex("^/gerer/corrige/[a-z0-9]{20}$")) -> json(sortie, modererCorrige(r.chemin.split("/")[3], lireJson(r).optString("action")))
                    r.methode == "GET" && r.chemin.matches(Regex("^/gerer/corrige/[a-z0-9]{20}/f$")) -> {
                        val f = File(dossierFichiers, r.chemin.split("/")[3])
                        if (f.isFile) envoyerChemin(sortie, r, f, r.query["type"], r.query["nom"], null)
                        else repondre(sortie, 404, "text/plain", "Introuvable".toByteArray())
                    }
                    r.methode == "POST" && r.chemin.matches(Regex("^/gerer/star/[a-z0-9]{16}$")) -> json(sortie, modererStar(r.chemin.split("/")[3], lireJson(r).optString("action")))
                    r.methode == "POST" && r.chemin.matches(Regex("^/gerer/alerte/[a-z0-9]{6,30}$")) -> json(sortie, modererAlerte(r.chemin.split("/")[3], lireJson(r).optString("action")))
                    r.methode == "POST" && r.chemin.matches(Regex("^/gerer/question/[a-z0-9]{6,30}$")) -> {
                        json(sortie, modererQuestion(r.chemin.split("/")[3], lireJson(r)))
                    }
                    r.methode == "POST" && r.chemin == "/gerer/wifi" -> {
                        val q = lireJson(r)
                        json(sortie, creerWifi(q.optString("nom"), q.optString("motDePasse")))
                    }
                    r.methode == "POST" && r.chemin == "/gerer/wifi/arreter" -> {
                        prefs().edit().putBoolean("wifi", false).apply()
                        supprimerGroupe()
                        json(sortie, JSONObject().put("ok", true))
                    }
                    r.methode == "POST" && r.chemin == "/gerer/restaurer" -> restaurer(sortie, r)
                    r.methode == "GET" && r.chemin == "/gerer/relais" -> json(sortie, JSONObject(etatRelais.get().toString()).put("derniere", prefs().getLong("relais_date", 0L)))
                    r.methode == "POST" && r.chemin == "/gerer/relais/copier" -> json(sortie, lancerCopie())
                    else -> repondre(sortie, 404, "text/plain; charset=utf-8", "Introuvable".toByteArray())
                }
            } catch (_: Exception) {
            }
        }
    }

    private fun actif(sortie: OutputStream, chemin: String, type: String, cache: String = "no-store") {
        val octets = assets.open(chemin).use { it.readBytes() }
        repondre(sortie, 200, type, octets, cache = cache)
    }

    // ── Contenu ────────────────────────────────────────────────────────────

    private fun lireContenu(): JSONObject = synchronized(this) {
        try {
            if (fichierContenu.exists()) JSONObject(fichierContenu.readText()) else contenuDeDepart()
        } catch (_: Exception) {
            contenuDeDepart()
        }
    }

    private fun contenuDeDepart() = JSONObject()
        .put("titre", "Physique Fondamentale · Licence 3")
        .put("accueil", JSONObject()
            .put("titre", "Bienvenue sur le Wi-Fi de l'amphi")
            .put("texte", "Ici, tout est gratuit et sans forfait : le programme de la semaine, les annonces et les documents des cours.")
            .put("editeur", "Edmond Atinzoun TCHOKPON"))
        .put("annonces", JSONArray())
        .put("programme", JSONObject().put("jours", JSONArray()))
        .put("documents", JSONArray())
        .put("pubs", JSONArray())

    /** Ce que voient les étudiants : le contenu sans le texte brut collé, et
     * sans les annonces ni les publicités arrivées à leur date de fin. */
    private fun contenuPublic(): JSONObject {
        val c = lireContenu()
        c.optJSONObject("programme")?.remove("brut")
        c.optJSONObject("programmes")?.let { ps -> ps.keys().asSequence().toList().forEach { k -> ps.optJSONObject(k)?.remove("brut") } }
        val maintenant = System.currentTimeMillis()
        for (cle in listOf("annonces", "pubs")) {
            val t = c.optJSONArray(cle) ?: continue
            val gardes = JSONArray()
            for (i in 0 until t.length()) {
                val o = t.optJSONObject(i) ?: continue
                val fin = o.optLong("fin", 0L)
                if (fin <= 0L || fin > maintenant) gardes.put(o)
            }
            c.put(cle, gardes)
        }
        return c
    }

    private fun enregistrerContenu(sortie: OutputStream, r: Requete) {
        val taille = r.entetes["content-length"]?.toLongOrNull() ?: 0
        if (taille <= 0 || taille > CONTENU_MAX) return json(sortie, JSONObject().put("erreur", "trop gros"), 413)
        val octets = ByteArray(taille.toInt())
        var lus = 0
        while (lus < octets.size) {
            val n = r.corps.read(octets, lus, octets.size - lus)
            if (n < 0) break
            lus += n
        }
        val c = try { JSONObject(String(octets, 0, lus, Charsets.UTF_8)) } catch (_: Exception) {
            return json(sortie, JSONObject().put("erreur", "contenu illisible"), 400)
        }
        synchronized(this) {
            val tmp = File(dossier, "contenu.json.tmp")
            tmp.writeText(c.toString())
            tmp.renameTo(fichierContenu)
        }
        nettoyerFichiers(c)
        json(sortie, JSONObject().put("ok", true))
    }

    /** Efface les fichiers qui ne sont plus utilisés par le contenu enregistré. */
    private fun nettoyerFichiers(c: JSONObject) {
        val utilises = HashSet<String>()
        fun ajouter(t: JSONArray?, cle: String) {
            if (t == null) return
            for (i in 0 until t.length()) t.optJSONObject(i)?.optString(cle)?.takeIf { it.isNotEmpty() }?.let { utilises.add(it) }
        }
        ajouter(c.optJSONArray("documents"), "id")
        ajouter(c.optJSONArray("pubs"), "image")
        ajouter(c.optJSONArray("annonces"), "image")
        ajouter(c.optJSONArray("evenements"), "image")
        c.optJSONObject("accueil")?.optString("photo")?.takeIf { it.isNotEmpty() }?.let { utilises.add(it) }
        ajouter(lireTableau(fichierPublications), "image")
        ajouter(lireTableau(fichierCorriges), "cid")
        val st = lireTableau(fichierStars)
        for (i in 0 until st.length()) st.optJSONObject(i)?.optJSONArray("photos")?.let { ph -> for (k in 0 until ph.length()) utilises.add(ph.optString(k)) }
        val maintenant = System.currentTimeMillis()
        dossierFichiers.listFiles()?.forEach { f ->
            // Un fichier tout juste reçu (pas encore enregistré dans le contenu) est gardé une heure.
            if (f.isFile && f.name !in utilises && maintenant - f.lastModified() > 3_600_000) f.delete()
        }
    }

    private fun idValide(id: String) = id.matches(Regex("^[a-z0-9]{8,40}$"))

    private fun recevoirFichier(sortie: OutputStream, r: Requete) {
        val taille = r.entetes["content-length"]?.toLongOrNull() ?: 0
        if (taille <= 0 || taille > FICHIER_MAX) return json(sortie, JSONObject().put("erreur", "taille"), 413)
        if (dossierFichiers.usableSpace < taille + 200_000_000L) {
            return json(sortie, JSONObject().put("erreur", "Plus assez de place sur le téléphone."), 507)
        }
        val id = java.util.UUID.randomUUID().toString().replace("-", "").take(20)
        val f = File(dossierFichiers, id)
        f.outputStream().use { o ->
            val tampon = ByteArray(64 * 1024)
            var reste = taille
            while (reste > 0) {
                val n = r.corps.read(tampon, 0, minOf(tampon.size.toLong(), reste).toInt())
                if (n < 0) break
                o.write(tampon, 0, n)
                reste -= n
            }
            if (reste > 0) {
                f.delete()
                return json(sortie, JSONObject().put("erreur", "envoi interrompu"), 400)
            }
        }
        json(sortie, JSONObject().put("id", id).put("taille", taille))
    }

    private fun envoyerFichier(sortie: OutputStream, r: Requete, id: String) {
        if (!idValide(id)) return repondre(sortie, 404, "text/plain", "Introuvable".toByteArray())
        val f = File(dossierFichiers, id)
        if (!f.isFile) return repondre(sortie, 404, "text/plain", "Introuvable".toByteArray())
        envoyerChemin(sortie, r, f, r.query["type"], r.query["nom"], id)
    }

    private fun envoyerChemin(sortie: OutputStream, r: Requete, f: File, typeDemande: String?, nomDemande: String?, compte: String?) {
        val id = compte ?: ""
        val type = typeDemande?.takeIf { it.matches(Regex("^[a-z]+/[a-z0-9.+-]+$")) } ?: "application/octet-stream"
        val nom = (nomDemande ?: "document").replace(Regex("[\\r\\n\"\\\\]"), "_").take(150)
        val disposition = if (r.query["dl"] == "1") "attachment" else "inline"
        val total = f.length()
        // Plage demandée (vidéos : avancer dans la lecture sans tout recharger).
        var debut = 0L
        var fin = total - 1
        var partiel = false
        r.entetes["range"]?.let { plage ->
            val m = Regex("bytes=(\\d*)-(\\d*)").find(plage)
            if (m != null) {
                val a = m.groupValues[1]
                val b = m.groupValues[2]
                if (a.isNotEmpty()) { debut = a.toLong(); if (b.isNotEmpty()) fin = minOf(b.toLong(), total - 1) }
                else if (b.isNotEmpty()) { debut = maxOf(0L, total - b.toLong()) }
                partiel = true
            }
        }
        // Un téléchargement (ou l'affichage d'une image de publicité) = un de plus.
        if (debut == 0L && id.isNotEmpty() && r.query["relais"] != "1") compter("telechargements", id)
        if (debut > fin || debut >= total) {
            return repondre(sortie, 416, "text/plain", ByteArray(0), mapOf("Content-Range" to "bytes */$total"))
        }
        val longueur = fin - debut + 1
        val tete = StringBuilder(if (partiel) "HTTP/1.1 206 Partial Content\r\n" else "HTTP/1.1 200 OK\r\n")
            .append("Content-Type: $type\r\n")
            .append("Content-Length: $longueur\r\n")
            .append("Accept-Ranges: bytes\r\n")
            .append("Content-Disposition: $disposition; filename*=UTF-8''${java.net.URLEncoder.encode(nom, "UTF-8").replace("+", "%20")}\r\n")
            .append("Cache-Control: max-age=3600\r\n")
            .append("Connection: close\r\n")
        if (partiel) tete.append("Content-Range: bytes $debut-$fin/$total\r\n")
        tete.append("\r\n")
        sortie.write(tete.toString().toByteArray())
        f.inputStream().use { i ->
            i.skip(debut)
            val tampon = ByteArray(64 * 1024)
            var reste = longueur
            while (reste > 0) {
                val n = i.read(tampon, 0, minOf(tampon.size.toLong(), reste).toInt())
                if (n < 0) break
                sortie.write(tampon, 0, n)
                reste -= n
            }
        }
        sortie.flush()
    }

    // ── Visites (pour montrer l'audience aux annonceurs) ──────────────────

    private fun aujourdhui() = SimpleDateFormat("yyyy-MM-dd", Locale.FRANCE).format(Date())

    private fun chargerStats() {
        jourCourant = aujourdhui()
    }

    private fun stats(): JSONObject = try {
        if (fichierStats.exists()) JSONObject(fichierStats.readText()) else JSONObject()
    } catch (_: Exception) { JSONObject() }

    private fun compterVisite(adresse: String) = synchronized(visiteursDuJour) {
        val jour = aujourdhui()
        if (jour != jourCourant) {
            visiteursDuJour.clear()
            jourCourant = jour
        }
        val st = stats()
        val j = st.optJSONObject(jour) ?: JSONObject()
        j.put("vues", j.optInt("vues") + 1)
        if (visiteursDuJour.add(adresse)) j.put("visiteurs", j.optInt("visiteurs") + 1)
        st.put(jour, j)
        // Garder 60 jours.
        val cles = st.keys().asSequence().toList().sorted()
        cles.dropLast(60).forEach { st.remove(it) }
        try { fichierStats.writeText(st.toString()) } catch (_: Exception) {}
    }

    private fun compter(sorte: String, id: String) = synchronized(visiteursDuJour) {
        val c = try { if (fichierCompteurs.exists()) JSONObject(fichierCompteurs.readText()) else JSONObject() } catch (_: Exception) { JSONObject() }
        val t = c.optJSONObject(sorte) ?: JSONObject()
        t.put(id, t.optInt(id) + 1)
        c.put(sorte, t)
        try { fichierCompteurs.writeText(c.toString()) } catch (_: Exception) {}
    }

    private fun compteurs(): JSONObject = try {
        if (fichierCompteurs.exists()) JSONObject(fichierCompteurs.readText()) else JSONObject()
    } catch (_: Exception) { JSONObject() }

    // ── Sauvegarde et restauration (téléphone perdu, changé ou réinitialisé) ──

    /** Tout le portail dans un seul fichier, rangé dans « Téléchargements ». */
    private fun sauvegarder(): JSONObject {
        val nom = "Portail-Amphi-" + SimpleDateFormat("yyyy-MM-dd-HH'h'mm", Locale.FRANCE).format(Date()) + ".zip"
        return try {
            val valeurs = ContentValues().apply {
                put(MediaStore.Downloads.DISPLAY_NAME, nom)
                put(MediaStore.Downloads.MIME_TYPE, "application/zip")
                put(MediaStore.Downloads.IS_PENDING, 1)
            }
            val uri = contentResolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, valeurs)
                ?: return JSONObject().put("erreur", "Impossible de créer le fichier.")
            contentResolver.openOutputStream(uri)!!.use { sortie ->
                ZipOutputStream(sortie).use { z ->
                    z.putNextEntry(ZipEntry("contenu.json"))
                    z.write(lireContenu().toString().toByteArray())
                    z.closeEntry()
                    dossierFichiers.listFiles()?.filter { idValide(it.name) }?.forEach { f ->
                        z.putNextEntry(ZipEntry("fichiers/" + f.name))
                        f.inputStream().use { it.copyTo(z) }
                        z.closeEntry()
                    }
                }
            }
            valeurs.clear()
            valeurs.put(MediaStore.Downloads.IS_PENDING, 0)
            contentResolver.update(uri, valeurs, null, null)
            JSONObject().put("ok", true).put("nom", nom)
        } catch (e: Exception) {
            JSONObject().put("erreur", e.message ?: "échec")
        }
    }

    /** Ne lit pas plus que la taille annoncée de la requête. */
    private class Borne(private val source: InputStream, private var reste: Long) : InputStream() {
        override fun read(): Int = if (reste <= 0) -1 else source.read().also { if (it >= 0) reste-- }
        override fun read(b: ByteArray, off: Int, len: Int): Int {
            if (reste <= 0) return -1
            val n = source.read(b, off, minOf(len.toLong(), reste).toInt())
            if (n > 0) reste -= n
            return n
        }
    }

    private fun restaurer(sortie: OutputStream, r: Requete) {
        val taille = r.entetes["content-length"]?.toLongOrNull() ?: 0
        if (taille <= 0 || taille > FICHIER_MAX * 4) return json(sortie, JSONObject().put("erreur", "taille"), 413)
        if (dossierFichiers.usableSpace < taille + 200_000_000L) {
            return json(sortie, JSONObject().put("erreur", "Plus assez de place sur le téléphone."), 507)
        }
        var contenu: JSONObject? = null
        var fichiers = 0
        try {
            ZipInputStream(Borne(r.corps, taille)).use { z ->
                while (true) {
                    val e = z.nextEntry ?: break
                    val nom = e.name
                    when {
                        nom == "contenu.json" -> contenu = JSONObject(String(z.readBytes(), Charsets.UTF_8))
                        nom.startsWith("fichiers/") && idValide(nom.removePrefix("fichiers/")) -> {
                            File(dossierFichiers, nom.removePrefix("fichiers/")).outputStream().use { z.copyTo(it) }
                            fichiers++
                        }
                    }
                }
            }
        } catch (_: Exception) {
            return json(sortie, JSONObject().put("erreur", "Ce fichier n'est pas une sauvegarde du Portail Amphi."), 400)
        }
        val c = contenu ?: return json(sortie, JSONObject().put("erreur", "Ce fichier n'est pas une sauvegarde du Portail Amphi."), 400)
        synchronized(this) {
            val tmp = File(dossier, "contenu.json.tmp")
            tmp.writeText(c.toString())
            tmp.renameTo(fichierContenu)
        }
        json(sortie, JSONObject().put("ok", true).put("fichiers", fichiers))
    }

    // ── Échanges entre étudiants ───────────────────────────────────────────
    //
    // Ce que les étudiants déposent est rangé à part du contenu de l'éditeur
    // (fichiers JSON séparés) : une publication de l'éditeur n'écrase jamais
    // une question ou un vote. Limites par adresse contre les abus.

    private val fichierQuestions get() = File(dossier, "questions.json")
    private val fichierVotes get() = File(dossier, "votes.json")
    private val fichierPropositions get() = File(dossier, "propositions.json")
    private val dossierPartages get() = File(dossierFichiers, "partages").apply { mkdirs() }
    private val dossierPropositions get() = File(dossierFichiers, "propositions").apply { mkdirs() }
    private val verrouEchanges = Any()
    private val essais = HashMap<String, MutableList<Long>>()

    /** Au plus [max] actions de ce genre par adresse sur [fenetreMs]. */
    private fun permis(ip: String, genre: String, max: Int, fenetreMs: Long): Boolean {
        synchronized(essais) {
            val cle = "$genre|$ip"
            val maintenant = System.currentTimeMillis()
            val liste = essais.getOrPut(cle) { mutableListOf() }
            liste.removeAll { maintenant - it > fenetreMs }
            if (liste.size >= max) return false
            liste.add(maintenant)
            return true
        }
    }

    private fun lireTableau(f: File): JSONArray = try { if (f.exists()) JSONArray(f.readText()) else JSONArray() } catch (_: Exception) { JSONArray() }
    private fun lireObjet(f: File): JSONObject = try { if (f.exists()) JSONObject(f.readText()) else JSONObject() } catch (_: Exception) { JSONObject() }
    private fun ecrire(f: File, contenu: Any) {
        val tmp = File(f.parentFile, f.name + ".tmp")
        tmp.writeText(contenu.toString())
        tmp.renameTo(f)
    }

    private val MOTS_INTERDITS = Regex("\\b(con(nard|nasse)?|salope|pute|encul\\w*|fdp|ntm|batard|bâtard|merde)\\b", RegexOption.IGNORE_CASE)
    private fun texteCorrect(t: String) = !MOTS_INTERDITS.containsMatchIn(t)

    private fun recevoirVers(f: File, r: Requete, max: Long): Long? {
        val taille = r.entetes["content-length"]?.toLongOrNull() ?: 0
        if (taille <= 0 || taille > max || dossierFichiers.usableSpace < taille + 300_000_000L) return null
        f.outputStream().use { o ->
            val tampon = ByteArray(64 * 1024)
            var reste = taille
            while (reste > 0) {
                val n = r.corps.read(tampon, 0, minOf(tampon.size.toLong(), reste).toInt())
                if (n < 0) break
                o.write(tampon, 0, n)
                reste -= n
            }
            if (reste > 0) { f.delete(); return null }
        }
        return taille
    }

    private fun nomPropre(n: String?) = (n ?: "fichier").replace(Regex("[\\\\/:*?\"<>|\\r\\n]"), "_").trim().take(120).ifEmpty { "fichier" }
    private fun typePropre(t: String?) = t?.takeIf { it.matches(Regex("^[a-z]+/[a-z0-9.+-]+$")) } ?: "application/octet-stream"
    private fun texte(v: String?, max: Int) = (v ?: "").replace(Regex("[\\u0000-\\u0008\\u000B-\\u001F]"), "").trim().take(max)

    /** Partage d'un téléphone à l'autre : un code à 4 chiffres, valable 24 heures. */
    /** Plusieurs fichiers sous un même code : le premier envoi donne le code
     * et un jeton secret ; les suivants s'ajoutent avec ce jeton. */
    private fun deposerPartage(sortie: OutputStream, r: Requete, ip: String) {
        val ajout = r.query["code"]?.takeIf { it.matches(Regex("^\\d{4}$")) }
        val jetonDonne = r.query["jeton"] ?: ""
        val (genre, max) = if (ajout != null) "partage-ajout" to 80 else "partage" to 15
        if (!permis(ip, genre, max, 3_600_000)) return json(sortie, JSONObject().put("erreur", "Trop de partages : réessayez dans une heure."), 429)
        nettoyerPartages()
        val code: String
        val jeton: String
        val rang: Int
        val f: File
        val info: File
        synchronized(verrouEchanges) {
            if (ajout != null) {
                val deja = partages(ajout)
                if (deja.isEmpty()) return json(sortie, JSONObject().put("erreur", "Code expiré."), 404)
                if (jetonDonne.isEmpty() || deja[0].second.optString("jeton") != jetonDonne) return json(sortie, JSONObject().put("erreur", "Code refusé."), 403)
                if (deja.size >= 30) return json(sortie, JSONObject().put("erreur", "30 fichiers au plus par code."), 400)
                code = ajout; jeton = jetonDonne; rang = deja.size
            } else {
                var c: String
                do { c = (1000..9999).random().toString() } while (dossierPartages.listFiles()?.any { it.name.startsWith("$c-") } == true)
                code = c; jeton = java.util.UUID.randomUUID().toString().replace("-", "").take(16); rang = 0
            }
            // Le code est réservé tout de suite : fichier d'information écrit avant le contenu.
            f = File(dossierPartages, "$code-" + java.util.UUID.randomUUID().toString().replace("-", "").take(12))
            info = File(f.path + ".json")
            info.writeText(JSONObject().put("nom", nomPropre(r.query["nom"])).put("type", typePropre(r.query["type"])).put("taille", 0)
                .put("date", System.currentTimeMillis()).put("jeton", jeton).put("rang", rang).toString())
        }
        val taille = recevoirVers(f, r, 150_000_000L)
        if (taille == null) {
            info.delete()
            return json(sortie, JSONObject().put("erreur", "Fichier trop gros (150 Mo au plus) ou téléphone plein."), 413)
        }
        info.writeText(JSONObject(info.readText()).put("taille", taille).toString())
        json(sortie, JSONObject().put("ok", true).put("code", code).put("jeton", jeton))
    }

    /** Les fichiers d'un code, dans l'ordre d'envoi. */
    private fun partages(code: String): List<Pair<File, JSONObject>> =
        (dossierPartages.listFiles() ?: emptyArray())
            .filter { it.name.startsWith("$code-") && !it.name.endsWith(".json") }
            .mapNotNull { f -> try { f to JSONObject(File(f.path + ".json").readText()) } catch (_: Exception) { null } }
            .filter { it.second.optLong("taille") > 0 }
            .sortedBy { it.second.optInt("rang") }

    private fun infoPartage(sortie: OutputStream, code: String) {
        nettoyerPartages()
        val liste = partages(code)
        if (liste.isEmpty()) return json(sortie, JSONObject().put("erreur", "Code inconnu ou expiré."), 404)
        val fichiers = JSONArray()
        liste.forEach { (_, i) -> fichiers.put(JSONObject().put("nom", i.optString("nom")).put("type", i.optString("type")).put("taille", i.optLong("taille"))) }
        val premier = liste[0].second
        json(sortie, JSONObject().put("nom", premier.optString("nom")).put("type", premier.optString("type")).put("taille", premier.optLong("taille")).put("fichiers", fichiers))
    }

    private fun fichierPartage(sortie: OutputStream, r: Requete, code: String) {
        nettoyerPartages()
        val liste = partages(code)
        val (f, info) = liste.getOrNull(r.query["i"]?.toIntOrNull() ?: 0) ?: return repondre(sortie, 404, "text/plain", "Code inconnu ou expiré.".toByteArray())
        envoyerChemin(sortie, r, f, info.optString("type"), info.optString("nom"), null)
    }

    private fun nettoyerPartages() {
        val limite = System.currentTimeMillis() - 24 * 3_600_000L
        dossierPartages.listFiles()?.forEach { if (it.lastModified() < limite) it.delete() }
    }

    /** Un étudiant propose un document à la bibliothèque : publié seulement après validation. */
    private fun proposer(sortie: OutputStream, r: Requete, ip: String) {
        if (!permis(ip, "proposer", 10, 3_600_000)) return json(sortie, JSONObject().put("erreur", "Trop d'envois : réessayez dans une heure."), 429)
        val id = java.util.UUID.randomUUID().toString().replace("-", "").take(20)
        val f = File(dossierPropositions, id)
        val taille = recevoirVers(f, r, 200_000_000L)
            ?: return json(sortie, JSONObject().put("erreur", "Fichier trop gros (200 Mo au plus) ou téléphone plein."), 413)
        synchronized(verrouEchanges) {
            val t = lireTableau(fichierPropositions)
            t.put(JSONObject()
                .put("id", id).put("nom", nomPropre(r.query["nom"])).put("type", typePropre(r.query["type"]))
                .put("titre", texte(r.query["titre"], 120)).put("matiere", texte(r.query["matiere"], 60))
                .put("auteur", texte(r.query["auteur"], 40)).put("taille", taille).put("date", System.currentTimeMillis())
                .put("pid", (r.query["pid"] ?: "").takeIf { ID_PROFIL.matches(it) } ?: ""))
            ecrire(fichierPropositions, t)
        }
        json(sortie, JSONObject().put("ok", true))
    }

    private fun deciderProposition(id: String, accepter: Boolean): JSONObject {
      synchronized(verrouEchanges) {
        val t = lireTableau(fichierPropositions)
        val gardees = JSONArray()
        var trouvee: JSONObject? = null
        for (i in 0 until t.length()) { val o = t.getJSONObject(i); if (o.optString("id") == id) trouvee = o else gardees.put(o) }
        ecrire(fichierPropositions, gardees)
        val o = trouvee ?: return JSONObject().put("erreur", "Déjà traitée.")
        val f = File(dossierPropositions, id)
        if (!accepter) { f.delete(); return JSONObject().put("ok", true) }
        // Le fichier rejoint la bibliothèque ; la page de gestion l'ajoute au contenu.
        val nouvelId = java.util.UUID.randomUUID().toString().replace("-", "").take(20)
        if (!f.renameTo(File(dossierFichiers, nouvelId))) return JSONObject().put("erreur", "Fichier introuvable.")
        // Document validé : 15 points et une contribution pour son auteur.
        o.optString("pid").takeIf { ID_PROFIL.matches(it) }?.let { pid ->
            val profils = lireObjet(fichierProfils); ajouterPoints(profils, pid, 15); ajouterContribution(profils, pid); ecrire(fichierProfils, profils)
        }
        return JSONObject().put("ok", true).put("document", JSONObject(o.toString()).put("id", nouvelId))
      }
    }

    /** Questions au professeur et demandes de documents : le mur de la promo. */
    private fun poserQuestion(sortie: OutputStream, r: Requete, ip: String) {
        if (!permis(ip, "question", 6, 600_000)) return json(sortie, JSONObject().put("erreur", "Trop de messages : attendez quelques minutes."), 429)
        val q = lireJson(r)
        val t = texte(q.optString("texte"), 400)
        if (t.length < 3) return json(sortie, JSONObject().put("erreur", "Message trop court."), 400)
        if (!texteCorrect(t)) return json(sortie, JSONObject().put("erreur", "Message refusé : restez correct."), 400)
        val sorte = q.optString("sorte").takeIf { it in setOf("question", "demande", "info", "idee") } ?: "question"
        val id = java.util.UUID.randomUUID().toString().replace("-", "").take(12)
        synchronized(verrouEchanges) {
            val tout = lireTableau(fichierQuestions)
            tout.put(JSONObject().put("id", id).put("texte", t).put("sorte", sorte).put("filiere", texte(q.optString("filiere"), 40))
                .put("date", System.currentTimeMillis()).put("votes", 0).put("ip", ip))
            // 300 messages au plus : les plus anciens partent.
            while (tout.length() > 300) tout.remove(0)
            ecrire(fichierQuestions, tout)
        }
        json(sortie, JSONObject().put("ok", true).put("id", id))
    }

    private fun plusUn(sortie: OutputStream, id: String, ip: String) {
        synchronized(verrouEchanges) {
            val tout = lireTableau(fichierQuestions)
            for (i in 0 until tout.length()) {
                val o = tout.getJSONObject(i)
                if (o.optString("id") != id) continue
                val qui = o.optJSONArray("qui") ?: JSONArray()
                val deja = (0 until qui.length()).any { qui.optString(it) == ip }
                if (!deja) { qui.put(ip); o.put("qui", qui); o.put("votes", o.optInt("votes") + 1); ecrire(fichierQuestions, tout) }
                break
            }
        }
        json(sortie, JSONObject().put("ok", true))
    }

    private fun voter(sortie: OutputStream, r: Requete, ip: String) {
        val q = lireJson(r)
        val sondage = q.optString("sondage")
        val option = q.optInt("option", -1)
        if (!sondage.matches(Regex("^[a-z0-9]{4,30}$")) || option < 0 || option > 20) return json(sortie, JSONObject().put("erreur", "Vote invalide."), 400)
        val s = lireContenu().optJSONArray("sondages")
        val existe = s != null && (0 until s.length()).any { s.optJSONObject(it)?.let { o -> o.optString("id") == sondage && o.optBoolean("ouvert", true) } == true }
        if (!existe) return json(sortie, JSONObject().put("erreur", "Ce sondage est clos."), 400)
        synchronized(verrouEchanges) {
            val v = lireObjet(fichierVotes)
            val parIp = v.optJSONObject(sondage) ?: JSONObject()
            parIp.put(ip, option) // un vote par téléphone ; revoter le change
            v.put(sondage, parIp)
            ecrire(fichierVotes, v)
        }
        json(sortie, JSONObject().put("ok", true))
    }

    private fun resultats(): JSONObject {
        val v = lireObjet(fichierVotes)
        val res = JSONObject()
        v.keys().forEach { sondage ->
            val parIp = v.optJSONObject(sondage) ?: return@forEach
            val comptes = HashMap<Int, Int>()
            parIp.keys().forEach { ip -> val o = parIp.optInt(ip, -1); if (o >= 0) comptes[o] = (comptes[o] ?: 0) + 1 }
            val t = JSONObject()
            comptes.forEach { (o, n) -> t.put(o.toString(), n) }
            res.put(sondage, t)
        }
        return res
    }

    /** Ce que voient les étudiants : questions visibles (sans adresses), résultats des sondages. */
    private fun echangesPublics(ip: String): JSONObject {
        val tout = lireTableau(fichierQuestions)
        val visibles = JSONArray()
        for (i in 0 until tout.length()) {
            val o = tout.getJSONObject(i)
            if (o.optBoolean("masquee")) continue
            val qui = o.optJSONArray("qui")
            val jai = qui != null && (0 until qui.length()).any { qui.optString(it) == ip }
            visibles.put(JSONObject().put("id", o.optString("id")).put("texte", o.optString("texte")).put("sorte", o.optString("sorte"))
                .put("filiere", o.optString("filiere")).put("date", o.optLong("date")).put("votes", o.optInt("votes"))
                .put("reponse", o.optString("reponse")).put("moi", jai).put("reponses", reponsesPubliques(o)))
        }
        val v = lireObjet(fichierVotes)
        val mesVotes = JSONObject()
        v.keys().forEach { k -> v.optJSONObject(k)?.let { if (it.has(ip)) mesVotes.put(k, it.optInt(ip)) } }
        return JSONObject().put("questions", visibles).put("resultats", resultats()).put("mesVotes", mesVotes)
            .put("publications", publicationsPubliques())
            .put("alertes", alertesActives(ip))
            .put("motDuJour", motDuJour())
            .put("devinette", devinettePublique(ip))
            .put("stars", starsPubliques())
            .put("participations", participations(ip))
            .put("corriges", corrigesPublics(false))
            .put("salle", salle())
            .put("mercis", mercisPublics())
    }

    private fun echangesGestion(): JSONObject {
        val q = lireTableau(fichierQuestions)
        val sansIp = JSONArray()
        for (i in 0 until q.length()) { val o = JSONObject(q.getJSONObject(i).toString()); o.remove("ip"); o.remove("qui"); sansIp.put(o) }
        val pubs = lireTableau(fichierPublications)
        val pubsSansIp = JSONArray()
        for (i in 0 until pubs.length()) { val o = JSONObject(pubs.getJSONObject(i).toString()); o.remove("ip"); pubsSansIp.put(o) }
        return JSONObject().put("propositions", lireTableau(fichierPropositions)).put("questions", sansIp).put("resultats", resultats())
            .put("publications", pubsSansIp).put("defi", defiGestion()).put("corriges", corrigesPublics(true)).put("honneur", honneur(lireObjet(fichierProfils))).put("stars", starsGestion()).put("participations", participations("")).put("alertes", alertesActives("")).put("motDuJour", motDuJour()).put("devinette", devinettePublique("").put("reponses", devinetteDuJour().optJSONArray("reponses")))
    }

    private fun modererQuestion(id: String, q: JSONObject): JSONObject = synchronized(verrouEchanges) {
        val tout = lireTableau(fichierQuestions)
        val gardees = JSONArray()
        for (i in 0 until tout.length()) {
            val o = tout.getJSONObject(i)
            if (o.optString("id") == id) {
                when (q.optString("action")) {
                    "supprimer" -> continue
                    "masquer" -> o.put("masquee", true)
                    "afficher" -> o.put("masquee", false)
                    "repondre" -> o.put("reponse", texte(q.optString("reponse"), 600))
                    "retirer-reponse" -> {
                        val rs = o.optJSONArray("reponses") ?: JSONArray()
                        val restantes = JSONArray()
                        for (k in 0 until rs.length()) rs.optJSONObject(k)?.takeIf { it.optString("rid") != q.optString("rid") }?.let { restantes.put(it) }
                        o.put("reponses", restantes)
                    }
                }
            }
            gardees.put(o)
        }
        ecrire(fichierQuestions, gardees)
        JSONObject().put("ok", true)
    }

    // ── Vie de la promo : dédicaces, objets perdus, petit marché ──────────
    //
    // Tout ce qu'écrivent les étudiants attend la validation de l'éditeur
    // (« attente »), puis s'affiche (« publie ») ; un objet rendu ou vendu
    // passe à « clos ». Pas de numéro de téléphone dans une dédicace.

    private val fichierPublications get() = File(dossier, "publications.json")
    private val fichierAlertes get() = File(dossier, "alertes.json")
    private val fichierDevinette get() = File(dossier, "devinette.json")
    private val SORTES_PUBLICATION = setOf("dedicace", "perdu", "trouve", "vente", "achat", "binome")
    private val NUMERO = Regex("(\\+?\\d[\\s.-]?){8,}")

    private fun publier(sortie: OutputStream, r: Requete, ip: String) {
        if (!permis(ip, "publier", 6, 3_600_000)) return json(sortie, JSONObject().put("erreur", "Trop de messages : réessayez dans une heure."), 429)
        val sorte = r.query["sorte"]?.takeIf { it in SORTES_PUBLICATION } ?: return json(sortie, JSONObject().put("erreur", "Type inconnu."), 400)
        val texteMsg = texte(r.query["texte"], 300)
        if (texteMsg.length < 3) return json(sortie, JSONObject().put("erreur", "Message trop court."), 400)
        if (!texteCorrect(texteMsg)) return json(sortie, JSONObject().put("erreur", "Message refusé : restez correct."), 400)
        if (sorte == "dedicace" && NUMERO.containsMatchIn(texteMsg)) return json(sortie, JSONObject().put("erreur", "Pas de numéro de téléphone dans une dédicace."), 400)
        val id = java.util.UUID.randomUUID().toString().replace("-", "").take(16)
        var image = ""
        val taille = r.entetes["content-length"]?.toLongOrNull() ?: 0
        if (taille > 0) {
            if (taille > 6_000_000) return json(sortie, JSONObject().put("erreur", "Photo trop lourde."), 413)
            val f = File(dossierFichiers, id + "p")
            if (recevoirVers(f, r, 6_000_000L) == null) return json(sortie, JSONObject().put("erreur", "Photo non reçue."), 400)
            image = f.name
        }
        synchronized(verrouEchanges) {
            val t = lireTableau(fichierPublications)
            t.put(JSONObject().put("id", id).put("sorte", sorte).put("texte", texteMsg)
                .put("prix", texte(r.query["prix"], 30)).put("contact", texte(r.query["contact"], 60))
                .put("filiere", texte(r.query["filiere"], 40)).put("image", image)
                .put("ue", texte(r.query["ue"], 60)).put("quartier", texte(r.query["quartier"], 40))
                .put("statut", "attente").put("date", System.currentTimeMillis()).put("ip", ip))
            while (t.length() > 400) t.remove(0)
            ecrire(fichierPublications, t)
        }
        json(sortie, JSONObject().put("ok", true))
    }

    private fun publicationsPubliques(): JSONArray {
        val t = lireTableau(fichierPublications)
        val maintenant = System.currentTimeMillis()
        val r = JSONArray()
        for (i in 0 until t.length()) {
            val o = t.getJSONObject(i)
            val statut = o.optString("statut")
            if (statut != "publie" && statut != "clos") continue
            val age = maintenant - o.optLong("date")
            // Dédicaces : 7 jours ; objets et annonces du marché : 30 jours ; clos : 3 jours.
            val duree = if (statut == "clos") 3L else if (o.optString("sorte") == "dedicace") 7L else 30L
            if (age > duree * 86_400_000L) continue
            val p = JSONObject(o.toString()); p.remove("ip")
            r.put(p)
        }
        return r
    }

    private fun modererPublication(id: String, action: String): JSONObject {
        synchronized(verrouEchanges) {
            val t = lireTableau(fichierPublications)
            val gardees = JSONArray()
            for (i in 0 until t.length()) {
                val o = t.getJSONObject(i)
                if (o.optString("id") == id) {
                    when (action) {
                        "publier" -> o.put("statut", "publie")
                        "clore" -> o.put("statut", "clos").put("date", System.currentTimeMillis())
                        "refuser", "supprimer" -> {
                            o.optString("image").takeIf { it.isNotEmpty() }?.let { File(dossierFichiers, it).delete() }
                            continue
                        }
                    }
                }
                gardees.put(o)
            }
            ecrire(fichierPublications, gardees)
        }
        return JSONObject().put("ok", true)
    }

    // ── Réviser : chaque téléphone a un identifiant tiré au hasard (pas de
    // compte, pas de nom réel) ; les points récompensent l'effort. ─────────

    private val fichierProfils get() = File(dossier, "profils.json")
    private val fichierDefiJour get() = File(dossier, "defi-jour.json")
    private val fichierDefiPerso get() = File(dossier, "defi-perso.json")
    private val ID_PROFIL = Regex("^[a-z0-9]{12,32}$")

    private fun jourIso(decalage: Int = 0) = SimpleDateFormat("yyyy-MM-dd", Locale.FRANCE).format(Date(System.currentTimeMillis() + decalage * 86_400_000L))
    private fun semaine(): String {
        val c = java.util.Calendar.getInstance(Locale.FRANCE)
        return String.format(Locale.ROOT, "%d-%02d", c.weekYear, c.get(java.util.Calendar.WEEK_OF_YEAR))
    }
    private fun pseudoPropre(v: String?): String {
        val p = texte(v, 20)
        return if (p.isEmpty() || !texteCorrect(p) || NUMERO.containsMatchIn(p)) "" else p
    }

    /** Ajoute des points (total et semaine en cours). À appeler sous verrouEchanges. */
    private fun ajouterPoints(profils: JSONObject, id: String, n: Int) {
        if (!ID_PROFIL.matches(id) || n == 0) return
        val p = profils.optJSONObject(id) ?: JSONObject().put("points", 0).also { profils.put(id, it) }
        p.put("points", p.optInt("points") + n)
        val sem = p.optJSONObject("semaines") ?: JSONObject().also { p.put("semaines", it) }
        val k = semaine()
        sem.put(k, sem.optInt(k) + n)
        sem.keys().asSequence().toList().sorted().dropLast(8).forEach { sem.remove(it) }
    }

    /** Une ressource validée (document, corrigé) ou une réponse jugée utile. */
    private fun ajouterContribution(profils: JSONObject, id: String) {
        if (!ID_PROFIL.matches(id)) return
        val p = profils.optJSONObject(id) ?: JSONObject().put("points", 0).also { profils.put(id, it) }
        p.put("contrib", p.optInt("contrib") + 1)
    }

    private fun profil(sortie: OutputStream, r: Requete, ip: String) {
        if (!permis(ip, "profil", 90, 3_600_000)) return json(sortie, JSONObject().put("erreur", "Attendez un peu."), 429)
        val q = lireJson(r)
        val id = q.optString("id")
        if (!ID_PROFIL.matches(id)) return json(sortie, JSONObject().put("erreur", "Identifiant invalide."), 400)
        val pseudo = pseudoPropre(q.optString("pseudo"))
        val reponse = JSONObject()
        synchronized(verrouEchanges) {
            val profils = lireObjet(fichierProfils)
            if (!profils.has(id) && profils.length() >= 5000) return json(sortie, JSONObject().put("erreur", "Trop de profils."), 507)
            val p = profils.optJSONObject(id) ?: JSONObject().put("points", 0).also { profils.put(id, it) }
            if (pseudo.isNotEmpty()) p.put("pseudo", pseudo)
            val auj = jourIso()
            if (p.optString("dernier") != auj) {
                // Série : jours d'affilée ; +1 point par jour, +10 tous les 7 jours.
                val serie = if (p.optString("dernier") == jourIso(-1)) p.optInt("serie") + 1 else 1
                p.put("serie", serie).put("meilleure", maxOf(serie, p.optInt("meilleure"))).put("dernier", auj)
                ajouterPoints(profils, id, 1 + if (serie % 7 == 0) 10 else 0)
            }
            ecrire(fichierProfils, profils)
            reponse.put("profil", profilPublic(profils, id)).put("honneur", honneur(profils))
        }
        json(sortie, reponse.put("defi", defiPublic(id)))
    }

    private fun profilPublic(profils: JSONObject, id: String): JSONObject {
        val p = profils.optJSONObject(id) ?: JSONObject()
        val k = semaine()
        val moi = p.optJSONObject("semaines")?.optInt(k) ?: 0
        var devant = 0
        profils.keys().forEach { x -> if ((profils.optJSONObject(x)?.optJSONObject("semaines")?.optInt(k) ?: 0) > moi) devant++ }
        val serie = if (p.optString("dernier") == jourIso() || p.optString("dernier") == jourIso(-1)) p.optInt("serie") else 0
        return JSONObject().put("pseudo", p.optString("pseudo")).put("points", p.optInt("points")).put("serie", serie)
            .put("meilleure", p.optInt("meilleure")).put("semaine", moi).put("rang", if (moi > 0) devant + 1 else 0)
    }

    /** Tableau d'honneur : les 10 meilleurs de la semaine et de tous les temps. */
    private fun honneur(profils: JSONObject): JSONObject {
        val k = semaine()
        val tous = profils.keys().asSequence().mapNotNull { profils.optJSONObject(it) }.toList()
        fun top(valeur: (JSONObject) -> Int): JSONArray {
            val t = JSONArray()
            tous.map { it to valeur(it) }.filter { it.second > 0 }.sortedByDescending { it.second }.take(10)
                .forEach { (p, v) -> t.put(JSONObject().put("pseudo", p.optString("pseudo").ifEmpty { "Anonyme" }).put("points", v).put("serie", p.optInt("serie"))) }
            return t
        }
        return JSONObject().put("semaine", top { it.optJSONObject("semaines")?.optInt(k) ?: 0 }).put("total", top { it.optInt("points") })
            .put("contributeurs", top { it.optInt("contrib") })
    }

    /** Le défi du jour : celui du délégué s'il en a mis un aujourd'hui, sinon la réserve. */
    private fun defiDuJour(): JSONObject {
        val perso = lireObjet(fichierDefiPerso)
        if (perso.optString("date") == jourIso() && perso.optString("question").isNotEmpty()) return perso
        val r = reserve("defis.json")
        if (r.length() == 0) return JSONObject()
        // Pas de 7 : les UE alternent d'un jour à l'autre.
        val i = Math.floorMod(numeroDuJour() * 7, r.length())
        return r.getJSONObject(i)
    }

    private fun etatDefi(): JSONObject {
        val e = lireObjet(fichierDefiJour)
        return if (e.optString("date") == jourIso()) e
        else JSONObject().put("date", jourIso()).put("reponses", JSONObject()).put("ips", JSONObject()).put("gagnants", JSONArray())
    }

    private fun defiPublic(id: String): JSONObject {
        val d = defiDuJour()
        if (d.optString("question").isEmpty()) return JSONObject()
        val e = etatDefi()
        val g = e.optJSONArray("gagnants") ?: JSONArray()
        val premiers = JSONArray()
        for (i in 0 until minOf(10, g.length())) premiers.put(g.get(i))
        val o = JSONObject().put("question", d.optString("question")).put("choix", d.optJSONArray("choix") ?: JSONArray())
            .put("ue", d.optString("ue")).put("gagnants", premiers).put("trouve", g.length())
            .put("participants", e.optJSONObject("reponses")?.length() ?: 0)
        e.optJSONObject("reponses")?.optJSONObject(id)?.let { rep ->
            o.put("reponse", JSONObject(rep.toString()).put("bonne", d.optInt("bonne")).put("explication", d.optString("explication")))
        }
        return o
    }

    private fun repondreDefi(sortie: OutputStream, r: Requete, ip: String) {
        val q = lireJson(r)
        val id = q.optString("id")
        if (!ID_PROFIL.matches(id)) return json(sortie, JSONObject().put("erreur", "Identifiant invalide."), 400)
        val d = defiDuJour()
        val choix = q.optInt("choix", -1)
        if (d.optString("question").isEmpty()) return json(sortie, JSONObject().put("erreur", "Pas de défi aujourd'hui."), 404)
        if (choix !in 0 until (d.optJSONArray("choix")?.length() ?: 0)) return json(sortie, JSONObject().put("erreur", "Choix invalide."), 400)
        val pseudo = pseudoPropre(q.optString("pseudo"))
        synchronized(verrouEchanges) {
            val e = etatDefi()
            val reps = e.optJSONObject("reponses") ?: JSONObject()
            val ips = e.optJSONObject("ips") ?: JSONObject()
            if (!reps.has(id)) {
                // Une réponse par téléphone et par jour (pas de seconde chance avec un autre identifiant).
                if (ips.optInt(ip) >= 1) return json(sortie, JSONObject().put("erreur", "Ce téléphone a déjà répondu aujourd'hui."), 429)
                val juste = choix == d.optInt("bonne")
                var rang = 0
                if (juste) {
                    val g = e.optJSONArray("gagnants") ?: JSONArray()
                    g.put(JSONObject().put("pseudo", pseudo.ifEmpty { "Anonyme" }).put("heure", System.currentTimeMillis()))
                    e.put("gagnants", g)
                    rang = g.length()
                }
                reps.put(id, JSONObject().put("choix", choix).put("juste", juste).put("rang", rang))
                ips.put(ip, ips.optInt(ip) + 1)
                e.put("reponses", reps).put("ips", ips)
                ecrire(fichierDefiJour, e)
                val profils = lireObjet(fichierProfils)
                // 10 points si juste (+5 pour les 3 premiers), 2 pour avoir essayé.
                ajouterPoints(profils, id, if (juste) 10 + (if (rang in 1..3) 5 else 0) else 2)
                if (pseudo.isNotEmpty()) profils.optJSONObject(id)?.put("pseudo", pseudo)
                ecrire(fichierProfils, profils)
            }
        }
        val profils = lireObjet(fichierProfils)
        json(sortie, JSONObject().put("ok", true).put("defi", defiPublic(id)).put("profil", profilPublic(profils, id)))
    }

    // ── Quiz en direct : le délégué lance les questions, tout l'amphi répond
    // en même temps ; plus on répond vite et juste, plus on marque. ───────

    private val verrouQuiz = Any()
    private var quiz: JSONObject? = null

    private fun piloterQuiz(action: String, q: JSONObject): JSONObject { synchronized(verrouQuiz) {
        val maintenant = System.currentTimeMillis()
        when (action) {
            "creer" -> {
                val ue = texte(q.optString("ue"), 20)
                val banque = reserve("defis.json")
                val choisies = (0 until banque.length()).map { banque.getJSONObject(it) }.filter { ue.isEmpty() || it.optString("ue") == ue }.shuffled()
                    .take(q.optInt("nombre", 10).coerceIn(3, 20))
                if (choisies.isEmpty()) return JSONObject().put("erreur", "Aucune question pour cette UE.")
                quiz = JSONObject().put("id", java.util.UUID.randomUUID().toString().take(8)).put("titre", texte(q.optString("titre"), 80).ifEmpty { "Quiz de la promo" })
                    .put("questions", JSONArray(choisies)).put("index", -1).put("phase", "attente")
                    .put("duree", q.optInt("duree", 20).coerceIn(10, 60)).put("debut", 0L)
                    .put("reponses", JSONObject()).put("scores", JSONObject())
            }
            "suivant" -> {
                val z = quiz ?: return JSONObject().put("erreur", "Pas de quiz.")
                val n = z.optJSONArray("questions")?.length() ?: 0
                val i = z.optInt("index") + 1
                if (i >= n) return piloterQuiz("terminer", q)
                z.put("index", i).put("phase", "question").put("debut", maintenant)
            }
            "terminer" -> {
                val z = quiz ?: return JSONObject().put("erreur", "Pas de quiz.")
                if (z.optString("phase") != "fin") {
                    z.put("phase", "fin")
                    // Podium : +10, +6, +3 points.
                    val profils = lireObjet(fichierProfils)
                    classement(z).take(3).forEachIndexed { k, (pid, _) -> ajouterPoints(profils, pid, intArrayOf(10, 6, 3)[k]) }
                    synchronized(verrouEchanges) { ecrire(fichierProfils, profils) }
                }
            }
            "fermer" -> quiz = null
        }
        return JSONObject().put("ok", true)
    } }

    private fun classement(z: JSONObject): List<Pair<String, JSONObject>> {
        val sc = z.optJSONObject("scores") ?: return emptyList()
        return sc.keys().asSequence().mapNotNull { k -> sc.optJSONObject(k)?.let { k to it } }.sortedByDescending { it.second.optInt("score") }.toList()
    }

    private fun tempsRestant(z: JSONObject): Int {
        if (z.optString("phase") != "question") return 0
        return maxOf(0, (z.optLong("debut") + z.optInt("duree") * 1000L - System.currentTimeMillis()).toInt() / 1000)
    }

    private fun quizPublic(pid: String): JSONObject { synchronized(verrouQuiz) {
        val z = quiz ?: return JSONObject().put("actif", false)
        val o = JSONObject().put("actif", true).put("id", z.optString("id")).put("titre", z.optString("titre")).put("phase", z.optString("phase"))
            .put("index", z.optInt("index")).put("total", z.optJSONArray("questions")?.length() ?: 0).put("duree", z.optInt("duree"))
        val i = z.optInt("index")
        val restant = tempsRestant(z)
        o.put("restant", restant)
        if (i >= 0 && z.optString("phase") != "attente") {
            val qu = z.getJSONArray("questions").getJSONObject(i)
            o.put("question", qu.optString("question")).put("choix", qu.optJSONArray("choix")).put("ue", qu.optString("ue"))
            val reps = z.optJSONObject("reponses")?.optJSONObject(i.toString())
            o.put("nbReponses", reps?.length() ?: 0)
            // La bonne réponse n'est montrée qu'une fois le temps écoulé.
            if (restant == 0) o.put("bonne", qu.optInt("bonne")).put("explication", qu.optString("explication"))
            reps?.optJSONObject(pid)?.let { o.put("monChoix", it.optInt("choix")) }
        }
        val top = JSONArray()
        val cl = classement(z)
        cl.take(10).forEach { (_, v) -> top.put(JSONObject().put("pseudo", v.optString("pseudo").ifEmpty { "Anonyme" }).put("score", v.optInt("score"))) }
        o.put("classement", top).put("joueurs", cl.size)
        z.optJSONObject("scores")?.optJSONObject(pid)?.let { me -> o.put("monScore", me.optInt("score")).put("monRang", cl.indexOfFirst { it.first == pid } + 1) }
        return o
    } }

    private fun quizGestion(): JSONObject { synchronized(verrouQuiz) {
        val z = quiz ?: return JSONObject().put("actif", false)
        val o = quizPublic("")
        val i = z.optInt("index")
        if (i >= 0) o.put("bonne", z.getJSONArray("questions").getJSONObject(i).optInt("bonne"))
        return o
    } }

    private fun repondreQuiz(sortie: OutputStream, r: Requete, ip: String) {
        val q = lireJson(r)
        val pid = q.optString("pid")
        if (!ID_PROFIL.matches(pid)) return json(sortie, JSONObject().put("erreur", "Identifiant invalide."), 400)
        val juste: Boolean
        val gain: Int
        synchronized(verrouQuiz) {
            val z = quiz ?: return json(sortie, JSONObject().put("erreur", "Pas de quiz en cours."), 404)
            val i = z.optInt("index")
            if (z.optString("phase") != "question" || q.optInt("index", -2) != i || tempsRestant(z) <= 0) return json(sortie, JSONObject().put("erreur", "Trop tard pour cette question."), 409)
            val reps = z.optJSONObject("reponses")!!
            val ici = reps.optJSONObject(i.toString()) ?: JSONObject().also { reps.put(i.toString(), it) }
            if (ici.has(pid) || ici.has("ip:$ip")) return json(sortie, JSONObject().put("erreur", "Déjà répondu."), 409)
            val choix = q.optInt("choix", -1)
            val qu = z.getJSONArray("questions").getJSONObject(i)
            juste = choix == qu.optInt("bonne")
            // 100 points si juste, plus jusqu'à 100 de bonus de rapidité.
            gain = if (juste) 100 + 100 * tempsRestant(z) / maxOf(1, z.optInt("duree")) else 0
            ici.put(pid, JSONObject().put("choix", choix)).put("ip:$ip", true)
            val sc = z.optJSONObject("scores")!!
            val me = sc.optJSONObject(pid) ?: JSONObject().put("score", 0).also { sc.put(pid, it) }
            me.put("score", me.optInt("score") + gain).put("pseudo", pseudoPropre(q.optString("pseudo")))
        }
        if (juste) synchronized(verrouEchanges) { val profils = lireObjet(fichierProfils); ajouterPoints(profils, pid, 1); ecrire(fichierProfils, profils) }
        json(sortie, JSONObject().put("ok", true).put("gain", gain))
    }

    // ── Salle d'étude : qui révise en ce moment (en mémoire seulement) ──

    private val etudiants = HashMap<String, JSONObject>()

    private fun etude(sortie: OutputStream, r: Requete, ip: String) {
        if (!permis(ip, "etude", 40, 3_600_000)) return json(sortie, JSONObject().put("erreur", "Attendez un peu."), 429)
        val q = lireJson(r)
        val id = q.optString("id")
        if (!ID_PROFIL.matches(id)) return json(sortie, JSONObject().put("erreur", "Identifiant invalide."), 400)
        val maintenant = System.currentTimeMillis()
        synchronized(etudiants) {
            val e = etudiants[id]
            // Une session continue tant que le téléphone donne signe de vie (moins de 5 min d'écart).
            val debut = if (e == null || maintenant - e.optLong("vu") > 300_000) maintenant else e.optLong("debut")
            etudiants[id] = JSONObject().put("pseudo", pseudoPropre(q.optString("pseudo"))).put("ue", texte(q.optString("ue"), 40))
                .put("vu", maintenant).put("debut", debut)
        }
        json(sortie, JSONObject().put("ok", true).put("salle", salle()))
    }

    /** 25 minutes de travail terminées : 3 points (4 sessions récompensées par jour au plus). */
    private fun etudeFinie(sortie: OutputStream, r: Requete, ip: String) {
        val id = lireJson(r).optString("id")
        val maintenant = System.currentTimeMillis()
        val e = synchronized(etudiants) { etudiants[id] }
        if (e == null || maintenant - e.optLong("debut") < 20 * 60_000L) return json(sortie, JSONObject().put("ok", true).put("points", 0))
        synchronized(etudiants) { e.put("debut", maintenant) }
        if (!permis(ip, "etude-fini", 4, 86_400_000)) return json(sortie, JSONObject().put("ok", true).put("points", 0))
        synchronized(verrouEchanges) { val profils = lireObjet(fichierProfils); ajouterPoints(profils, id, 3); ecrire(fichierProfils, profils) }
        json(sortie, JSONObject().put("ok", true).put("points", 3))
    }

    private fun salle(): JSONObject {
        val limite = System.currentTimeMillis() - 150_000
        val ues = JSONObject()
        val noms = JSONArray()
        var n = 0
        synchronized(etudiants) {
            etudiants.entries.removeAll { it.value.optLong("vu") < limite }
            etudiants.values.forEach { e ->
                n++
                e.optString("ue").takeIf { it.isNotEmpty() }?.let { ues.put(it, ues.optInt(it) + 1) }
                e.optString("pseudo").takeIf { it.isNotEmpty() && noms.length() < 15 }?.let { noms.put(it) }
            }
        }
        return JSONObject().put("n", n).put("ues", ues).put("noms", noms)
    }

    /** Une session de fiches flash : 2 points, 3 fois par jour au plus. */
    private fun pointsFiches(sortie: OutputStream, r: Requete, ip: String) {
        val id = lireJson(r).optString("id")
        if (!ID_PROFIL.matches(id)) return json(sortie, JSONObject().put("erreur", "Identifiant invalide."), 400)
        if (!permis(ip, "fiches", 3, 86_400_000)) return json(sortie, JSONObject().put("ok", true).put("points", 0))
        synchronized(verrouEchanges) { val profils = lireObjet(fichierProfils); ajouterPoints(profils, id, 2); ecrire(fichierProfils, profils) }
        json(sortie, JSONObject().put("ok", true).put("points", 2))
    }

    // ── Bataille des corrigés : sous chaque document, les corrigés des
    // étudiants (validés par l'éditeur), votés « utile » par la promo. ─────

    private val fichierCorriges get() = File(dossier, "corriges.json")
    private val fichierMercis get() = File(dossier, "mercis.json")

    private fun documentConnu(id: String): JSONObject? {
        val d = lireContenu().optJSONArray("documents") ?: return null
        for (i in 0 until d.length()) if (d.optJSONObject(i)?.optString("id") == id) return d.getJSONObject(i)
        return null
    }

    private fun deposerCorrige(sortie: OutputStream, r: Requete, ip: String) {
        if (!permis(ip, "corrige", 8, 3_600_000)) return json(sortie, JSONObject().put("erreur", "Trop d'envois : réessayez dans une heure."), 429)
        val doc = r.query["doc"] ?: ""
        if (!idValide(doc) || documentConnu(doc) == null) return json(sortie, JSONObject().put("erreur", "Document introuvable."), 404)
        val cid = java.util.UUID.randomUUID().toString().replace("-", "").take(20)
        val f = File(dossierFichiers, cid)
        val taille = recevoirVers(f, r, 40_000_000L) ?: return json(sortie, JSONObject().put("erreur", "Fichier trop gros (40 Mo au plus) ou téléphone plein."), 413)
        synchronized(verrouEchanges) {
            val t = lireTableau(fichierCorriges)
            t.put(JSONObject().put("cid", cid).put("doc", doc).put("auteurId", (r.query["pid"] ?: "").takeIf { ID_PROFIL.matches(it) } ?: "")
                .put("pseudo", pseudoPropre(r.query["pseudo"])).put("nom", nomPropre(r.query["nom"])).put("type", typePropre(r.query["type"]))
                .put("note", texte(r.query["note"], 120)).put("taille", taille).put("date", System.currentTimeMillis())
                .put("statut", "attente").put("votes", 0).put("votants", JSONArray()))
            ecrire(fichierCorriges, t)
        }
        json(sortie, JSONObject().put("ok", true))
    }

    private fun voterCorrige(sortie: OutputStream, r: Requete, cid: String, ip: String) {
        val pid = lireJson(r).optString("pid")
        synchronized(verrouEchanges) {
            val t = lireTableau(fichierCorriges)
            for (i in 0 until t.length()) {
                val o = t.getJSONObject(i)
                if (o.optString("cid") != cid || o.optString("statut") != "publie") continue
                val v = o.optJSONArray("votants") ?: JSONArray()
                val cles = (0 until v.length()).map { v.optString(it) }
                if (pid == o.optString("auteurId") && pid.isNotEmpty()) return json(sortie, JSONObject().put("erreur", "C'est votre corrigé 🙂"), 400)
                if ("ip:$ip" in cles || (pid.isNotEmpty() && pid in cles)) return json(sortie, JSONObject().put("ok", true).put("deja", true))
                v.put("ip:$ip"); if (ID_PROFIL.matches(pid)) v.put(pid)
                o.put("votants", v).put("votes", o.optInt("votes") + 1)
                ecrire(fichierCorriges, t)
                val profils = lireObjet(fichierProfils); ajouterPoints(profils, o.optString("auteurId"), 2); ecrire(fichierProfils, profils)
                return json(sortie, JSONObject().put("ok", true))
            }
        }
        json(sortie, JSONObject().put("erreur", "Corrigé introuvable."), 404)
    }

    private fun corrigesPublics(gestion: Boolean): JSONArray {
        val t = lireTableau(fichierCorriges)
        val r = JSONArray()
        for (i in 0 until t.length()) {
            val o = t.getJSONObject(i)
            if (!gestion && o.optString("statut") != "publie") continue
            val p = JSONObject(o.toString()); p.remove("votants"); p.remove("auteurId")
            r.put(p)
        }
        return r
    }

    private fun modererCorrige(cid: String, action: String): JSONObject {
        synchronized(verrouEchanges) {
            val t = lireTableau(fichierCorriges)
            val gardes = JSONArray()
            for (i in 0 until t.length()) {
                val o = t.getJSONObject(i)
                if (o.optString("cid") == cid) {
                    when (action) {
                        "publier" -> if (o.optString("statut") != "publie") {
                            o.put("statut", "publie")
                            val profils = lireObjet(fichierProfils)
                            ajouterPoints(profils, o.optString("auteurId"), 20); ajouterContribution(profils, o.optString("auteurId"))
                            ecrire(fichierProfils, profils)
                        }
                        "refuser", "supprimer" -> { File(dossierFichiers, cid).delete(); continue }
                    }
                }
                gardes.put(o)
            }
            ecrire(fichierCorriges, gardes)
        }
        return JSONObject().put("ok", true)
    }

    /** « 🙏 Merci » à l'auteur d'un document : un par téléphone. */
    private fun merci(sortie: OutputStream, r: Requete, doc: String, ip: String) {
        val d = documentConnu(doc) ?: return json(sortie, JSONObject().put("erreur", "Document introuvable."), 404)
        val pid = lireJson(r).optString("pid")
        val n: Int
        synchronized(verrouEchanges) {
            val m = lireObjet(fichierMercis)
            val l = m.optJSONArray(doc) ?: JSONArray()
            val cles = (0 until l.length()).map { l.optString(it) }
            if ("ip:$ip" !in cles && (pid.isEmpty() || pid !in cles)) {
                l.put("ip:$ip"); if (ID_PROFIL.matches(pid)) l.put(pid)
                m.put(doc, l); ecrire(fichierMercis, m)
                val auteur = d.optString("auteurId")
                if (auteur != pid) { val profils = lireObjet(fichierProfils); ajouterPoints(profils, auteur, 2); ecrire(fichierProfils, profils) }
            }
            n = (0 until l.length()).count { l.optString(it).startsWith("ip:") }
        }
        json(sortie, JSONObject().put("ok", true).put("n", n))
    }

    private fun mercisPublics(): JSONObject {
        val m = lireObjet(fichierMercis)
        val r = JSONObject()
        m.keys().forEach { k -> val l = m.optJSONArray(k) ?: JSONArray(); r.put(k, (0 until l.length()).count { l.optString(it).startsWith("ip:") }) }
        return r
    }

    // ── Entraide : les étudiants répondent aux questions, la promo vote ──

    private fun reponsesPubliques(q: JSONObject): JSONArray {
        val rs = q.optJSONArray("reponses") ?: return JSONArray()
        val r = JSONArray()
        for (k in 0 until rs.length()) rs.optJSONObject(k)?.let { o -> val p = JSONObject(o.toString()); p.remove("votants"); p.remove("auteurId"); p.remove("ip"); r.put(p) }
        return r
    }

    private fun repondreQuestion(sortie: OutputStream, r: Requete, id: String, ip: String) {
        if (!permis(ip, "repondre", 10, 3_600_000)) return json(sortie, JSONObject().put("erreur", "Trop de réponses : attendez un peu."), 429)
        val q = lireJson(r)
        val t = texte(q.optString("texte"), 600)
        if (t.length < 3) return json(sortie, JSONObject().put("erreur", "Réponse trop courte."), 400)
        if (!texteCorrect(t)) return json(sortie, JSONObject().put("erreur", "Message refusé : restez correct."), 400)
        if (NUMERO.containsMatchIn(t)) return json(sortie, JSONObject().put("erreur", "Pas de numéro de téléphone."), 400)
        val pid = q.optString("pid").takeIf { ID_PROFIL.matches(it) } ?: ""
        synchronized(verrouEchanges) {
            val tout = lireTableau(fichierQuestions)
            for (i in 0 until tout.length()) {
                val o = tout.getJSONObject(i)
                if (o.optString("id") != id) continue
                val rs = o.optJSONArray("reponses") ?: JSONArray()
                if (rs.length() >= 30) return json(sortie, JSONObject().put("erreur", "Assez de réponses ici."), 400)
                rs.put(JSONObject().put("rid", java.util.UUID.randomUUID().toString().replace("-", "").take(12)).put("texte", t)
                    .put("pseudo", pseudoPropre(q.optString("pseudo"))).put("auteurId", pid).put("date", System.currentTimeMillis())
                    .put("votes", 0).put("votants", JSONArray()))
                o.put("reponses", rs)
                ecrire(fichierQuestions, tout)
                // 2 points par réponse, 5 réponses récompensées par jour au plus.
                if (pid.isNotEmpty() && permis(ip, "repondre-points", 5, 86_400_000)) { val profils = lireObjet(fichierProfils); ajouterPoints(profils, pid, 2); ecrire(fichierProfils, profils) }
                return json(sortie, JSONObject().put("ok", true))
            }
        }
        json(sortie, JSONObject().put("erreur", "Question introuvable."), 404)
    }

    private fun voterReponse(sortie: OutputStream, r: Requete, id: String, rid: String, ip: String) {
        val pid = lireJson(r).optString("pid")
        synchronized(verrouEchanges) {
            val tout = lireTableau(fichierQuestions)
            for (i in 0 until tout.length()) {
                val o = tout.getJSONObject(i)
                if (o.optString("id") != id) continue
                val rs = o.optJSONArray("reponses") ?: break
                for (k in 0 until rs.length()) {
                    val x = rs.optJSONObject(k) ?: continue
                    if (x.optString("rid") != rid) continue
                    if (pid.isNotEmpty() && pid == x.optString("auteurId")) return json(sortie, JSONObject().put("erreur", "C'est votre réponse 🙂"), 400)
                    val v = x.optJSONArray("votants") ?: JSONArray()
                    val cles = (0 until v.length()).map { v.optString(it) }
                    if ("ip:$ip" in cles || (pid.isNotEmpty() && pid in cles)) return json(sortie, JSONObject().put("ok", true).put("deja", true))
                    v.put("ip:$ip"); if (ID_PROFIL.matches(pid)) v.put(pid)
                    x.put("votants", v).put("votes", x.optInt("votes") + 1)
                    ecrire(fichierQuestions, tout)
                    val profils = lireObjet(fichierProfils)
                    ajouterPoints(profils, x.optString("auteurId"), 3)
                    if (x.optInt("votes") == 3) ajouterContribution(profils, x.optString("auteurId"))
                    ecrire(fichierProfils, profils)
                    return json(sortie, JSONObject().put("ok", true))
                }
            }
        }
        json(sortie, JSONObject().put("erreur", "Réponse introuvable."), 404)
    }

    /** Le délégué remplace le défi du jour par le sien (aujourd'hui seulement). */
    private fun defiPerso(q: JSONObject): JSONObject {
        val question = texte(q.optString("question"), 400)
        val choix = q.optJSONArray("choix") ?: JSONArray()
        val propres = JSONArray()
        for (i in 0 until minOf(4, choix.length())) texte(choix.optString(i), 160).takeIf { it.isNotEmpty() }?.let { propres.put(it) }
        val bonne = q.optInt("bonne", -1)
        if (question.length < 5 || propres.length() < 2 || bonne !in 0 until propres.length()) return JSONObject().put("erreur", "Une question, au moins deux réponses et la bonne réponse.")
        synchronized(verrouEchanges) {
            ecrire(fichierDefiPerso, JSONObject().put("date", jourIso()).put("question", question).put("choix", propres).put("bonne", bonne)
                .put("explication", texte(q.optString("explication"), 600)).put("ue", texte(q.optString("ue"), 20)))
            // Les réponses déjà données portaient sur l'autre défi : on repart de zéro.
            fichierDefiJour.delete()
        }
        return JSONObject().put("ok", true)
    }

    private fun defiGestion(): JSONObject {
        val d = defiDuJour()
        val e = etatDefi()
        return JSONObject(d.toString()).put("participants", e.optJSONObject("reponses")?.length() ?: 0)
            .put("trouve", e.optJSONArray("gagnants")?.length() ?: 0)
            .put("perso", lireObjet(fichierDefiPerso).optString("date") == jourIso())
    }

    // ── Star du jour : celui qui fête son anniversaire publie ses photos ;
    // la promo lui souhaite (phrases toutes faites : rien à modérer). ─────

    private val fichierStars get() = File(dossier, "stars.json")
    private val fichierParticipations get() = File(dossier, "participations.json")
    private val NB_VOEUX = 8

    private fun nouvelId16() = java.util.UUID.randomUUID().toString().replace("-", "").take(16)

    private fun creerStar(sortie: OutputStream, r: Requete, ip: String) {
        if (!permis(ip, "star", 3, 3_600_000)) return json(sortie, JSONObject().put("erreur", "Trop d'envois : réessayez dans une heure."), 429)
        val q = lireJson(r)
        val nom = texte(q.optString("nom"), 30)
        val message = texte(q.optString("message"), 200)
        if (nom.length < 2) return json(sortie, JSONObject().put("erreur", "Écrivez le prénom de la star."), 400)
        if (!texteCorrect("$nom $message")) return json(sortie, JSONObject().put("erreur", "Message refusé : restez correct."), 400)
        if (NUMERO.containsMatchIn("$nom $message")) return json(sortie, JSONObject().put("erreur", "Pas de numéro de téléphone."), 400)
        val id = nouvelId16()
        val jeton = nouvelId16()
        synchronized(verrouEchanges) {
            val t = lireTableau(fichierStars)
            val gardees = JSONArray()
            val limite = System.currentTimeMillis() - 8L * 86_400_000
            for (i in 0 until t.length()) {
                val o = t.getJSONObject(i)
                if (o.optLong("date") < limite) { supprimerPhotos(o); continue }
                gardees.put(o)
            }
            gardees.put(JSONObject().put("id", id).put("nom", nom).put("message", message)
                .put("filiere", texte(q.optString("filiere"), 40)).put("date", System.currentTimeMillis())
                .put("statut", "attente").put("photos", JSONArray()).put("voeux", JSONArray()).put("nbVoeux", 0)
                .put("jeton", jeton).put("ip", ip))
            ecrire(fichierStars, gardees)
        }
        json(sortie, JSONObject().put("ok", true).put("id", id).put("jeton", jeton))
    }

    private fun supprimerPhotos(o: JSONObject) {
        o.optJSONArray("photos")?.let { ph -> for (k in 0 until ph.length()) File(dossierFichiers, ph.optString(k)).takeIf { idValide(it.name) }?.delete() }
    }

    private fun photoStar(sortie: OutputStream, r: Requete, id: String, ip: String) {
        if (!permis(ip, "star-photo", 24, 3_600_000)) return json(sortie, JSONObject().put("erreur", "Trop de photos : réessayez plus tard."), 429)
        val jeton = r.query["jeton"] ?: ""
        val n = synchronized(verrouEchanges) {
            val o = trouverStar(id) ?: return json(sortie, JSONObject().put("erreur", "Star introuvable."), 404)
            if (jeton.isEmpty() || o.optString("jeton") != jeton) return json(sortie, JSONObject().put("erreur", "Refusé."), 403)
            val ph = o.optJSONArray("photos") ?: JSONArray()
            if (ph.length() >= 8) return json(sortie, JSONObject().put("erreur", "8 photos au plus."), 400)
            ph.length()
        }
        val f = File(dossierFichiers, "${id}s$n${(100..999).random()}")
        recevoirVers(f, r, 6_000_000L) ?: return json(sortie, JSONObject().put("erreur", "Photo trop lourde (6 Mo au plus)."), 413)
        synchronized(verrouEchanges) {
            val t = lireTableau(fichierStars)
            for (i in 0 until t.length()) {
                val o = t.getJSONObject(i)
                if (o.optString("id") == id) o.put("photos", (o.optJSONArray("photos") ?: JSONArray()).put(f.name))
            }
            ecrire(fichierStars, t)
        }
        json(sortie, JSONObject().put("ok", true).put("photo", f.name))
    }

    private fun trouverStar(id: String): JSONObject? {
        val t = lireTableau(fichierStars)
        for (i in 0 until t.length()) if (t.getJSONObject(i).optString("id") == id) return t.getJSONObject(i)
        return null
    }

    private fun voeuStar(sortie: OutputStream, r: Requete, id: String, ip: String) {
        val q = lireJson(r)
        val phrase = q.optInt("phrase", -1)
        if (phrase !in 0 until NB_VOEUX) return json(sortie, JSONObject().put("erreur", "Vœu inconnu."), 400)
        var de = texte(q.optString("de"), 20)
        if (!texteCorrect(de) || NUMERO.containsMatchIn(de)) de = ""
        if (!permis(ip, "voeu-$id", 2, 86_400_000)) return json(sortie, JSONObject().put("erreur", "Vous avez déjà souhaité à cette star 🎉"), 429)
        synchronized(verrouEchanges) {
            val t = lireTableau(fichierStars)
            for (i in 0 until t.length()) {
                val o = t.getJSONObject(i)
                if (o.optString("id") != id || o.optString("statut") != "publie") continue
                val v = o.optJSONArray("voeux") ?: JSONArray()
                v.put(JSONObject().put("p", phrase).put("de", de).put("date", System.currentTimeMillis()))
                while (v.length() > 300) v.remove(0)
                o.put("voeux", v).put("nbVoeux", o.optInt("nbVoeux") + 1)
            }
            ecrire(fichierStars, t)
        }
        json(sortie, JSONObject().put("ok", true))
    }

    /** Stars validées des 3 derniers jours (la page met en avant celles du jour). */
    private fun starsPubliques(): JSONArray {
        val t = lireTableau(fichierStars)
        val limite = System.currentTimeMillis() - 3L * 86_400_000
        val r = JSONArray()
        for (i in 0 until t.length()) {
            val o = t.getJSONObject(i)
            if (o.optString("statut") != "publie" || o.optLong("date") < limite) continue
            val p = JSONObject(o.toString()); p.remove("ip"); p.remove("jeton")
            val v = p.optJSONArray("voeux") ?: JSONArray()
            val derniers = JSONArray()
            for (k in maxOf(0, v.length() - 40) until v.length()) derniers.put(v.get(k))
            p.put("voeux", derniers)
            r.put(p)
        }
        return r
    }

    private fun starsGestion(): JSONArray {
        val t = lireTableau(fichierStars)
        val r = JSONArray()
        for (i in 0 until t.length()) { val p = JSONObject(t.getJSONObject(i).toString()); p.remove("ip"); p.remove("jeton"); p.remove("voeux"); r.put(p) }
        return r
    }

    private fun modererStar(id: String, action: String): JSONObject {
        synchronized(verrouEchanges) {
            val t = lireTableau(fichierStars)
            val gardees = JSONArray()
            for (i in 0 until t.length()) {
                val o = t.getJSONObject(i)
                if (o.optString("id") == id) {
                    when (action) {
                        "publier" -> o.put("statut", "publie")
                        "refuser", "supprimer" -> { supprimerPhotos(o); continue }
                    }
                }
                gardees.put(o)
            }
            ecrire(fichierStars, gardees)
        }
        return JSONObject().put("ok", true)
    }

    /** « J'y serai » aux événements de la promo (un par téléphone, touche à nouveau pour annuler). */
    private fun participer(sortie: OutputStream, id: String, ip: String) {
        if (!permis(ip, "viens", 30, 3_600_000)) return json(sortie, JSONObject().put("erreur", "Attendez un peu."), 429)
        val moi: Boolean
        synchronized(verrouEchanges) {
            val p = lireObjet(fichierParticipations)
            val liste = p.optJSONArray(id) ?: JSONArray()
            val gardee = JSONArray()
            var etait = false
            for (k in 0 until liste.length()) if (liste.optString(k) == ip) etait = true else gardee.put(liste.optString(k))
            if (!etait) gardee.put(ip)
            moi = !etait
            p.put(id, gardee)
            ecrire(fichierParticipations, p)
        }
        json(sortie, JSONObject().put("ok", true).put("moi", moi))
    }

    private fun participations(ip: String): JSONObject {
        val p = lireObjet(fichierParticipations)
        val r = JSONObject()
        p.keys().forEach { k ->
            val l = p.optJSONArray(k) ?: JSONArray()
            r.put(k, JSONObject().put("n", l.length()).put("moi", ip.isNotEmpty() && (0 until l.length()).any { l.optString(it) == ip }))
        }
        return r
    }

    // ── Alerte cours : signalée par les étudiants, confirmée par les +1 ──

    private val SORTES_ALERTE = setOf("present", "annule", "salle", "retard")

    private fun alerter(sortie: OutputStream, r: Requete, ip: String) {
        if (!permis(ip, "alerte", 4, 600_000)) return json(sortie, JSONObject().put("erreur", "Attendez quelques minutes."), 429)
        val q = lireJson(r)
        val sorte = q.optString("sorte").takeIf { it in SORTES_ALERTE } ?: return json(sortie, JSONObject().put("erreur", "Alerte inconnue."), 400)
        val precision = texte(q.optString("texte"), 120)
        if (precision.isNotEmpty() && !texteCorrect(precision)) return json(sortie, JSONObject().put("erreur", "Message refusé : restez correct."), 400)
        val filiere = texte(q.optString("filiere"), 40)
        synchronized(verrouEchanges) {
            val t = alertesValides()
            // Même alerte déjà signalée : on la confirme au lieu d'en créer une autre.
            for (i in 0 until t.length()) {
                val o = t.getJSONObject(i)
                if (o.optString("sorte") == sorte && o.optString("filiere") == filiere) {
                    val ips = o.optJSONArray("ips") ?: JSONArray()
                    if ((0 until ips.length()).none { ips.optString(it) == ip }) ips.put(ip)
                    o.put("ips", ips)
                    if (precision.isNotEmpty()) o.put("texte", precision)
                    ecrire(fichierAlertes, t)
                    return json(sortie, JSONObject().put("ok", true))
                }
            }
            t.put(JSONObject().put("id", java.util.UUID.randomUUID().toString().replace("-", "").take(12)).put("sorte", sorte)
                .put("texte", precision).put("filiere", filiere).put("date", System.currentTimeMillis()).put("ips", JSONArray().put(ip)))
            ecrire(fichierAlertes, t)
        }
        json(sortie, JSONObject().put("ok", true))
    }

    /** Alertes de moins de 3 heures. */
    private fun alertesValides(): JSONArray {
        val t = lireTableau(fichierAlertes)
        val limite = System.currentTimeMillis() - 3 * 3_600_000L
        val r = JSONArray()
        for (i in 0 until t.length()) t.optJSONObject(i)?.let { if (it.optLong("date") > limite) r.put(it) }
        return r
    }

    private fun confirmerAlerte(sortie: OutputStream, id: String, ip: String) {
        synchronized(verrouEchanges) {
            val t = alertesValides()
            for (i in 0 until t.length()) {
                val o = t.getJSONObject(i)
                if (o.optString("id") != id) continue
                val ips = o.optJSONArray("ips") ?: JSONArray()
                if ((0 until ips.length()).none { ips.optString(it) == ip }) { ips.put(ip); o.put("ips", ips); ecrire(fichierAlertes, t) }
            }
        }
        json(sortie, JSONObject().put("ok", true))
    }

    private fun alertesActives(ip: String): JSONArray {
        val t = alertesValides()
        val r = JSONArray()
        for (i in 0 until t.length()) {
            val o = t.getJSONObject(i)
            val ips = o.optJSONArray("ips") ?: JSONArray()
            r.put(JSONObject().put("id", o.optString("id")).put("sorte", o.optString("sorte")).put("texte", o.optString("texte"))
                .put("filiere", o.optString("filiere")).put("date", o.optLong("date")).put("confirmations", ips.length())
                .put("officielle", o.optBoolean("officielle")).put("moi", ip.isNotEmpty() && (0 until ips.length()).any { ips.optString(it) == ip }))
        }
        return r
    }

    private fun modererAlerte(id: String, action: String): JSONObject {
        synchronized(verrouEchanges) {
            val t = alertesValides()
            val gardees = JSONArray()
            for (i in 0 until t.length()) {
                val o = t.getJSONObject(i)
                if (o.optString("id") == id) {
                    if (action == "supprimer") continue
                    if (action == "officielle") o.put("officielle", true)
                }
                gardees.put(o)
            }
            ecrire(fichierAlertes, gardees)
        }
        return JSONObject().put("ok", true)
    }

    // ── Mot du jour et devinette du jour : réserve fournie, choix automatique ──

    private fun numeroDuJour(decalage: Int = 0): Int {
        val maintenant = java.util.Calendar.getInstance()
        val jours = (maintenant.timeInMillis + maintenant.get(java.util.Calendar.ZONE_OFFSET) + maintenant.get(java.util.Calendar.DST_OFFSET)) / 86_400_000L
        return (jours + decalage).toInt()
    }

    private fun reserve(nom: String): JSONArray = try { JSONArray(assets.open("amphi/$nom").use { String(it.readBytes(), Charsets.UTF_8) }) } catch (_: Exception) { JSONArray() }

    /** L'éditeur peut écrire le sien pour aujourd'hui ; sinon la réserve. */
    private fun motDuJour(): JSONObject {
        val perso = lireContenu().optJSONObject("motDuJour")
        if (perso != null && perso.optString("date") == aujourdhui() && perso.optString("texte").isNotEmpty()) return perso
        val r = reserve("mots.json")
        return if (r.length() == 0) JSONObject() else r.getJSONObject(numeroDuJour().mod(r.length()))
    }

    private fun devinetteDuJour(decalage: Int = 0): JSONObject {
        if (decalage == 0) {
            val perso = lireContenu().optJSONObject("devinette")
            if (perso != null && perso.optString("date") == aujourdhui() && perso.optString("question").isNotEmpty()) return perso
        }
        val r = reserve("devinettes.json")
        return if (r.length() == 0) JSONObject() else r.getJSONObject(numeroDuJour(decalage).mod(r.length()))
    }

    private fun normaliser(t: String): String = java.text.Normalizer.normalize(t.lowercase(Locale.FRANCE), java.text.Normalizer.Form.NFD)
        .replace(Regex("\\p{M}+"), "").replace(Regex("[^a-z0-9]+"), " ").trim()

    private val ARTICLES = setOf("le", "la", "les", "l", "un", "une", "des", "du", "de", "d", "c", "est", "ce", "mon", "ma", "ton", "ta", "son", "sa")

    /** Juste si tous les mots d'une réponse acceptée sont dans la réponse donnée. */
    private fun bonneReponse(donnee: String, acceptees: JSONArray?): Boolean {
        val mots = normaliser(donnee).split(" ").filter { it.isNotEmpty() && it !in ARTICLES }.toSet()
        if (mots.isEmpty() || acceptees == null) return false
        for (i in 0 until acceptees.length()) {
            val attendus = normaliser(acceptees.optString(i)).split(" ").filter { it.isNotEmpty() && it !in ARTICLES }
            if (attendus.isNotEmpty() && attendus.all { it in mots }) return true
        }
        return false
    }

    private fun etatDevinette(): JSONObject {
        val e = lireObjet(fichierDevinette)
        return if (e.optString("jour") == aujourdhui()) e else JSONObject().put("jour", aujourdhui()).put("gagnants", JSONArray()).put("trouve", JSONArray()).put("essais", JSONObject())
    }

    private fun devinettePublique(ip: String): JSONObject {
        val d = devinetteDuJour()
        val e = etatDevinette()
        val trouve = e.optJSONArray("trouve") ?: JSONArray()
        val hier = devinetteDuJour(-1)
        return JSONObject().put("question", d.optString("question"))
            .put("gagnants", e.optJSONArray("gagnants") ?: JSONArray())
            .put("trouve", ip.isNotEmpty() && (0 until trouve.length()).any { trouve.optString(it) == ip })
            .put("hier", JSONObject().put("question", hier.optString("question")).put("reponse", hier.optJSONArray("reponses")?.optString(0) ?: ""))
    }

    private fun repondreDevinette(sortie: OutputStream, r: Requete, ip: String) {
        val q = lireJson(r)
        val reponse = texte(q.optString("reponse"), 80)
        val pseudo = texte(q.optString("pseudo"), 20).ifEmpty { "Anonyme" }
        if (!texteCorrect(pseudo)) return json(sortie, JSONObject().put("erreur", "Choisissez un autre pseudo."), 400)
        synchronized(verrouEchanges) {
            val e = etatDevinette()
            val trouve = e.optJSONArray("trouve") ?: JSONArray()
            if ((0 until trouve.length()).any { trouve.optString(it) == ip }) return json(sortie, JSONObject().put("juste", true).put("deja", true))
            val essais = e.optJSONObject("essais") ?: JSONObject()
            val n = essais.optInt(ip)
            if (n >= 15) return json(sortie, JSONObject().put("erreur", "Plus d'essais aujourd'hui : revenez demain !"), 429)
            essais.put(ip, n + 1)
            e.put("essais", essais)
            val juste = bonneReponse(reponse, devinetteDuJour().optJSONArray("reponses"))
            var rang = 0
            if (juste) {
                trouve.put(ip)
                e.put("trouve", trouve)
                val gagnants = e.optJSONArray("gagnants") ?: JSONArray()
                rang = gagnants.length() + 1
                if (gagnants.length() < 20) gagnants.put(JSONObject().put("pseudo", pseudo).put("heure", System.currentTimeMillis()))
                e.put("gagnants", gagnants)
            }
            ecrire(fichierDevinette, e)
            return json(sortie, JSONObject().put("juste", juste).put("rang", rang).put("essaisRestants", 15 - n - 1))
        }
    }

    // ── Le Wi-Fi de l'amphi, créé par le téléphone (Wi-Fi Direct) ─────────
    //
    // Adresse toujours la même (192.168.49.1) : elle s'écrit une fois pour
    // toutes au tableau ou sur une affiche. Le téléphone garde ses données
    // mobiles pour lui ; les étudiants n'ont que la page.

    private fun prefs() = getSharedPreferences(PREFS, 0)
    private var p2p: WifiP2pManager? = null
    private var canalP2p: WifiP2pManager.Channel? = null
    @Volatile private var veille = true

    private fun lireJson(r: Requete): JSONObject {
        val taille = (r.entetes["content-length"]?.toLongOrNull() ?: 0).coerceIn(0, 10_000).toInt()
        val octets = ByteArray(taille)
        var lus = 0
        while (lus < taille) { val n = r.corps.read(octets, lus, taille - lus); if (n < 0) break; lus += n }
        return try { JSONObject(String(octets, 0, lus, Charsets.UTF_8)) } catch (_: Exception) { JSONObject() }
    }

    /** « DIRECT-AM-Physique L3 » : préfixe exigé par Android, 32 octets au plus. */
    private fun nomDuReseau(nom: String): String {
        val simple = java.text.Normalizer.normalize(nom, java.text.Normalizer.Form.NFD)
            .replace(Regex("\\p{M}+"), "")
            .replace(Regex("[^A-Za-z0-9 ._-]"), "")
            .replace(Regex("\\s+"), " ")
            .trim()
            .ifEmpty { "Amphi" }
        return ("DIRECT-AM-$simple").take(32).trim()
    }

    private fun autorise(): Boolean =
        if (Build.VERSION.SDK_INT >= 33) checkSelfPermission(Manifest.permission.NEARBY_WIFI_DEVICES) == PackageManager.PERMISSION_GRANTED
        else checkSelfPermission(Manifest.permission.ACCESS_FINE_LOCATION) == PackageManager.PERMISSION_GRANTED

    private fun gestionnaire(): WifiP2pManager? {
        if (p2p == null) {
            p2p = getSystemService(WifiP2pManager::class.java)
            canalP2p = p2p?.initialize(this, mainLooper, null)
        }
        return p2p
    }

    private fun attendre(lancer: (WifiP2pManager.ActionListener) -> Unit): Int? {
        val resultat = AtomicReference<Int?>(null)
        val fini = CountDownLatch(1)
        try {
            lancer(object : WifiP2pManager.ActionListener {
                override fun onSuccess() { resultat.set(-1); fini.countDown() }
                override fun onFailure(raison: Int) { resultat.set(raison); fini.countDown() }
            })
            fini.await(10, TimeUnit.SECONDS)
        } catch (_: Exception) {}
        return resultat.get()
    }

    private fun supprimerGroupe() {
        val m = p2p ?: return
        val c = canalP2p ?: return
        try { attendre { m.removeGroup(c, it) } } catch (_: Exception) {}
    }

    @SuppressLint("MissingPermission")
    private fun groupeActuel(): WifiP2pGroup? {
        val m = gestionnaire() ?: return null
        val c = canalP2p ?: return null
        val g = AtomicReference<WifiP2pGroup?>(null)
        val fini = CountDownLatch(1)
        try {
            m.requestGroupInfo(c) { groupe -> g.set(groupe); fini.countDown() }
            fini.await(5, TimeUnit.SECONDS)
        } catch (_: Exception) {}
        return g.get()
    }

    private val verrouCreation = Any()

    /** Un seul essai à la fois ; la page des étudiants n'attend jamais après. */
    private fun creerWifi(nomVoulu: String, mdpVoulu: String): JSONObject = synchronized(verrouCreation) { creerWifiMaintenant(nomVoulu, mdpVoulu) }

    @SuppressLint("MissingPermission")
    private fun creerWifiMaintenant(nomVoulu: String, mdpVoulu: String): JSONObject {
        val wifi = applicationContext.getSystemService(WifiManager::class.java)
        if (wifi != null && !wifi.isWifiEnabled) return JSONObject().put("erreur", "wifi_eteint")
        if (!autorise()) return JSONObject().put("erreur", "autorisation")
        val m = gestionnaire() ?: return JSONObject().put("erreur", "Ce téléphone ne sait pas créer de Wi-Fi Direct.")
        val canal = canalP2p ?: return JSONObject().put("erreur", "Ce téléphone ne sait pas créer de Wi-Fi Direct.")
        val nomRetenu = nomVoulu.ifBlank { prefs().getString("wifi_nom", "") ?: "" }
        val nom = nomDuReseau(nomRetenu)
        val mdp = mdpVoulu.ifBlank { prefs().getString("wifi_mdp", "") ?: "" }.trim()
        if (mdp.length < 8 || mdp.length > 63) return JSONObject().put("erreur", "Le mot de passe doit avoir au moins 8 caractères.")
        supprimerGroupe()
        val config = WifiP2pConfig.Builder()
            .setNetworkName(nom)
            .setPassphrase(mdp)
            .setGroupOperatingBand(WifiP2pConfig.GROUP_OWNER_BAND_2GHZ)
            .build()
        var raison: Int? = null
        for (essai in 0..1) {
            raison = attendre { m.createGroup(canal, config, it) }
            if (raison == -1) break
            supprimerGroupe()
            Thread.sleep(2000)
        }
        if (raison != -1) {
            val pourquoi = when (raison) {
                WifiP2pManager.BUSY -> "Le Wi-Fi est occupé : éteignez le point d'accès mobile, puis réessayez."
                WifiP2pManager.P2P_UNSUPPORTED -> "Ce téléphone ne sait pas créer de Wi-Fi Direct."
                else -> "Le téléphone a refusé de créer le Wi-Fi. Éteignez puis rallumez le Wi-Fi, et réessayez."
            }
            return JSONObject().put("erreur", pourquoi)
        }
        prefs().edit().putBoolean("wifi", true).putString("wifi_nom", nomRetenu).putString("wifi_mdp", mdp).apply()
        // L'adresse arrive une ou deux secondes après la création.
        var adresse = ""
        for (i in 0 until 10) {
            adresse = adressesWifiDirect().firstOrNull() ?: ""
            if (adresse.isNotEmpty()) break
            Thread.sleep(500)
        }
        return JSONObject().put("ok", true).put("ssid", nom).put("motDePasse", mdp).put("adresse", adresse.ifEmpty { "192.168.49.1" } + ":" + PORT)
    }

    private fun adressesWifiDirect(): List<String> = try {
        NetworkInterface.getNetworkInterfaces().toList()
            .filter { it.isUp && it.name.startsWith("p2p") }
            .flatMap { i -> i.inetAddresses.toList().filterIsInstance<Inet4Address>().mapNotNull { it.hostAddress } }
    } catch (_: Exception) { emptyList() }

    @Volatile private var dernierMainAMain = 0L

    /** Recrée le Wi-Fi s'il devait être allumé et ne l'est plus. */
    private fun veillerWifi() {
        try { Thread.sleep(3000) } catch (_: InterruptedException) { return }
        while (veille) {
            try {
                // Un transfert « Main à main » utilise aussi le Wi-Fi Direct : on
                // ne touche à rien pendant qu'il tourne, ni dans les 2 minutes après.
                if (MainAMain.enCours()) dernierMainAMain = System.currentTimeMillis()
                val libre = !MainAMain.enCours() && System.currentTimeMillis() - dernierMainAMain > 120_000
                if (libre && prefs().getBoolean("wifi", false) && autorise()) {
                    val g = groupeActuel()
                    if (g == null || !g.isGroupOwner) creerWifi("", "")
                }
            } catch (_: Exception) {}
            var attente = 0
            while (veille && attente < 60) {
                try { Thread.sleep(1000) } catch (_: InterruptedException) { return }
                attente++
            }
        }
    }

    // ── Relais : ce téléphone recopie le téléphone principal, puis crée son
    // propre Wi-Fi. Un téléphone n'accueille qu'une dizaine d'étudiants : avec
    // des relais répartis dans l'amphi, toute la promo a les documents. ────

    private val etatRelais = AtomicReference(JSONObject().put("enCours", false))

    /** Le Wi-Fi auquel ce téléphone est connecté (celui du téléphone principal). */
    private fun reseauWifi(): Network? {
        val cm = getSystemService(ConnectivityManager::class.java) ?: return null
        @Suppress("DEPRECATION")
        return cm.allNetworks.firstOrNull { cm.getNetworkCapabilities(it)?.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) == true }
    }

    /** Adresse du téléphone principal : la passerelle de ce Wi-Fi (192.168.49.1 en Wi-Fi Direct). */
    private fun principal(n: Network): String {
        val cm = getSystemService(ConnectivityManager::class.java)
        val gw = cm?.getLinkProperties(n)?.routes
            ?.firstOrNull { it.destination.prefixLength == 0 && it.gateway is Inet4Address }
            ?.gateway?.hostAddress
        return gw ?: "192.168.49.1"
    }

    private fun ouvrir(n: Network, url: String): HttpURLConnection =
        (n.openConnection(java.net.URL(url)) as HttpURLConnection).apply { connectTimeout = 8_000; readTimeout = 30_000 }

    private fun lancerCopie(): JSONObject {
        synchronized(etatRelais) {
            if (etatRelais.get().optBoolean("enCours")) return etatRelais.get()
            // Son propre Wi-Fi allumé, ce téléphone serait lui-même 192.168.49.1.
            if (prefs().getBoolean("wifi", false) || adressesWifiDirect().isNotEmpty()) return JSONObject().put("erreur", "wifi_propre")
            val n = reseauWifi() ?: return JSONObject().put("erreur", "pas_connecte")
            etatRelais.set(JSONObject().put("enCours", true).put("fait", 0).put("total", 0).put("etape", "Lecture du contenu…"))
            Thread({ copier(n) }, "amphi-relais").start()
            return etatRelais.get()
        }
    }

    private fun copier(n: Network) {
        val avant = prefs().getLong("relais_date", 0L)
        try {
            val hote = principal(n)
            val base = "http://$hote:$PORT"
            val c = ouvrir(n, "$base/donnees.json").run {
                try { if (responseCode != 200) throw Exception("code $responseCode"); inputStream.use { JSONObject(String(it.readBytes(), Charsets.UTF_8)) } } finally { disconnect() }
            }
            // Tous les fichiers dont la page a besoin : documents, images des publicités, des annonces, photo.
            val besoins = LinkedHashMap<String, Long>()
            fun ajouter(t: JSONArray?, cle: String, cleTaille: String?) {
                if (t == null) return
                for (i in 0 until t.length()) {
                    val o = t.optJSONObject(i) ?: continue
                    val id = o.optString(cle)
                    if (idValide(id)) besoins[id] = if (cleTaille != null) o.optLong(cleTaille, -1L) else -1L
                }
            }
            ajouter(c.optJSONArray("documents"), "id", "taille")
            ajouter(c.optJSONArray("pubs"), "image", null)
            ajouter(c.optJSONArray("annonces"), "image", null)
            ajouter(c.optJSONArray("evenements"), "image", null)
            c.optJSONObject("accueil")?.optString("photo")?.takeIf { idValide(it) }?.let { besoins[it] = -1L }
            var fait = 0
            var octets = 0L
            for ((id, taille) in besoins) {
                etatRelais.set(JSONObject().put("enCours", true).put("fait", fait).put("total", besoins.size).put("octets", octets).put("etape", "Copie des documents…"))
                val f = File(dossierFichiers, id)
                if (f.isFile && (taille < 0 || f.length() == taille)) { fait++; continue }
                if (taille > 0 && dossierFichiers.usableSpace < taille + 200_000_000L) throw Exception("place")
                val tmp = File(dossierFichiers, "$id.part")
                val cx = ouvrir(n, "$base/f/$id?relais=1")
                try {
                    if (cx.responseCode != 200) { fait++; continue }
                    cx.inputStream.use { i -> tmp.outputStream().use { o -> octets += i.copyTo(o, 64 * 1024) } }
                } finally { cx.disconnect() }
                tmp.renameTo(f)
                fait++
            }
            val maintenant = System.currentTimeMillis()
            c.put("relais", JSONObject().put("date", maintenant))
            synchronized(this) {
                val tmp = File(dossier, "contenu.json.tmp")
                tmp.writeText(c.toString())
                tmp.renameTo(fichierContenu)
            }
            prefs().edit().putLong("relais_date", maintenant).apply()
            dossierFichiers.listFiles()?.filter { it.name.endsWith(".part") }?.forEach { it.delete() }
            nettoyerFichiers(c)
            etatRelais.set(JSONObject().put("enCours", false).put("ok", true).put("fait", fait).put("total", besoins.size).put("octets", octets).put("date", maintenant))
        } catch (e: Exception) {
            dossierFichiers.listFiles()?.filter { it.name.endsWith(".part") }?.forEach { it.delete() }
            val raison = when {
                e.message == "place" -> "place"
                e is java.net.SocketTimeoutException || e is java.net.ConnectException || e is java.net.NoRouteToHostException -> "injoignable"
                else -> "echec"
            }
            etatRelais.set(JSONObject().put("enCours", false).put("erreur", raison).put("date", avant))
        }
    }

    private fun etat(): JSONObject {
        val st = stats()
        val jour = aujourdhui()
        var visiteurs7 = 0
        var vues7 = 0
        val format = SimpleDateFormat("yyyy-MM-dd", Locale.FRANCE)
        val limite = format.format(Date(System.currentTimeMillis() - 6L * 86_400_000))
        st.keys().forEach { k ->
            if (k >= limite) {
                visiteurs7 += st.optJSONObject(k)?.optInt("visiteurs") ?: 0
                vues7 += st.optJSONObject(k)?.optInt("vues") ?: 0
            }
        }
        return JSONObject()
            .put("enMarche", enMarche)
            .put("port", PORT)
            .put("adresses", JSONArray(adresses()))
            .put("visiteursAujourdhui", st.optJSONObject(jour)?.optInt("visiteurs") ?: 0)
            .put("vuesAujourdhui", st.optJSONObject(jour)?.optInt("vues") ?: 0)
            .put("visiteurs7jours", visiteurs7)
            .put("vues7jours", vues7)
            .put("placeLibre", dossierFichiers.usableSpace)
            .put("compteurs", compteurs())
            .put("aValider", lireTableau(fichierPropositions).length())
            .put("wifi", JSONObject()
                .put("actif", prefs().getBoolean("wifi", false) && adressesWifiDirect().isNotEmpty())
                .put("voulu", prefs().getBoolean("wifi", false))
                .put("ssid", nomDuReseau(prefs().getString("wifi_nom", "") ?: ""))
                .put("nom", prefs().getString("wifi_nom", "") ?: "")
                .put("motDePasse", prefs().getString("wifi_mdp", "") ?: "")
                .put("adresse", (adressesWifiDirect().firstOrNull() ?: "192.168.49.1") + ":" + PORT))
    }
}
