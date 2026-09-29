package bj.photocopie.envoyeur

import android.annotation.SuppressLint
import android.bluetooth.BluetoothManager
import android.bluetooth.le.AdvertiseCallback
import android.bluetooth.le.AdvertiseData
import android.bluetooth.le.AdvertiseSettings
import android.bluetooth.le.ScanCallback
import android.bluetooth.le.ScanFilter
import android.bluetooth.le.ScanResult
import android.bluetooth.le.ScanSettings
import android.content.ContentValues
import android.content.Context
import android.content.pm.PackageManager
import android.net.Uri
import android.net.wifi.WifiManager
import android.net.wifi.p2p.WifiP2pConfig
import android.net.wifi.p2p.WifiP2pInfo
import android.net.wifi.p2p.WifiP2pManager
import android.os.Environment
import android.os.StatFs
import android.os.SystemClock
import android.provider.DocumentsContract
import android.provider.MediaStore
import android.provider.OpenableColumns
import org.json.JSONArray
import org.json.JSONObject
import java.io.FileInputStream
import java.io.FileOutputStream
import java.io.IOException
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.ServerSocket
import java.net.Socket
import java.net.SocketTimeoutException
import java.nio.ByteBuffer
import java.security.MessageDigest
import java.security.SecureRandom
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference

/**
 * « Main à main » : envoyer n'importe quel fichier d'un téléphone à un
 * autre, sans internet, sans QR, sans réseau à choisir.
 *
 * - Celui qui REÇOIT touche « Recevoir » : son téléphone émet une petite
 *   balise Bluetooth « KR » (son numéro de session, et s'il sait faire du
 *   Wi-Fi 5 GHz).
 * - Celui qui ENVOIE touche « Envoyer » : son téléphone écoute ces balises,
 *   garde le téléphone NETTEMENT le plus proche, crée un réseau Wi-Fi
 *   Direct « DIRECT-KM-<numéro> » (5 GHz si les deux savent) et émet à son
 *   tour une balise « KS » qui dit à qui l'envoi est destiné.
 * - Le receveur entend qu'on l'appelle, rejoint ce réseau tout seul
 *   (connexion Wi-Fi Direct rapide d'Android 10, identifiants passés par
 *   Bluetooth), et le transfert chiffré commence (voir [MamCanal]).
 *
 * Un téléphone qui ne sait pas émettre de balise peut quand même recevoir :
 * sans cible, l'envoi est « ouvert » au téléphone le plus proche.
 * Mode groupe : jusqu'à [GROUPE_MAX] receveurs, chacun servi en parallèle.
 *
 * Le mot de passe Wi-Fi se déduit du nom du réseau : il ne protège pas, il
 * sépare. La protection, c'est le chiffrement de bout en bout et le symbole
 * identique sur les deux écrans.
 */
object MainAMain {
    const val PORT = 48174
    const val PREFIXE_RESEAU = "DIRECT-KM-"
    private const val SECRET_RESEAU = "photocopie-benin/main-a-main/v1"
    private const val FABRICANT = 0xFFFF

    /** Deux téléphones côte à côte donnent bien plus ; à régler pendant l'essai. */
    private const val SEUIL_PROCHE = -65

    /** Le plus proche doit l'être nettement : sinon, on demande de se rapprocher. */
    private const val MARGE_PLUS_PROCHE = 8
    private const val SEUIL_GROUPE = -82
    const val GROUPE_MAX = 5
    private const val VU_RECEMMENT_MS = 3000L
    private const val ATTENTE_MAX_MS = 5 * 60 * 1000L
    private const val DOSSIER_RECUS = "Main a main"
    private const val MARGE_DISQUE = 50L * 1024 * 1024

    private const val DRAPEAU_5GHZ = 1
    private const val DRAPEAU_OUVERT = 2
    private const val DRAPEAU_GROUPE = 4

    /** Reçoit chaque événement pour l'interface (type « mam »). */
    @Volatile var ecouteur: ((JSONObject) -> Unit)? = null

    @Volatile private var session: Session? = null

    /** Fichiers reçus depuis l'ouverture de l'application : (adresse, type), pour « Ouvrir ». */
    val recus: MutableList<Pair<Uri, String>> = java.util.Collections.synchronizedList(mutableListOf())

    fun enCours(): Boolean = session != null

    /** Un fichier choisi pour l'envoi. `taille` -1 : inconnue (pas de reprise possible). */
    class FichierAEnvoyer(val uri: Uri, val nom: String, val taille: Long, val type: String, val modifie: Long)

    fun decrire(ctx: Context, uri: Uri): FichierAEnvoyer {
        var nom = uri.lastPathSegment?.substringAfterLast('/') ?: "fichier"
        var taille = -1L
        var modifie = 0L
        try {
            ctx.contentResolver.query(uri, null, null, null, null)?.use { c ->
                if (c.moveToFirst()) {
                    c.getColumnIndex(OpenableColumns.DISPLAY_NAME).takeIf { it >= 0 }
                        ?.let { c.getString(it) }?.takeIf { it.isNotBlank() }?.let { nom = it }
                    c.getColumnIndex(OpenableColumns.SIZE).takeIf { it >= 0 && !c.isNull(it) }
                        ?.let { taille = c.getLong(it) }
                    c.getColumnIndex(DocumentsContract.Document.COLUMN_LAST_MODIFIED).takeIf { it >= 0 && !c.isNull(it) }
                        ?.let { modifie = c.getLong(it) }
                }
            }
        } catch (_: Exception) {
        }
        val type = ctx.contentResolver.getType(uri) ?: "application/octet-stream"
        return FichierAEnvoyer(uri, nom.replace('/', '_').take(150), taille, type, modifie)
    }

