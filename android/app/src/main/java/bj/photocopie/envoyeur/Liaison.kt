package bj.photocopie.envoyeur

import android.Manifest
import android.annotation.SuppressLint
import android.bluetooth.BluetoothManager
import android.bluetooth.le.AdvertiseCallback
import android.bluetooth.le.AdvertiseData
import android.bluetooth.le.AdvertiseSettings
import android.bluetooth.le.ScanCallback
import android.bluetooth.le.ScanFilter
import android.bluetooth.le.ScanResult
import android.bluetooth.le.ScanSettings
import android.content.Context
import android.content.pm.PackageManager
import android.location.LocationManager
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.wifi.WifiManager
import android.net.wifi.p2p.WifiP2pConfig
import android.net.wifi.p2p.WifiP2pManager
import org.json.JSONObject
import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.Inet4Address
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.NetworkInterface
import java.net.HttpURLConnection
import java.net.Socket
import java.net.URL
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference

/**
 * Le lien entre le téléphone et le PC du kiosque, sans internet.
 *
 * Première méthode (la principale) : le PC crée son Wi-Fi, le client scanne
 * le QR Wi-Fi du guichet avec l'appareil photo et accepte. Le téléphone est
 * alors sur le réseau du PC : on le trouve tout de suite (voir [pcSurLeWifi]).
 *
 * Seconde méthode (PC qui ne sait pas créer de Wi-Fi), comme Quick Share :
 * 1. le téléphone crée son propre réseau (Wi-Fi Direct), avec le mot de
 *    passe du kiosque ;
 * 2. il APPELLE le PC par Bluetooth : « je suis tel réseau, viens »
 *    (voir `appel_ble.rs`) — le PC n'a plus à le chercher ;
 * 3. le PC le rejoint aussitôt, et le téléphone trouve son adresse.
 * Pendant l'attente, la première méthode reste guettée : un client qui
 * scanne le QR Wi-Fi entre-temps est relié aussitôt.
 *
 * L'envoi lui-même est fait par l'interface (client-web/index.html), qui
 * affiche la progression. Bloquant : à lancer hors du fil de l'interface.
 * `dire` reçoit chaque étape, pour le journal.
 */
