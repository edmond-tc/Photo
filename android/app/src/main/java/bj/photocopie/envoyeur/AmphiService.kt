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
                    r.methode == "GET" && (r.chemin == "/lib/pdf.min.js" || r.chemin == "/lib/pdf.worker.min.js") ->
                        actif(sortie, "amphi" + r.chemin, "text/javascript; charset=utf-8", "max-age=604800")
                    r.methode == "GET" && r.chemin.startsWith("/f/") -> envoyerFichier(sortie, r, r.chemin.removePrefix("/f/"))
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
        c.optJSONObject("accueil")?.optString("photo")?.takeIf { it.isNotEmpty() }?.let { utilises.add(it) }
        val maintenant = System.currentTimeMillis()
        dossierFichiers.listFiles()?.forEach { f ->
            // Un fichier tout juste reçu (pas encore enregistré dans le contenu) est gardé une heure.
            if (f.name !in utilises && maintenant - f.lastModified() > 3_600_000) f.delete()
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
        if (!f.exists()) return repondre(sortie, 404, "text/plain", "Introuvable".toByteArray())
        val type = r.query["type"]?.takeIf { it.matches(Regex("^[a-z]+/[a-z0-9.+-]+$")) } ?: "application/octet-stream"
        val nom = (r.query["nom"] ?: "document").replace(Regex("[\\r\\n\"\\\\]"), "_").take(150)
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
        if (debut == 0L) compter("telechargements", id)
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

    /** Recrée le Wi-Fi s'il devait être allumé et ne l'est plus. */
    private fun veillerWifi() {
        try { Thread.sleep(3000) } catch (_: InterruptedException) { return }
        while (veille) {
            try {
                if (prefs().getBoolean("wifi", false) && autorise()) {
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
            .put("wifi", JSONObject()
                .put("actif", prefs().getBoolean("wifi", false) && adressesWifiDirect().isNotEmpty())
                .put("voulu", prefs().getBoolean("wifi", false))
                .put("ssid", nomDuReseau(prefs().getString("wifi_nom", "") ?: ""))
                .put("nom", prefs().getString("wifi_nom", "") ?: "")
                .put("motDePasse", prefs().getString("wifi_mdp", "") ?: "")
                .put("adresse", (adressesWifiDirect().firstOrNull() ?: "192.168.49.1") + ":" + PORT))
    }
}