    fun capacites(ctx: Context): JSONObject {
        val bt = ctx.getSystemService(BluetoothManager::class.java)?.adapter
        val wifi = ctx.applicationContext.getSystemService(WifiManager::class.java)
        val annonce = try { bt?.isMultipleAdvertisementSupported == true } catch (_: Exception) { false }
        return JSONObject()
            .put("bluetooth", bt != null)
            .put("annonce", annonce)
            .put("wifiDirect", ctx.packageManager.hasSystemFeature(PackageManager.FEATURE_WIFI_DIRECT))
            .put("cinqGhz", wifi?.is5GHzBandSupported == true)
            .put("enCours", enCours())
    }

    fun envoyer(ctx: Context, fichiers: List<FichierAEnvoyer>, groupe: Boolean, prenom: String) {
        if (session != null || fichiers.isEmpty()) return
        Envoi(ctx.applicationContext, fichiers, groupe, prenom).also { session = it }.demarrer()
    }

    fun recevoir(ctx: Context, prenom: String) {
        if (session != null) return
        Reception(ctx.applicationContext, prenom).also { session = it }.demarrer()
    }

    fun arreter() {
        session?.arreter()
    }

    fun signaler(etat: String, remplir: JSONObject.() -> Unit = {}) {
        val e = JSONObject().put("type", "mam").put("etat", etat)
        e.remplir()
        ecouteur?.invoke(e)
    }

    // ───────────────────────────── Outils ─────────────────────────────

    private fun hex(octets: ByteArray, debut: Int = 0, longueur: Int = octets.size - debut): String =
        (debut until debut + longueur).joinToString("") { "%02X".format(octets[it]) }

    private fun aleatoire(n: Int): ByteArray = ByteArray(n).also { SecureRandom().nextBytes(it) }

    private fun sha256(texte: String): ByteArray =
        MessageDigest.getInstance("SHA-256").digest(texte.toByteArray(Charsets.UTF_8))

    fun motDePasse(ssid: String): String = hex(sha256("$SECRET_RESEAU:$ssid"), 0, 8).lowercase()

    private fun maintenant() = SystemClock.elapsedRealtime()

    /** Identifiant durable de CE téléphone : sert à reprendre un envoi coupé. */
    private fun appareil(ctx: Context): String {
        val p = ctx.getSharedPreferences("main_a_main", Context.MODE_PRIVATE)
        return p.getString("appareil", null) ?: hex(aleatoire(8)).also { p.edit().putString("appareil", it).apply() }
    }

    /** Une balise entendue, et ses dernières mesures de force. */
    private class Vu {
        @Volatile var donnees: ByteArray = ByteArray(0)
        @Volatile var dernier = 0L
        val mesures = ArrayDeque<Pair<Long, Int>>()

        fun moyenne(): Int = synchronized(this) {
            if (mesures.isEmpty()) -127 else mesures.sumOf { it.second } / mesures.size
        }
    }

    private class Candidat(val id: String, val vu: Vu, val force: Int)

    // ───────────────────────────── Session commune ─────────────────────────────

    private abstract class Session(val ctx: Context) {
        val fini = AtomicBoolean(false)
        private val p2p: WifiP2pManager? = ctx.getSystemService(WifiP2pManager::class.java)
        private val canalP2p: WifiP2pManager.Channel? = p2p?.initialize(ctx, ctx.mainLooper, null)
        private val bt = ctx.getSystemService(BluetoothManager::class.java)?.adapter
        private var rappelAnnonce: AdvertiseCallback? = null
        private var rappelScan: ScanCallback? = null
        val vus = ConcurrentHashMap<String, Vu>()
        val prises = java.util.Collections.synchronizedList(mutableListOf<java.io.Closeable>())

        abstract fun deroulement()

        fun demarrer() {
            Thread({
                try {
                    if (p2p == null || canalP2p == null) throw Refus("Ce téléphone ne sait pas faire de Wi-Fi Direct.")
                    if (bt == null || !bt.isEnabled) throw Refus("Allumez le Bluetooth : il sert à trouver l'autre téléphone.")
                    deroulement()
                } catch (e: Refus) {
                    if (!fini.get()) signaler("erreur") { put("message", e.message) }
                } catch (e: SecurityException) {
                    if (!fini.get()) signaler("erreur") { put("message", "Autorisation manquante : touchez de nouveau pour l'accorder.") }
                } catch (e: Exception) {
                    if (!fini.get()) signaler("erreur") { put("message", e.message ?: "La liaison a échoué. Réessayez.") }
                } finally {
                    nettoyer()
                    if (fini.get()) signaler("arrete")
                }
            }, "main-a-main").start()
        }

        fun arreter() {
            fini.set(true)
            synchronized(prises) { prises.forEach { try { it.close() } catch (_: Exception) {} } }
        }