class Liaison(
    private val ctx: Context,
    private val dire: (String) -> Unit,
) {
    private val p2p = ctx.getSystemService(WifiP2pManager::class.java)
    private val canal = p2p?.initialize(ctx, ctx.mainLooper, null)

    companion object {
        /** Le temps de voir si le téléphone est déjà sur le Wi-Fi de la boutique. */
        private const val ATTENTE_WIFI_BOUTIQUE_MS = 5_000L
    }

    private val bluetooth = try {
        ctx.getSystemService(BluetoothManager::class.java)?.adapter
    } catch (_: Exception) {
        null
    }

    /** L'appel Bluetooth en cours (voir [appelerPc]). */
    @Volatile private var rappelAppel: AdvertiseCallback? = null

    /** Le nom du réseau créé, pour le journal. */
    var nomReseau: String? = null
        private set

    /** Échec parce que la Localisation est éteinte (Android 10 à 12) : l'activité ouvre le réglage. */
    var localisationRequise = false
        private set

    /**
     * Le résultat d'une liaison : l'adresse du PC sur un Wi-Fi, ou le canal
     * Bluetooth (secours, plus lent) quand aucun Wi-Fi ne passe.
     */
    class Resultat(val pc: InetAddress?, val canal: CanalBt?)

    /** Le canal Bluetooth gardé pour le mode Bluetooth (voir [Resultat]). */
    @Volatile var canalBt: CanalBt? = null
        private set

    /** Relie le téléphone au PC, par le meilleur chemin qui marche. Null : échec (raison dans [raison]). */
    fun ouvrir(): Resultat? {
        val debut = System.currentTimeMillis()
        val wifi = attendreWifi()

        // Pendant qu'on regarde le Wi-Fi de la boutique : écouter la balise
        // du PC (numéro du kiosque, adresse Bluetooth du PC).
        val balise = Thread {
            if (Reglages.kiosqueRecent(ctx) == null || Reglages.adressePcRecente(ctx) == null) ecouterBalise(4000)
        }
        balise.start()

        // 1. Le téléphone est-il déjà sur le Wi-Fi de la boutique ?
        if (wifi) {
            dire("📶 Recherche du Wi-Fi de la boutique…")
            val finWifi = debut + ATTENTE_WIFI_BOUTIQUE_MS
            while (System.currentTimeMillis() < finWifi) {
                pcSurLeWifi()?.let { pc ->
                    val secondes = (System.currentTimeMillis() - debut) / 1000.0
                    dire("✅ Téléphone sur le Wi-Fi de la boutique : PC à ${pc.hostAddress} (${"%.1f".format(secondes)} s).")
                    return Resultat(pc, null)
                }
                Thread.sleep(1000)
            }
        }
        balise.join(5000)

        // 2. Le canal Bluetooth avec le PC : on se dit où se retrouver.
        val c = ouvrirCanal()

        // 3. Le réseau du téléphone, que le PC rejoint.
        if (wifi) {
            val reseau = creerUnReseau()
            if (reseau != null) {
                val (nom, motDePasse) = reseau
                if (c != null) {
                    lienParLeCanal(c, nom, motDePasse)?.let { return it }
                } else {
                    // Sans canal : l'appel Bluetooth, et le PC cherche le réseau.
                    appelerPc(nom)
                    try {
                        val pc = trouverPc(90_000)
                        if (pc != null) {
                            if (reseauWifi != null) supprimerReseau()
                            val secondes = (System.currentTimeMillis() - debut) / 1000.0
                            dire("✅ PC trouvé à ${pc.hostAddress} (${"%.1f".format(secondes)} s).")
                            return Resultat(pc, null)
                        }
                        dire("❌ Le PC ne s'est pas connecté en 90 s.")
                    } finally {
                        arreterAppel()
                    }
                }
                supprimerReseau()
            }
        }

        // 4. Secours : tout par le canal Bluetooth.
        if (c != null && c.ouvert) {
            dire("🔵 Aucun Wi-Fi ne passe : envoi par Bluetooth (plus lent).")
            canalBt = c
            return Resultat(null, c)
        }
        c?.fermer()
        if (raison == null) {
            raison = when {
                !wifi -> "Allumez le Wi-Fi ou le Bluetooth du téléphone, puis réessayez."
                localisationRequise -> "Allumez la « Localisation » du téléphone (Android l'exige pour créer le lien avec le PC), puis revenez ici."
                else -> "L'ordinateur de la boutique ne s'est pas relié. Approchez-vous du guichet, vérifiez que le logiciel est ouvert et que le Bluetooth du téléphone est allumé, puis réessayez."
            }
        }
        return null
    }

    /** Le canal Bluetooth vers le PC entendu dans la balise, ou null. */
    private fun ouvrirCanal(): CanalBt? {
        val mac = Reglages.adressePcRecente(ctx)
        if (mac == null) {
            dire("Adresse Bluetooth du PC inconnue (balise non entendue) : pas de canal.")
            return null
        }
        val pourquoi = when {
            bluetooth == null -> "pas de Bluetooth sur ce téléphone"
            !bluetooth.isEnabled -> "Bluetooth éteint"
            android.os.Build.VERSION.SDK_INT >= 31 && !permis(Manifest.permission.BLUETOOTH_CONNECT) -> "autorisation Bluetooth refusée"
            else -> null
        }
        if (pourquoi != null) {
            dire("⚠️ Pas de canal Bluetooth ($pourquoi).")
            return null
        }
        dire("🔵 Canal Bluetooth vers le PC ($mac)…")
        val c = CanalBt(ctx, dire)
        if (!c.ouvrir(mac)) {
            dire("⚠️ Le PC ne répond pas sur le canal Bluetooth.")
            return null
        }
        dire("🔵 Canal Bluetooth ouvert.")
        return c
    }

    /** Wi-Fi Direct d'abord, point d'accès local d'Android ensuite. (nom, mot de passe) ou null. */
    private fun creerUnReseau(): Pair<String, String>? {
        if (!peutCreerReseau()) {
            dire("⚠️ Autorisation « Appareils à proximité » refusée : pas de réseau.")
            return null
        }
        if (localisationEteinte()) {
            localisationRequise = true
            dire("⚠️ Localisation éteinte : pas de réseau.")
            return null
        }
        if (p2p != null && canal != null) {
            creerReseau()?.let { nom ->
                nomReseau = nom
                dire("📶 Réseau « $nom » créé.")
                val kiosque = Reglages.kiosqueRecent(ctx)
                return nom to (kiosque?.let { Reglages.motDePasseKiosque(it) } ?: Reglages.motDePasse(ctx))
            }
        }
        return creerPointAccesLocal()
    }

    /**
     * Le canal Bluetooth est ouvert : on donne au PC le nom et le mot de passe,
     * il répond quand il est relié. Null : à passer au secours Bluetooth.
     */
    private fun lienParLeCanal(c: CanalBt, nom: String, motDePasse: String): Resultat? {
        appelerPc(nom) // le PC sait aussi par l'appel, au cas où le canal sauterait
        try {
            dire("🔵 Le PC rejoint « $nom »…")
            val (r, _) = try {
                c.echangerAvecReprise(JSONObject().put("t", "wifi").put("ssid", nom).put("mdp", motDePasse), null)
            } catch (e: Exception) {
                dire("⚠️ Canal Bluetooth : ${e.message}")
                return null
            }
            when (r.optString("etat")) {
                "ok" -> {
                    val ip = r.optString("ip")
                    val pc = try { InetAddress.getByName(ip) } catch (_: Exception) { null }
                    if (pc != null && joignable(pc)) {
                        dire("✅ PC relié au réseau du téléphone : $ip.")
                        c.fermer()
                        return Resultat(pc, null)
                    }
                    dire("⚠️ Le PC se dit relié ($ip) mais ne répond pas.")
                }
                "boutique" -> {
                    supprimerReseau()
                    val pc = rejoindreWifiBoutique(r.optString("ssid"), r.optString("mdp"))
                    if (pc != null) {
                        c.fermer()
                        return Resultat(pc, null)
                    }
                }
                else -> dire("⚠️ Le PC n'a pas pu rejoindre : ${r.optString("raison")}.")
            }
            return null
        } finally {
            arreterAppel()
        }
    }

    private fun joignable(pc: InetAddress): Boolean {
        repeat(5) {
            if (portOuvert(pc)) return true
            Thread.sleep(1000)
        }
        return false
    }

    // ─────────────── Point d'accès local (secours du Wi-Fi Direct) ───────────────

    @Volatile private var reservation: WifiManager.LocalOnlyHotspotReservation? = null

    @SuppressLint("MissingPermission")
    private fun creerPointAccesLocal(): Pair<String, String>? {
        val wm = ctx.applicationContext.getSystemService(WifiManager::class.java) ?: return null
        val obtenue = AtomicReference<WifiManager.LocalOnlyHotspotReservation?>(null)
        val fini = CountDownLatch(1)
        try {
            wm.startLocalOnlyHotspot(object : WifiManager.LocalOnlyHotspotCallback() {
                override fun onStarted(r: WifiManager.LocalOnlyHotspotReservation) {
                    obtenue.set(r)
                    fini.countDown()
                }

                override fun onFailed(code: Int) {
                    dire("⚠️ Point d'accès local refusé (code $code).")
                    fini.countDown()
                }
            }, android.os.Handler(android.os.Looper.getMainLooper()))
        } catch (e: Exception) {
            dire("⚠️ Point d'accès local impossible : ${e.message}")
            return null
        }
        fini.await(15, TimeUnit.SECONDS)
        val r = obtenue.get() ?: return null
        reservation = r
        val (nom, mdp) = if (android.os.Build.VERSION.SDK_INT >= 30) {
            val conf = r.softApConfiguration
            val ssid = if (android.os.Build.VERSION.SDK_INT >= 33) {
                conf.wifiSsid?.toString()?.removeSurrounding("\"")
            } else {
                @Suppress("DEPRECATION")
                conf.ssid
            }
            ssid to conf.passphrase
        } else {
            @Suppress("DEPRECATION")
            val conf = r.wifiConfiguration
            @Suppress("DEPRECATION")
            conf?.SSID?.removeSurrounding("\"") to conf?.preSharedKey?.removeSurrounding("\"")
        }
        if (nom.isNullOrEmpty() || mdp.isNullOrEmpty()) {
            fermerPointAccesLocal()
            return null
        }
        nomReseau = nom
        dire("📶 Point d'accès local « $nom » créé.")
        return nom to mdp
    }

    private fun fermerPointAccesLocal() {
        try { reservation?.close() } catch (_: Exception) {}
        reservation = null
    }

    // ─────────────── Rejoindre le Wi-Fi de la boutique tout seul ───────────────

    @Volatile private var demandeBoutique: ConnectivityManager.NetworkCallback? = null

    /**
     * Le PC a donné le nom et le mot de passe du Wi-Fi de la boutique :
     * Android propose de s'y relier (une fenêtre, un geste), puis toute
     * l'application passe par lui, données mobiles allumées ou non.
     */
    private fun rejoindreWifiBoutique(ssid: String, mdp: String): InetAddress? {
        if (ssid.isEmpty()) return null
        val cm = ctx.getSystemService(ConnectivityManager::class.java) ?: return null
        dire("📶 Le PC propose son Wi-Fi « $ssid » : connexion…")
        val specif = android.net.wifi.WifiNetworkSpecifier.Builder().setSsid(ssid).apply {
            if (mdp.length >= 8) setWpa2Passphrase(mdp)
        }.build()
        val demande = android.net.NetworkRequest.Builder()
            .addTransportType(NetworkCapabilities.TRANSPORT_WIFI)
            .removeCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
            .setNetworkSpecifier(specif)
            .build()
        val trouve = AtomicReference<Network?>(null)
        val fini = CountDownLatch(1)
        val rappel = object : ConnectivityManager.NetworkCallback() {
            override fun onAvailable(reseau: Network) {
                trouve.set(reseau)
                fini.countDown()
            }

            override fun onUnavailable() {
                fini.countDown()
            }
        }
        try {
            cm.requestNetwork(demande, rappel, 60_000)
        } catch (e: Exception) {
            dire("⚠️ Wi-Fi de la boutique : ${e.message}")
            return null
        }
        demandeBoutique = rappel
        fini.await(65, TimeUnit.SECONDS)
        val reseau = trouve.get()
        if (reseau == null) {
            dire("⚠️ Wi-Fi de la boutique non rejoint.")
            oublierWifiBoutique()
            return null
        }
        try { cm.bindProcessToNetwork(reseau) } catch (_: Exception) {}
        repeat(10) {
            pcSurLeWifi()?.let { pc ->
                dire("✅ Sur le Wi-Fi de la boutique : PC à ${pc.hostAddress}.")
                return pc
            }
            Thread.sleep(1000)
        }
        oublierWifiBoutique()
        return null
    }

    private fun oublierWifiBoutique() {
        val r = demandeBoutique ?: return
        demandeBoutique = null
        try { ctx.getSystemService(ConnectivityManager::class.java)?.unregisterNetworkCallback(r) } catch (_: Exception) {}
    }

    var raison: String? = null
        private set

    private fun permis(p: String) = ctx.checkSelfPermission(p) == PackageManager.PERMISSION_GRANTED

    private fun peutCreerReseau(): Boolean =
        if (android.os.Build.VERSION.SDK_INT >= 33) permis(Manifest.permission.NEARBY_WIFI_DEVICES)
        else permis(Manifest.permission.ACCESS_FINE_LOCATION)

    private fun localisationEteinte(): Boolean {
        if (android.os.Build.VERSION.SDK_INT >= 33) return false
        val lm = ctx.getSystemService(LocationManager::class.java) ?: return false
        return !lm.isLocationEnabled
    }

    // ───────────────────────────── Bluetooth ─────────────────────────────

    /** Écoute la balise du PC quelques secondes : son numéro de kiosque. */
    @SuppressLint("MissingPermission")
    private fun ecouterBalise(dureeMs: Long) {
        val adaptateur = bluetooth?.takeIf { it.isEnabled } ?: return
        if (android.os.Build.VERSION.SDK_INT >= 31 && !permis(Manifest.permission.BLUETOOTH_SCAN)) return
        val ecoute = try { adaptateur.bluetoothLeScanner } catch (_: Exception) { null } ?: return
        val meilleur = AtomicReference<Pair<Int, String>?>(null)
        val mac = AtomicReference<String?>(null)
        val rappel = object : ScanCallback() {
            override fun onScanResult(type: Int, r: ScanResult) {
                val d = r.scanRecord?.getManufacturerSpecificData(Reglages.FABRICANT_BLE) ?: return
                if (d.size < 5 || d[0] != 'K'.code.toByte() || d[1] != 'Q'.code.toByte()) return
                val numero = (2..4).joinToString("") { "%02X".format(d[it]) }
                val m = meilleur.get()
                if (m == null || r.rssi > m.first) {
                    meilleur.set(r.rssi to numero)
                    // Adresse Bluetooth du PC, si sa balise la donne (11 octets).
                    if (d.size >= 11) mac.set((5..10).joinToString(":") { "%02X".format(d[it]) })
                }
            }
        }
        val filtre = ScanFilter.Builder().setManufacturerData(Reglages.FABRICANT_BLE, Reglages.DONNEES_BLE).build()
        val reglages = ScanSettings.Builder().setScanMode(ScanSettings.SCAN_MODE_LOW_LATENCY).build()
        try {
            ecoute.startScan(listOf(filtre), reglages, rappel)
        } catch (_: Exception) {
            return
        }
        Thread.sleep(dureeMs)
        try { ecoute.stopScan(rappel) } catch (_: Exception) {}
        val trouve = meilleur.get()
        if (trouve != null) {
            Reglages.noterKiosque(ctx, trouve.second, System.currentTimeMillis())
            mac.get()?.let { Reglages.noterAdressePc(ctx, it, System.currentTimeMillis()) }
            dire("🏷 Kiosque ${trouve.second} entendu (${trouve.first} dBm)" + (mac.get()?.let { ", PC $it." } ?: "."))
        } else {
            dire("Balise du kiosque non entendue : réseau sans numéro.")
        }
    }

    /**
     * Appelle le PC par Bluetooth (sans connexion ni appairage) jusqu'à ce
     * qu'il soit trouvé. Sans Bluetooth, le PC cherche quand même le réseau
     * dans la liste des Wi-Fi, en plus lent.
     */
    @SuppressLint("MissingPermission")
    private fun appelerPc(nom: String) {
        val donnees = Reglages.donneesAppel(nom) ?: return
        val adaptateur = bluetooth
        val pourquoi = when {
            adaptateur == null -> "pas de Bluetooth sur ce téléphone"
            !adaptateur.isEnabled -> "Bluetooth éteint"
            android.os.Build.VERSION.SDK_INT >= 31 && !permis(Manifest.permission.BLUETOOTH_ADVERTISE) -> "autorisation Bluetooth refusée"
            else -> null
        }
        if (pourquoi != null) {
            dire("⚠️ Pas d'appel Bluetooth ($pourquoi) : le PC cherchera le réseau tout seul (plus lent).")
            return
        }
        val annonceur = try { adaptateur!!.bluetoothLeAdvertiser } catch (_: Exception) { null }
        if (annonceur == null) {
            dire("⚠️ Ce téléphone ne sait pas appeler en Bluetooth : le PC cherchera le réseau tout seul (plus lent).")
            return
        }
        val reglages = AdvertiseSettings.Builder()
            .setAdvertiseMode(AdvertiseSettings.ADVERTISE_MODE_LOW_LATENCY)
            .setTxPowerLevel(AdvertiseSettings.ADVERTISE_TX_POWER_HIGH)
            .setConnectable(false)
            .setTimeout(0)
            .build()
        val contenu = AdvertiseData.Builder()
            .setIncludeDeviceName(false)
            .setIncludeTxPowerLevel(false)
            .addManufacturerData(Reglages.FABRICANT_BLE, donnees)
            .build()
        val rappel = object : AdvertiseCallback() {
            override fun onStartSuccess(effectifs: AdvertiseSettings?) {
                dire("📣 Appel Bluetooth lancé : le PC sait quel réseau rejoindre.")
            }

            override fun onStartFailure(code: Int) {
                dire("⚠️ Appel Bluetooth refusé par le téléphone (code $code) : le PC cherchera le réseau (plus lent).")
            }
        }
        try {
            annonceur.startAdvertising(reglages, contenu, rappel)
            rappelAppel = rappel
        } catch (e: Exception) {
            dire("⚠️ Appel Bluetooth impossible (${e.message}) : le PC cherchera le réseau (plus lent).")
        }
    }

    @SuppressLint("MissingPermission")
    private fun arreterAppel() {
        val rappel = rappelAppel ?: return
        rappelAppel = null
        try { bluetooth?.bluetoothLeAdvertiser?.stopAdvertising(rappel) } catch (_: Exception) {}
    }

    /** Supprime le réseau : le PC est libéré pour le client suivant. */
    fun fermer() {
        arreterAppel()
        canalBt?.fermer()
        canalBt = null
        oublierWifiBoutique()
        if (reseauWifi != null) {
            reseauWifi = null
            try { ctx.getSystemService(ConnectivityManager::class.java)?.bindProcessToNetwork(null) } catch (_: Exception) {}
        }
        supprimerReseau()
        dire("Réseau supprimé.")
    }

    // ───────────────────────────── Wi-Fi ─────────────────────────────

    /** Le Wi-Fi doit être allumé (sans être connecté à quoi que ce soit). */
    private fun attendreWifi(): Boolean {
        val wifi = ctx.applicationContext.getSystemService(WifiManager::class.java) ?: return true
        if (wifi.isWifiEnabled) return true
        dire("Activez le Wi-Fi dans le panneau qui vient de s'ouvrir (aucun réseau à choisir).")
        val fin = System.currentTimeMillis() + 60_000
        while (System.currentTimeMillis() < fin) {
            if (wifi.isWifiEnabled) {
                Thread.sleep(1500)
                return true
            }
            Thread.sleep(500)
        }
        dire("❌ Le Wi-Fi est resté éteint.")
        return false
    }

    @SuppressLint("MissingPermission")
    private fun creerReseau(): String? {
        supprimerReseau()
        val suffixe = (1..4).map { "ABCDEFGHJKLMNPQRSTUVWXYZ23456789".random() }.joinToString("")
        // Pour le kiosque où se trouve le client (balise entendue) : seul ce
        // kiosque-là le rejoindra. Sans balise : réseau « sans numéro ».
        val kiosque = Reglages.kiosqueRecent(ctx)
        val nom = Reglages.PREFIXE_RESEAU + (kiosque?.let { "$it-" } ?: "") + suffixe
        val motDePasse = kiosque?.let { Reglages.motDePasseKiosque(it) } ?: Reglages.motDePasse(ctx)
        if (kiosque != null) dire("🏷 Réseau pour le kiosque $kiosque.")
        val config = WifiP2pConfig.Builder()
            .setNetworkName(nom)
            .setPassphrase(motDePasse)
            // 2,4 GHz : toutes les cartes Wi-Fi des PC, même anciennes, le voient.
            .setGroupOperatingBand(WifiP2pConfig.GROUP_OWNER_BAND_2GHZ)
            .build()
        repeat(2) { essai ->
            val resultat = AtomicReference<Int?>(null)
            val fini = CountDownLatch(1)
            p2p!!.createGroup(canal!!, config, object : WifiP2pManager.ActionListener {
                override fun onSuccess() {
                    resultat.set(-1)
                    fini.countDown()
                }

                override fun onFailure(raison: Int) {
                    resultat.set(raison)
                    fini.countDown()
                }
            })
            fini.await(10, TimeUnit.SECONDS)
            when (val r = resultat.get()) {
                -1 -> return nom
                else -> {
                    val pourquoi = when (r) {
                        WifiP2pManager.P2P_UNSUPPORTED -> "Wi-Fi Direct non pris en charge"
                        WifiP2pManager.BUSY -> "Wi-Fi occupé"
                        WifiP2pManager.ERROR -> "erreur interne"
                        null -> "pas de réponse"
                        else -> "code $r"
                    }
                    dire("⚠️ Création du réseau refusée ($pourquoi)" + if (essai == 0) ", nouvel essai…" else ".")
                    supprimerReseau()
                    Thread.sleep(2000)
                }
            }
        }
        return null
    }

    private fun supprimerReseau() {
        fermerPointAccesLocal()
        val fini = CountDownLatch(1)
        try {
            p2p?.removeGroup(canal, object : WifiP2pManager.ActionListener {
                override fun onSuccess() = fini.countDown()
                override fun onFailure(raison: Int) = fini.countDown()
            })
            fini.await(3, TimeUnit.SECONDS)
        } catch (_: Exception) {
        }
    }

    // ───────────────────────────── Trouver le PC ─────────────────────────────

    /**
     * Deux moyens en parallèle, le premier qui répond gagne :
     * - l'annonce que le PC diffuse une fois connecté (port 48173) ;
     * - le tour des adresses du réseau, port 4173 (secours).
     */
    private fun trouverPc(delaiMs: Long): InetAddress? {
        val trouve = AtomicReference<InetAddress?>(null)
        val fin = System.currentTimeMillis() + delaiMs

        val ecoute = Thread {
            try {
                DatagramSocket(null).use { s ->
                    s.reuseAddress = true
                    s.broadcast = true
                    s.bind(InetSocketAddress(Reglages.PORT_BALISE))
                    s.soTimeout = 1000
                    val tampon = ByteArray(256)
                    while (trouve.get() == null && System.currentTimeMillis() < fin) {
                        try {
                            val paquet = DatagramPacket(tampon, tampon.size)
                            s.receive(paquet)
                            val texte = String(paquet.data, 0, paquet.length, Charsets.UTF_8)
                            if (texte.startsWith("KIOSQUE")) {
                                dire("📣 Annonce du PC reçue de ${paquet.address.hostAddress}")
                                trouve.compareAndSet(null, paquet.address)
                            }
                        } catch (_: java.net.SocketTimeoutException) {
                        }
                    }
                }
            } catch (e: Exception) {
                dire("⚠️ Écoute de l'annonce impossible : ${e.message}")
            }
        }
        ecoute.start()

        val pool = Executors.newFixedThreadPool(48)
        try {
            while (trouve.get() == null && System.currentTimeMillis() < fin) {
                // Le client a peut-être scanné le QR Wi-Fi du guichet entre-temps.
                pcSurLeWifi()?.let { pc ->
                    dire("✅ Le téléphone a rejoint le Wi-Fi du guichet : PC à ${pc.hostAddress}.")
                    trouve.compareAndSet(null, pc)
                }
                if (trouve.get() != null) break
                val moi = adresseReseauDirect()
                if (moi != null) {
                    val base = moi.address
                    val candidats = (2..254).map { i ->
                        InetAddress.getByAddress(byteArrayOf(base[0], base[1], base[2], i.toByte()))
                    }.filter { it != moi }
                    val taches = candidats.map { ip ->
                        pool.submit {
                            if (trouve.get() == null && portOuvert(ip)) trouve.compareAndSet(null, ip)
                        }
                    }
                    taches.forEach { runCatching { it.get() } }
                }
                if (trouve.get() == null) Thread.sleep(2000)
            }
        } finally {
            pool.shutdownNow()
        }
        ecoute.join(1500)
        return trouve.get()
    }

    // ───────────────────────────── Première méthode ─────────────────────────────

    /** Le réseau Wi-Fi du PC, quand le téléphone y est (première méthode). */
    @Volatile private var reseauWifi: Network? = null

    /**
     * Le téléphone est-il sur le Wi-Fi créé par le PC du guichet ? Le PC est
     * alors la passerelle de ce réseau (192.168.137.1 en général) et répond
     * sur /infos. Les échanges sont ensuite forcés sur ce Wi-Fi : sans
     * internet, Android enverrait sinon tout par les données mobiles.
     */
    @Suppress("DEPRECATION")
    fun pcSurLeWifi(): InetAddress? {
        val cm = ctx.getSystemService(ConnectivityManager::class.java) ?: return null
        val wifis = try {
            cm.allNetworks.filter { cm.getNetworkCapabilities(it)?.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) == true }
        } catch (_: Exception) {
            emptyList()
        }
        for (reseau in wifis) {
            val proprietes = cm.getLinkProperties(reseau) ?: continue
            val candidats = buildList<InetAddress> {
                if (android.os.Build.VERSION.SDK_INT >= 30) proprietes.dhcpServerAddress?.let { add(it) }
                proprietes.routes.filter { it.hasGateway() }.mapNotNull { it.gateway }.forEach { add(it) }
                add(InetAddress.getByAddress(byteArrayOf(192.toByte(), 168.toByte(), 137.toByte(), 1)))
            }.filterIsInstance<Inet4Address>().filter { !it.isAnyLocalAddress }.distinct()
            for (ip in candidats) {
                if (reponduParLePc(reseau, ip)) {
                    reseauWifi = reseau
                    try { cm.bindProcessToNetwork(reseau) } catch (_: Exception) {}
                    return ip
                }
            }
        }
        return null
    }

    private fun reponduParLePc(reseau: Network, ip: InetAddress): Boolean = try {
        val c = reseau.openConnection(URL("http://${ip.hostAddress}:${Reglages.PORT_PC}/infos")) as HttpURLConnection
        c.connectTimeout = 800
        c.readTimeout = 1500
        try { c.responseCode == 200 } finally { c.disconnect() }
    } catch (_: Exception) {
        false
    }

    /** Adresse du téléphone sur son propre réseau Wi-Fi Direct (192.168.49.1 en général). */
    private fun adresseReseauDirect(): Inet4Address? =
        NetworkInterface.getNetworkInterfaces()?.toList().orEmpty()
            .filter { it.isUp && it.name.startsWith("p2p") }
            .flatMap { it.inetAddresses.toList() }
            .filterIsInstance<Inet4Address>()
            .firstOrNull()

    private fun portOuvert(ip: InetAddress): Boolean = try {
        Socket().use { it.connect(InetSocketAddress(ip, Reglages.PORT_PC), 400) }
        true
    } catch (_: Exception) {
        false
    }
}
