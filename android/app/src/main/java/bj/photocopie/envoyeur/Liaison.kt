package bj.photocopie.envoyeur

import android.annotation.SuppressLint
import android.content.Context
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
 * Seconde méthode (secours, PC qui ne sait pas créer de Wi-Fi) :
 * 1. le téléphone crée son propre réseau (Wi-Fi Direct), avec le mot de
 *    passe de la boutique ;
 * 2. le PC du kiosque le repère et le rejoint tout seul ;
 * 3. le téléphone trouve l'adresse du PC.
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

    /** Le nom du réseau créé, pour le journal. */
    var nomReseau: String? = null
        private set

    /** Crée le réseau et attend le PC. Rend son adresse, ou null (et la raison dans [raison]). */
    fun ouvrir(): InetAddress? {
        if (!attendreWifi()) {
            raison = "Le Wi-Fi est resté éteint. Allumez-le, puis rejoignez le Wi-Fi de la boutique."
            return null
        }
        val debut = System.currentTimeMillis()
        // Le téléphone doit être sur le Wi-Fi de la boutique (créé par le PC,
        // allumé en permanence). L'ancienne seconde méthode — le téléphone
        // crée son réseau et le PC le rejoint — est retirée : une seule
        // carte Wi-Fi ne peut pas faire les deux à la fois. On attend donc
        // que le client rejoigne le Wi-Fi (QR du guichet ou liste Wi-Fi).
        dire("📶 En attente du Wi-Fi de la boutique…")
        val fin = debut + 90_000
        while (System.currentTimeMillis() < fin) {
            pcSurLeWifi()?.let { pc ->
                val secondes = (System.currentTimeMillis() - debut) / 1000.0
                dire("✅ Téléphone sur le Wi-Fi de la boutique : PC à ${pc.hostAddress} (${"%.1f".format(secondes)} s).")
                return pc
            }
            Thread.sleep(2000)
        }
        raison = "Rejoignez d'abord le Wi-Fi de la boutique : scannez le code du guichet, ou choisissez son nom dans vos réseaux Wi-Fi. Puis revenez ici."
        dire("❌ Pas sur le Wi-Fi de la boutique après 90 s.")
        return null
    }

    var raison: String? = null
        private set

    /** Supprime le réseau : le PC est libéré pour le client suivant. */
    fun fermer() {
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