        private fun nettoyer() {
            arreterAnnonce()
            arreterEcoute()
            supprimerGroupe()
            try { canalP2p?.close() } catch (_: Exception) {}
            if (session === this) session = null
            MamService.arreter(ctx)
        }

        // ─── Bluetooth ───

        @SuppressLint("MissingPermission")
        fun annoncer(donnees: ByteArray): Boolean {
            val annonceur = bt?.bluetoothLeAdvertiser ?: return false
            arreterAnnonce()
            val reglages = AdvertiseSettings.Builder()
                .setAdvertiseMode(AdvertiseSettings.ADVERTISE_MODE_LOW_LATENCY)
                .setTxPowerLevel(AdvertiseSettings.ADVERTISE_TX_POWER_MEDIUM)
                .setConnectable(false)
                .build()
            val contenu = AdvertiseData.Builder()
                .addManufacturerData(FABRICANT, donnees)
                .setIncludeDeviceName(false)
                .setIncludeTxPowerLevel(false)
                .build()
            val reponse = CountDownLatch(1)
            val reussi = AtomicBoolean(false)
            val rappel = object : AdvertiseCallback() {
                override fun onStartSuccess(effectifs: AdvertiseSettings) {
                    reussi.set(true)
                    reponse.countDown()
                }

                override fun onStartFailure(code: Int) = reponse.countDown()
            }
            annonceur.startAdvertising(reglages, contenu, rappel)
            reponse.await(3, TimeUnit.SECONDS)
            rappelAnnonce = rappel
            return reussi.get()
        }

        @SuppressLint("MissingPermission")
        fun arreterAnnonce() {
            val rappel = rappelAnnonce ?: return
            rappelAnnonce = null
            try { bt?.bluetoothLeAdvertiser?.stopAdvertising(rappel) } catch (_: Exception) {}
        }

        /** Écoute les balises « K<genre> » (R : receveurs, S : envoyeurs). */
        @SuppressLint("MissingPermission")
        fun ecouter(genre: Char) {
            val scanner = bt?.bluetoothLeScanner ?: throw Refus("Bluetooth indisponible pour l'instant.")
            val filtre = ScanFilter.Builder()
                .setManufacturerData(FABRICANT, byteArrayOf('K'.code.toByte(), genre.code.toByte()))
                .build()
            val reglages = ScanSettings.Builder().setScanMode(ScanSettings.SCAN_MODE_LOW_LATENCY).build()
            val rappel = object : ScanCallback() {
                override fun onScanResult(type: Int, r: ScanResult) = noter(r)
                override fun onBatchScanResults(liste: MutableList<ScanResult>) = liste.forEach { noter(it) }
            }
            scanner.startScan(listOf(filtre), reglages, rappel)
            rappelScan = rappel
        }

        private fun noter(r: ScanResult) {
            val d = r.scanRecord?.getManufacturerSpecificData(FABRICANT) ?: return
            if (d.size < 6) return
            val id = hex(d, 2, 3)
            val t = maintenant()
            val vu = vus.getOrPut(id) { Vu() }
            synchronized(vu) {
                vu.donnees = d
                vu.dernier = t
                vu.mesures.addLast(t to r.rssi)
                while (vu.mesures.isNotEmpty() && vu.mesures.first().first < t - 2000) vu.mesures.removeFirst()
            }
        }

        @SuppressLint("MissingPermission")
        fun arreterEcoute() {
            val rappel = rappelScan ?: return
            rappelScan = null
            try { bt?.bluetoothLeScanner?.stopScan(rappel) } catch (_: Exception) {}
        }

        /** Balises entendues à l'instant, de la plus forte à la plus faible. */
        fun proches(): List<Candidat> {
            val t = maintenant()
            return vus.entries
                .filter { t - it.value.dernier < VU_RECEMMENT_MS }
                .map { Candidat(it.key, it.value, it.value.moyenne()) }
                .sortedByDescending { it.force }
        }

        // ─── Wi-Fi Direct ───

        /** Lance une action Wi-Fi Direct et attend sa réponse : -1 si réussie, sinon le code d'échec. */
        fun action(lancer: (WifiP2pManager.ActionListener) -> Unit): Int? {
            val code = AtomicReference<Int?>(null)
            val fin = CountDownLatch(1)
            lancer(object : WifiP2pManager.ActionListener {
                override fun onSuccess() {
                    code.set(-1)
                    fin.countDown()
                }

                override fun onFailure(raison: Int) {
                    code.set(raison)
                    fin.countDown()
                }
            })
            fin.await(10, TimeUnit.SECONDS)
            return code.get()
        }

        fun supprimerGroupe() {
            try { action { p2p?.removeGroup(canalP2p, it) } } catch (_: Exception) {}
        }

        @SuppressLint("MissingPermission")
        fun creerGroupe(ssid: String, cinqGhz: Boolean): Boolean {
            supprimerGroupe()
            val config = WifiP2pConfig.Builder()
                .setNetworkName(ssid)
                .setPassphrase(motDePasse(ssid))
                .setGroupOperatingBand(if (cinqGhz) WifiP2pConfig.GROUP_OWNER_BAND_5GHZ else WifiP2pConfig.GROUP_OWNER_BAND_2GHZ)
                .build()
            return action { p2p!!.createGroup(canalP2p!!, config, it) } == -1
        }

