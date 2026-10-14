package bj.photocopie.envoyeur

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.wifi.WifiManager
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
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
import java.util.concurrent.Executors

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

        @Volatile var enMarche = false
            private set

        fun demarrer(ctx: Context) {
            ctx.startForegroundService(Intent(ctx, AmphiService::class.java))
        }

        fun arreter(ctx: Context) {
            ctx.stopService(Intent(ctx, AmphiService::class.java))
        }

        /** Adresses IPv4 du téléphone sur ses réseaux locaux (point d'accès compris). */
        fun adresses(): List<String> = try {
            NetworkInterface.getNetworkInterfaces().toList()
                .filter { it.isUp && !it.isLoopback }
                // Le point d'accès d'abord (swlan, ap, wlan), les données mobiles exclues.
                .filterNot { it.name.startsWith("rmnet") || it.name.startsWith("ccmni") || it.name.startsWith("dummy") || it.name.startsWith("tun") }
                .sortedBy { if (it.name.startsWith("swlan") || it.name.startsWith("ap")) 0 else 1 }
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

        chargerStats()
        Thread({ ecouter() }, "amphi-serveur").start()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int = START_STICKY

    override fun onDestroy() {
        enMarche = false
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

    private fun repondre(sortie: OutputStream, code: Int, type: String, corps: ByteArray, extra: Map<String, String> = emptyMap()) {
        val texte = when (code) { 200 -> "OK"; 206 -> "Partial Content"; 403 -> "Forbidden"; 404 -> "Not Found"; 413 -> "Payload Too Large"; 416 -> "Range Not Satisfiable"; 507 -> "Insufficient Storage"; else -> "Error" }
        val tete = StringBuilder("HTTP/1.1 $code $texte\r\nContent-Type: $type\r\nContent-Length: ${corps.size}\r\nConnection: close\r\nCache-Control: no-store\r\n")
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
                    r.methode == "GET" && r.chemin.startsWith("/f/") -> envoyerFichier(sortie, r, r.chemin.removePrefix("/f/"))
                    r.chemin.startsWith("/gerer") && !local -> repondre(sortie, 403, "text/plain; charset=utf-8", "Réservé au téléphone de l'éditeur.".toByteArray())
                    r.methode == "GET" && r.chemin == "/gerer" -> actif(sortie, "amphi/gerer.html", "text/html; charset=utf-8")
                    r.methode == "GET" && r.chemin == "/gerer/qrcode.js" -> actif(sortie, "amphi/qrcode.js", "text/javascript; charset=utf-8")
                    r.methode == "GET" && r.chemin == "/gerer/etat" -> json(sortie, etat())
                    r.methode == "GET" && r.chemin == "/gerer/contenu" -> json(sortie, lireContenu())
                    r.methode == "POST" && r.chemin == "/gerer/contenu" -> enregistrerContenu(sortie, r)
                    r.methode == "POST" && r.chemin == "/gerer/fichier" -> recevoirFichier(sortie, r)
                    else -> repondre(sortie, 404, "text/plain; charset=utf-8", "Introuvable".toByteArray())
                }
            } catch (_: Exception) {
            }
        }
    }

    private fun actif(sortie: OutputStream, chemin: String, type: String) {
        val octets = assets.open(chemin).use { it.readBytes() }
        repondre(sortie, 200, type, octets)
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

    /** Ce que voient les étudiants : le contenu, sans le texte brut collé. */
    private fun contenuPublic(): JSONObject {
        val c = lireContenu()
        c.optJSONObject("programme")?.remove("brut")
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
    }
}
