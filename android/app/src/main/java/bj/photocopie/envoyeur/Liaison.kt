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

    /** Crée le réseau et attend le PC. Rend son adresse, ou null (et la raison dans [raison]). */
    fun ouvrir(): InetAddress? {
        if (!attendreWifi()) {
            raison = "Le Wi-Fi est resté éteint. Allumez-le (sans choisir de réseau)."
            return null
        }
        val debut = System.currentTimeMillis()

        // Pendant qu'on regarde le Wi-Fi de la boutique : écouter la balise
        // du PC, pour créer le réseau au numéro de CE kiosque.
        val balise = Thread { if (Reglages.kiosqueRecent(ctx) == null) ecouterBalise(4000) }
        balise.start()

        // 1. Le téléphone est-il déjà sur le Wi-Fi de la boutique ?
        dire("📶 Recherche du Wi-Fi de la boutique…")
        val finWifi = debut + ATTENTE_WIFI_BOUTIQUE_MS
        while (System.currentTimeMillis() < finWifi) {
            pcSurLeWifi()?.let { pc ->
                val secondes = (System.currentTimeMillis() - debut) / 1000.0
                dire("✅ Téléphone sur le Wi-Fi de la boutique : PC à ${pc.hostAddress} (${"%.1f".format(secondes)} s).")
                return pc
            }
            Thread.sleep(1000)
        }
        balise.join(5000)

        // 2. Sinon, comme Quick Share : le téléphone crée son réseau et
        //    appelle le PC par Bluetooth.
        if (p2p == null || canal == null) return attendreWifiBoutique(debut, "Ce téléphone ne sait pas créer de réseau Wi-Fi Direct.")
        if (!peutCreerReseau()) return attendreWifiBoutique(debut, "Autorisation « Appareils à proximité » refusée.")
        if (localisationEteinte()) {
            localisationRequise = true
            raison = "Allumez la « Localisation » du téléphone (Android l'exige pour créer le lien avec le PC), puis revenez ici."
            dire("❌ Localisation éteinte : réseau impossible.")
            return null
        }
        val nom = creerReseau() ?: return attendreWifiBoutique(debut, "Le téléphone n'a pas pu créer son réseau.")
        nomReseau = nom
        dire("📶 Réseau « $nom » créé.")
        appelerPc(nom)
        try {
            val pc = trouverPc(90_000)
            if (pc == null) {
                raison = "L'ordinateur de la boutique ne s'est pas relié. Approchez-vous du guichet, vérifiez que le logiciel est ouvert, puis réessayez."
                dire("❌ Le PC ne s'est pas connecté en 90 s.")
                supprimerReseau()
                return null
            }
            // Relié par le Wi-Fi de la boutique entre-temps : le réseau du téléphone ne sert plus.
            if (reseauWifi != null) supprimerReseau()
            val secondes = (System.currentTimeMillis() - debut) / 1000.0
            dire("✅ PC trouvé à ${pc.hostAddress} (${"%.1f".format(secondes)} s).")
            return pc
        } finally {
            arreterAppel()
        }
    }

    /** Pas de réseau possible depuis le téléphone : seul le Wi-Fi de la boutique reste. */
    private fun attendreWifiBoutique(debut: Long, pourquoi: String): InetAddress? {
        dire("⚠️ $pourquoi En attente du Wi-Fi de la boutique…")
        val fin = debut + 90_000
        while (System.currentTimeMillis() < fin) {
            pcSurLeWifi()?.let { return it }
            Thread.sleep(2000)
        }
        raison = "$pourquoi Rejoignez le Wi-Fi de la boutique (code du guichet, ou liste des Wi-Fi), puis revenez ici."
        return null
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
        val rappel = object : ScanCallback() {
            override fun onScanResult(type: Int, r: ScanResult) {
                val d = r.scanRecord?.getManufacturerSpecificData(Reglages.FABRICANT_BLE) ?: return
                if (d.size < 5 || d[0] != 'K'.code.toByte() || d[1] != 'Q'.code.toByte()) return
                val numero = (2..4).joinToString("") { "%02X".format(d[it]) }
                val m = meilleur.get()
                if (m == null || r.rssi > m.first) meilleur.set(r.rssi to numero)
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
            dire("🏷 Kiosque ${trouve.second} entendu (${trouve.first} dBm).")
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