        /** Rejoint le réseau de l'envoyeur ; rend l'adresse de l'envoyeur, ou null. */
        @SuppressLint("MissingPermission")
        fun rejoindre(ssid: String, cinqGhz: Boolean): InetAddress? {
            supprimerGroupe()
            repeat(3) {
                if (fini.get()) return null
                val config = WifiP2pConfig.Builder()
                    .setNetworkName(ssid)
                    .setPassphrase(motDePasse(ssid))
                    .setGroupOperatingBand(if (cinqGhz) WifiP2pConfig.GROUP_OWNER_BAND_5GHZ else WifiP2pConfig.GROUP_OWNER_BAND_2GHZ)
                    .build()
                if (action { p2p!!.connect(canalP2p!!, config, it) } == -1) {
                    val limite = maintenant() + 25_000
                    while (!fini.get() && maintenant() < limite) {
                        val info = infoConnexion()
                        if (info != null && info.groupFormed && !info.isGroupOwner && info.groupOwnerAddress != null) {
                            return info.groupOwnerAddress
                        }
                        Thread.sleep(500)
                    }
                }
                try { action { p2p!!.cancelConnect(canalP2p!!, it) } } catch (_: Exception) {}
                Thread.sleep(1500)
            }
            return null
        }

        @SuppressLint("MissingPermission")
        fun infoConnexion(): WifiP2pInfo? {
            val info = AtomicReference<WifiP2pInfo?>(null)
            val fin = CountDownLatch(1)
            p2p?.requestConnectionInfo(canalP2p) {
                info.set(it)
                fin.countDown()
            } ?: return null
            fin.await(3, TimeUnit.SECONDS)
            return info.get()
        }

        fun suivre(prise: java.io.Closeable) {
            prises.add(prise)
            if (fini.get()) try { prise.close() } catch (_: Exception) {}
        }
    }

    /** Échec à expliquer tel quel à la personne, sans nouvel essai. */
    private class Refus(message: String) : IOException(message)

    /** Débit lissé, pour une vitesse et un temps restant qui ne sautent pas. */
    private class Compteur(val total: Long) {
        var fait = 0L
        private var debut = maintenant()
        private var dernierSignal = 0L
        private var vitesse = 0.0

        fun avancer(n: Long, signalerSi: (Long, Long, Long) -> Unit) {
            fait += n
            val t = maintenant()
            if (t - dernierSignal < 400) return
            val ecoule = (t - debut).coerceAtLeast(1)
            val instant = fait * 1000.0 / ecoule
            vitesse = if (vitesse == 0.0) instant else vitesse * 0.7 + instant * 0.3
            dernierSignal = t
            signalerSi(fait, total, vitesse.toLong())
        }

        fun repartir() {
            debut = maintenant()
            fait = 0
            vitesse = 0.0
        }
    }

    // ───────────────────────────── Envoyer ─────────────────────────────

