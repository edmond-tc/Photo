package bj.photocopie.envoyeur

import android.annotation.SuppressLint
import android.content.Context
import android.net.wifi.WifiManager
import android.net.wifi.p2p.WifiP2pConfig
import android.net.wifi.p2p.WifiP2pManager
import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.Inet4Address
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.NetworkInterface
import java.net.Socket
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference

/**
 * Le lien entre le téléphone et le PC du kiosque, sans internet :
 * 1. le téléphone crée son propre réseau (Wi-Fi Direct), avec le mot de
 *    passe de la boutique — rien à allumer à la main, pas de forfait utilisé ;
 * 2. le PC du kiosque le repère et le rejoint tout seul ;
 * 3. le téléphone trouve l'adresse du PC.
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
        if (p2p == null || canal == null) {
            raison = "Ce téléphone ne sait pas créer de réseau Wi-Fi Direct."
            return null
        }
        if (!attendreWifi()) {
            raison = "Le Wi-Fi est resté éteint. Allumez-le (sans choisir de réseau)."
            return null
        }
        val debut = System.currentTimeMillis()
        val nom = creerReseau()
        if (nom == null) {
            raison = "Le téléphone n'a pas pu créer son réseau. Réessayez."
            return null
        }
        nomReseau = nom
        dire("📶 Réseau « $nom » créé. Le PC du kiosque va le rejoindre…")
        val pc = trouverPc(90_000)
        if (pc == null) {
            raison = "L'ordinateur de la boutique ne s'est pas relié. Approchez-vous du guichet et vérifiez que le logiciel est ouvert."
            dire("❌ Le PC ne s'est pas connecté en 90 s.")
            supprimerReseau()
            return null
        }
        val secondes = (System.currentTimeMillis() - debut) / 1000.0
        dire("✅ PC trouvé à ${pc.hostAddress} (${"%.1f".format(secondes)} s).")
        return pc
    }

    var raison: String? = null
        private set

    /** Supprime le réseau : le PC est libéré pour le client suivant. */
    fun fermer() {
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