    private class Envoi(
        ctx: Context,
        val fichiers: List<FichierAEnvoyer>,
        val groupe: Boolean,
        val prenom: String,
    ) : Session(ctx) {
        val monId = aleatoire(3)
        val ssid = PREFIXE_RESEAU + hex(monId)
        val cibles = mutableListOf<String>()
        @Volatile var ouvert = false
        @Volatile var cinqGhz = false

        /** Receveurs acceptés (id → prénom), et ceux qui ont tout reçu. */
        val servis = ConcurrentHashMap<String, String>()
        val termines: MutableSet<String> = ConcurrentHashMap.newKeySet()
        private var dernierConseil = 0L

        override fun deroulement() {
            MamService.demarrer(ctx, "Main à main : recherche du téléphone qui reçoit…")
            signaler("recherche") { put("groupe", groupe) }
            ecouter('R')

            // 1. Le receveur a peut-être déjà touché « Recevoir » : quelques
            // secondes pour l'entendre, et savoir s'il fait du 5 GHz.
            val limite = maintenant() + 3000
            var choisi: Candidat? = null
            while (!fini.get() && maintenant() < limite) {
                choisi = if (groupe) proches().firstOrNull { it.force >= SEUIL_GROUPE } else plusProche()
                if (choisi != null && !groupe) break
                Thread.sleep(200)
            }
            if (fini.get()) return
            val trouve: Candidat? = choisi
            val moi5 = ctx.applicationContext.getSystemService(WifiManager::class.java)?.is5GHzBandSupported == true
            cinqGhz = !groupe && trouve != null && moi5 && (trouve.vu.donnees[5].toInt() and 1) == 1
            when {
                groupe -> majGroupe()
                trouve != null -> synchronized(cibles) { cibles.add(trouve.id) }
                else -> ouvert = true
            }

            // 2. Le réseau direct entre les téléphones.
            signaler("reseau") { put("cinqGhz", cinqGhz) }
            if (!creerGroupe(ssid, cinqGhz)) {
                if (!cinqGhz || !creerGroupe(ssid, false)) {
                    throw Refus("Le téléphone n'a pas pu créer son réseau. Vérifiez que le Wi-Fi est allumé, puis réessayez.")
                }
                cinqGhz = false
            }
            val serveur = ServerSocket()
            serveur.reuseAddress = true
            serveur.soTimeout = 1000
            serveur.bind(InetSocketAddress(PORT))
            suivre(serveur)
            Thread({ accueillir(serveur) }, "mam-accueil").start()
            if (!annoncerEtat()) {
                throw Refus("Ce téléphone ne sait pas émettre de signal Bluetooth : envoyez depuis l'autre téléphone, ou passez par le kiosque.")
            }
            MamService.maj(ctx, "Main à main : en attente du téléphone qui reçoit", null)
            signaler("attente") { put("ouvert", ouvert); put("groupe", groupe) }

            // 3. Suivre qui est à côté, jusqu'à la fin.
            val debut = maintenant()
            while (!fini.get()) {
                if (!groupe && termines.isNotEmpty()) break
                if (groupe) {
                    if (majGroupe()) annoncerEtat()
                } else if (servis.isEmpty() && ouvert) {
                    plusProche()?.let {
                        synchronized(cibles) { cibles.add(it.id) }
                        ouvert = false
                        annoncerEtat()
                    }
                }
                if (servis.isEmpty() && maintenant() - debut > ATTENTE_MAX_MS) {
                    throw Refus("Personne n'a touché « Recevoir » à côté de vous. Réessayez quand l'autre téléphone est prêt.")
                }
                Thread.sleep(500)
            }
            // Laisse au receveur le temps de lire la fin avant de couper le réseau.
            if (!fini.get()) Thread.sleep(2000)
        }

        /** Le receveur NETTEMENT le plus proche, ou null (et un conseil à l'écran). */
        private fun plusProche(): Candidat? {
            val liste = proches()
            val premier = liste.firstOrNull() ?: return null
            val second = liste.getOrNull(1)
            val conseil = when {
                premier.force < SEUIL_PROCHE -> "rapprocher"
                second != null && premier.force - second.force < MARGE_PLUS_PROCHE -> "plusieurs"
                else -> return premier
            }
            if (maintenant() - dernierConseil > 3000) {
                dernierConseil = maintenant()
                signaler(conseil)
            }
            return null
        }

        /** Groupe : ajoute les receveurs proches, jusqu'à GROUPE_MAX. Vrai si la liste a changé. */
        private fun majGroupe(): Boolean = synchronized(cibles) {
            var change = false
            for (c in proches()) {
                if (cibles.size >= GROUPE_MAX) break
                if (c.force >= SEUIL_GROUPE && c.id !in cibles) {
                    cibles.add(c.id)
                    change = true
                }
            }
            change
        }

        /** Balise « KS » : mon numéro, mes drapeaux, et à qui je veux envoyer. */
        private fun annoncerEtat(): Boolean {
            val liste = synchronized(cibles) { cibles.take(GROUPE_MAX).toList() }
            var drapeaux = 0
            if (cinqGhz) drapeaux = drapeaux or DRAPEAU_5GHZ
            if (ouvert || groupe) drapeaux = drapeaux or DRAPEAU_OUVERT
            if (groupe) drapeaux = drapeaux or DRAPEAU_GROUPE
            val donnees = ByteBuffer.allocate(6 + 3 * liste.size)
            donnees.put('K'.code.toByte()).put('S'.code.toByte()).put(monId).put(drapeaux.toByte())
            liste.forEach { id -> donnees.put(ByteArray(3) { i -> id.substring(i * 2, i * 2 + 2).toInt(16).toByte() }) }
            return annoncer(donnees.array())
        }

        private fun accueillir(serveur: ServerSocket) {
            while (!fini.get()) {
                val prise = try {
                    serveur.accept()
                } catch (_: SocketTimeoutException) {
                    continue
                } catch (_: IOException) {
                    break
                }
                suivre(prise)
                Thread({ servir(prise) }, "mam-envoi").start()
            }
        }

        private fun servir(prise: Socket) {
            var id = ""
            var prenomReceveur = ""
            try {
                prise.soTimeout = 30_000
                MamCanal.ouvrir(prise, serveur = true).use { canal ->
                    val bonjour = canal.recevoirJson()
                    id = bonjour.optString("id").take(6)
                    prenomReceveur = bonjour.optString("prenom").take(40)
                    val accepte = synchronized(cibles) {
                        val oui = when {
                            servis.containsKey(id) -> true // reprise après une coupure
                            groupe -> servis.size < GROUPE_MAX
                            servis.isNotEmpty() -> false // un seul receveur hors groupe
                            ouvert -> true
                            else -> id in cibles
                        }
                        if (oui) servis[id] = prenomReceveur
                        oui
                    }
                    if (!accepte) {
                        canal.envoyerJson(JSONObject().put("type", "refus").put("raison", "Cet envoi est destiné à un autre téléphone."))
                        return
                    }
                    val liste = JSONArray()
                    fichiers.forEachIndexed { i, f ->
                        liste.put(JSONObject().put("i", i).put("nom", f.nom).put("taille", f.taille)
                            .put("type", f.type).put("modifie", f.modifie))
                    }
                    canal.envoyerJson(JSONObject().put("type", "liste").put("prenom", prenom)
                        .put("appareil", appareil(ctx)).put("fichiers", liste))
                    val total = fichiers.sumOf { it.taille.coerceAtLeast(0) }
                    signaler("relie") {
                        put("role", "envoi"); put("id", id); put("prenom", prenomReceveur)
                        put("symbole", canal.symbole()); put("total", total); put("nombre", fichiers.size)
                        put("cinqGhz", cinqGhz)
                    }
                    MamService.maj(ctx, "Main à main : envoi à ${prenomReceveur.ifBlank { "l'autre téléphone" }}", 0)

                    val demande = canal.recevoirJson()
                    if (demande.optString("type") == "refus") {
                        signaler("erreur") { put("id", id); put("message", demande.optString("raison", "L'autre téléphone a refusé.")) }
                        synchronized(cibles) { servis.remove(id) }
                        return
                    }
                    val depuis = demande.getJSONArray("depuis")
                    val deja = (0 until fichiers.size).sumOf { depuis.optLong(it, 0L).coerceAtLeast(0) }
                    val compteur = Compteur(total)
                    compteur.fait = deja
                    fichiers.forEachIndexed { i, f ->
                        envoyerFichier(canal, i, f, depuis.optLong(i, 0L), compteur, id, prenomReceveur)
                    }
                    canal.envoyerJson(JSONObject().put("type", "fin"))
                    val merci = canal.recevoirJson()
                    if (merci.optString("type") != "merci") throw IOException("Fin inattendue.")
                    termines.add(id)
                    signaler("termine") { put("role", "envoi"); put("id", id); put("prenom", prenomReceveur); put("nombre", fichiers.size) }
                }
            } catch (e: Exception) {
                if (!fini.get() && id.isNotEmpty() && id !in termines) {
                    signaler("coupure") { put("id", id); put("prenom", prenomReceveur) }
                }
            } finally {
                try { prise.close() } catch (_: Exception) {}
            }
        }

        private fun envoyerFichier(
            canal: MamCanal, i: Int, f: FichierAEnvoyer, depuis: Long,
            compteur: Compteur, id: String, prenomReceveur: String,
        ) {
            if (f.taille in 0..depuis) {
                canal.envoyerJson(JSONObject().put("type", "fin-fichier").put("i", i))
                return
            }
            val pfd = ctx.contentResolver.openFileDescriptor(f.uri, "r")
                ?: throw Refus("« ${f.nom} » ne peut plus être lu sur ce téléphone.")
            pfd.use {
                FileInputStream(pfd.fileDescriptor).use { flux ->
                    if (depuis > 0) {
                        try {
                            flux.channel.position(depuis)
                        } catch (_: Exception) {
                            var reste = depuis
                            while (reste > 0) {
                                val saute = flux.skip(reste)
                                if (saute <= 0) throw IOException("Reprise impossible.")
                                reste -= saute
                            }
                        }
                    }
                    val tampon = ByteArray(4 + MamCanal.TAILLE_MORCEAU)
                    ByteBuffer.wrap(tampon).putInt(0, i)
                    while (true) {
                        if (fini.get()) throw IOException("Arrêté.")
                        val n = flux.read(tampon, 4, MamCanal.TAILLE_MORCEAU)
                        if (n < 0) break
                        if (n == 0) continue
                        canal.envoyer(MamCanal.TYPE_DONNEES, tampon, 0, 4 + n)
                        compteur.avancer(n.toLong()) { fait, total, vitesse ->
                            signaler("progression") {
                                put("role", "envoi"); put("id", id); put("prenom", prenomReceveur)
                                put("fait", fait); put("total", total); put("vitesse", vitesse); put("fichier", f.nom)
                            }
                            if (total > 0) MamService.maj(ctx, "Main à main : envoi de ${f.nom}", (fait * 100 / total).toInt())
                        }
                    }
                }
            }
            canal.envoyerJson(JSONObject().put("type", "fin-fichier").put("i", i))
        }
    }

    // ───────────────────────────── Recevoir ─────────────────────────────

    private class Reception(ctx: Context, val prenom: String) : Session(ctx) {
        val monId = aleatoire(3)
        val monIdHex = hex(monId)

        override fun deroulement() {
            MamService.demarrer(ctx, "Main à main : prêt à recevoir")
            val moi5 = ctx.applicationContext.getSystemService(WifiManager::class.java)?.is5GHzBandSupported == true
            val balise = byteArrayOf('K'.code.toByte(), 'R'.code.toByte(), monId[0], monId[1], monId[2], if (moi5) 1 else 0)
            val annonce = annoncer(balise)
            ecouter('S')
            signaler("attente-envoyeur") { put("annonce", annonce) }

            // 1. Trouver l'envoyeur : celui qui m'appelle, sinon l'envoi
            // ouvert nettement le plus proche.
            val limite = maintenant() + ATTENTE_MAX_MS
            var choisi: Candidat? = null
            while (!fini.get() && choisi == null) {
                if (maintenant() > limite) throw Refus("Aucun envoi à côté de vous. Réessayez quand l'autre téléphone touche « Envoyer ».")
                choisi = envoyeur()
                if (choisi == null) Thread.sleep(200)
            }
            if (fini.get() || choisi == null) return
            val drapeaux = choisi.vu.donnees[5].toInt()
            val ssid = PREFIXE_RESEAU + choisi.id
            signaler("connexion")
            MamService.maj(ctx, "Main à main : liaison avec l'autre téléphone…", null)

            // 2. Rejoindre son réseau.
            var adresse = rejoindre(ssid, drapeaux and DRAPEAU_5GHZ != 0)
                ?: throw Refus("Impossible de rejoindre l'autre téléphone. Rapprochez-vous, puis réessayez des deux côtés.")
            arreterAnnonce()
            arreterEcoute()

            // 3. Recevoir, en reprenant là où ça s'est arrêté si la liaison saute.
            var coupures = 0
            while (!fini.get()) {
                try {
                    recevoirDe(adresse)
                    return
                } catch (e: Refus) {
                    throw e
                } catch (e: IOException) {
                    if (fini.get()) return
                    coupures++
                    if (coupures > 6) throw Refus("La liaison a sauté trop souvent. Ce qui est déjà reçu est gardé : réessayez, ça reprendra où ça s'est arrêté.")
                    signaler("coupure")
                    Thread.sleep(1500)
                    val info = infoConnexion()
                    if (info == null || !info.groupFormed) {
                        adresse = rejoindre(ssid, drapeaux and DRAPEAU_5GHZ != 0) ?: continue
                    }
                }
            }
        }

        private fun envoyeur(): Candidat? {
            val liste = proches()
            liste.firstOrNull { c -> c.vu.donnees.size >= 9 && viseMoi(c.vu.donnees) }?.let { return it }
            val ouverts = liste.filter { it.vu.donnees[5].toInt() and DRAPEAU_OUVERT != 0 }
            val premier = ouverts.firstOrNull() ?: return null
            if (premier.vu.donnees[5].toInt() and DRAPEAU_GROUPE != 0) {
                return premier.takeIf { it.force >= SEUIL_GROUPE }
            }
            val second = ouverts.getOrNull(1)
            if (premier.force < SEUIL_PROCHE) return null
            if (second != null && premier.force - second.force < MARGE_PLUS_PROCHE) return null
            return premier
        }

        private fun viseMoi(d: ByteArray): Boolean {
            var i = 6
            while (i + 3 <= d.size) {
                if (hex(d, i, 3) == monIdHex) return true
                i += 3
            }
            return false
        }

        private fun connecter(adresse: InetAddress): Socket {
            var derniere: Exception? = null
            repeat(10) {
                if (fini.get()) throw IOException("Arrêté.")
                try {
                    return Socket().apply { connect(InetSocketAddress(adresse, PORT), 5000) }
                } catch (e: Exception) {
                    derniere = e
                    Thread.sleep(1000)
                }
            }
            throw IOException(derniere?.message ?: "L'autre téléphone ne répond pas.")
        }

        private fun recevoirDe(adresse: InetAddress) {
            val prise = connecter(adresse)
            suivre(prise)
            prise.soTimeout = 30_000
            MamCanal.ouvrir(prise, serveur = false).use { canal ->
                canal.envoyerJson(JSONObject().put("type", "bonjour").put("id", monIdHex).put("prenom", prenom))
                val liste = canal.recevoirJson()
                if (liste.optString("type") == "refus") throw Refus(liste.optString("raison", "L'autre téléphone a refusé."))
                val appareilEnvoyeur = liste.optString("appareil").take(32)
                val prenomEnvoyeur = liste.optString("prenom").take(40)
                val fichiers = liste.getJSONArray("fichiers")
                val nombre = fichiers.length()
                if (nombre !in 1..500) throw Refus("Liste de fichiers invalide.")
                val total = (0 until nombre).sumOf { fichiers.getJSONObject(it).optLong("taille", 0L).coerceAtLeast(0) }
                signaler("relie") {
                    put("role", "reception"); put("prenom", prenomEnvoyeur); put("symbole", canal.symbole())
                    put("total", total); put("nombre", nombre)
                }

                // Place sur le téléphone.
                val parties = (0 until nombre).map { i -> Partiel.preparer(ctx, appareilEnvoyeur, fichiers.getJSONObject(i)) }
                val reste = parties.sumOf { p -> if (p.taille >= 0) p.taille - p.deja else 0 }
                val libre = StatFs(Environment.getExternalStorageDirectory().path).availableBytes
                if (reste + MARGE_DISQUE > libre) {
                    canal.envoyerJson(JSONObject().put("type", "refus").put("raison", "L'autre téléphone n'a pas assez de place."))
                    throw Refus("Pas assez de place sur ce téléphone : il manque ${(reste + MARGE_DISQUE - libre) / (1024 * 1024)} Mo.")
                }
                val depuis = JSONArray()
                parties.forEach { depuis.put(it.deja) }
                canal.envoyerJson(JSONObject().put("type", "demande").put("depuis", depuis))
                MamService.maj(ctx, "Main à main : réception de ${prenomEnvoyeur.ifBlank { "l'autre téléphone" }}", 0)

                val compteur = Compteur(total)
                compteur.fait = parties.sumOf { it.deja }
                var courant = -1
                var sortie: FileOutputStream? = null
                var pfd: android.os.ParcelFileDescriptor? = null
                fun fermer() {
                    try { sortie?.fd?.sync() } catch (_: Exception) {}
                    try { sortie?.close() } catch (_: Exception) {}
                    try { pfd?.close() } catch (_: Exception) {}
                    sortie = null
                    pfd = null
                    courant = -1
                }
                try {
                    while (true) {
                        val (type, contenu) = canal.recevoir()
                        if (type == MamCanal.TYPE_DONNEES) {
                            val i = ByteBuffer.wrap(contenu, 0, 4).int
                            val p = parties.getOrNull(i) ?: throw IOException("Morceau inattendu.")
                            if (i != courant) {
                                fermer()
                                val ouvert = ctx.contentResolver.openFileDescriptor(p.uri, "rw")
                                    ?: throw Refus("Impossible d'écrire « ${p.nom} » sur ce téléphone.")
                                pfd = ouvert
                                sortie = FileOutputStream(ouvert.fileDescriptor).also { it.channel.position(p.deja) }
                                courant = i
                            }
                            val n = contenu.size - 4
                            if (p.taille >= 0 && p.deja + n > p.taille) throw IOException("Fichier plus gros qu'annoncé.")
                            sortie!!.write(contenu, 4, n)
                            p.deja += n
                            compteur.avancer(n.toLong()) { fait, t, vitesse ->
                                signaler("progression") {
                                    put("role", "reception"); put("fait", fait); put("total", t)
                                    put("vitesse", vitesse); put("fichier", p.nom)
                                }
                                if (t > 0) MamService.maj(ctx, "Main à main : réception de ${p.nom}", (fait * 100 / t).toInt())
                            }
                        } else {
                            val message = JSONObject(String(contenu, Charsets.UTF_8))
                            when (message.optString("type")) {
                                "fin-fichier" -> {
                                    val i = message.getInt("i")
                                    val p = parties.getOrNull(i) ?: throw IOException("Fichier inattendu.")
                                    if (courant == i) fermer()
                                    if (p.taille >= 0 && p.deja != p.taille) throw IOException("Fichier incomplet.")
                                    Partiel.publier(ctx, p)
                                    val n = synchronized(recus) { recus.add(p.uri to p.type); recus.size - 1 }
                                    signaler("fichier") { put("n", n); put("nom", p.nom); put("typeFichier", p.type); put("taille", p.deja) }
                                }
                                "fin" -> {
                                    fermer()
                                    canal.envoyerJson(JSONObject().put("type", "merci"))
                                    signaler("termine") { put("role", "reception"); put("prenom", prenomEnvoyeur); put("nombre", nombre) }
                                    return
                                }
                            }
                        }
                    }
                } finally {
                    fermer()
                }
            }
        }
    }

    // ───────────────────────────── Fichiers reçus, et leur reprise ─────────────────────────────

    /**
     * Un fichier en cours de réception, dans Téléchargements/Main a main.
     * Invisible (« en attente ») tant qu'il n'est pas complet ; son adresse
     * est notée pour reprendre à la même place si la liaison saute, même
     * plus tard, avec le même envoyeur et le même fichier.
     */
    private class Partiel(val cle: String, val uri: Uri, val nom: String, val type: String, val taille: Long, var deja: Long) {
        companion object {
            private fun prefs(ctx: Context) = ctx.getSharedPreferences("main_a_main_partiels", Context.MODE_PRIVATE)

            fun preparer(ctx: Context, appareilEnvoyeur: String, f: JSONObject): Partiel {
                val nom = f.optString("nom", "fichier").replace('/', '_').replace('\\', '_').take(150).ifBlank { "fichier" }
                val taille = f.optLong("taille", -1L)
                val type = f.optString("type", "application/octet-stream").take(100)
                val cle = hex(sha256("$appareilEnvoyeur|$nom|$taille|${f.optLong("modifie", 0L)}"), 0, 12)
                if (taille >= 0) {
                    prefs(ctx).getString(cle, null)?.let { adresse ->
                        val uri = Uri.parse(adresse)
                        val deja = try {
                            ctx.contentResolver.openFileDescriptor(uri, "r")?.use { it.statSize }
                        } catch (_: Exception) {
                            null
                        }
                        if (deja != null && deja in 0..taille) return Partiel(cle, uri, nom, type, taille, deja)
                    }
                }
                val valeurs = ContentValues().apply {
                    put(MediaStore.Downloads.DISPLAY_NAME, nom)
                    put(MediaStore.Downloads.MIME_TYPE, type)
                    put(MediaStore.Downloads.RELATIVE_PATH, Environment.DIRECTORY_DOWNLOADS + "/" + DOSSIER_RECUS)
                    put(MediaStore.Downloads.IS_PENDING, 1)
                }
                val uri = ctx.contentResolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, valeurs)
                    ?: throw Refus("Impossible de créer « $nom » dans Téléchargements.")
                if (taille >= 0) prefs(ctx).edit().putString(cle, uri.toString()).apply()
                return Partiel(cle, uri, nom, type, taille, 0L)
            }

            fun publier(ctx: Context, p: Partiel) {
                val valeurs = ContentValues().apply { put(MediaStore.Downloads.IS_PENDING, 0) }
                try { ctx.contentResolver.update(p.uri, valeurs, null, null) } catch (_: Exception) {}
                prefs(ctx).edit().remove(p.cle).apply()
            }
        }
    }
}
