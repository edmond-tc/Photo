package bj.photocopie.envoyeur

import android.content.Context

/**
 * Ce que l'envoyeur et le logiciel du PC doivent avoir en commun. Les
 * valeurs par défaut sont celles du PC (voir `reception_directe.rs`).
 */
object Reglages {
    /** Mot de passe du réseau que crée le téléphone : celui que le PC essaie. */
    const val MOT_DE_PASSE_PAR_DEFAUT = "kiosque2026"

    /** Début du nom du réseau : le PC reconnaît ainsi l'envoyeur et le sert en premier. */
    const val PREFIXE_RESEAU = "DIRECT-KQ-"

    /** Port de la page d'envoi du PC. */
    const val PORT_PC = 4173

    /** Port où le PC annonce sa présence une fois connecté (voir `reception_directe.rs`). */
    const val PORT_BALISE = 48173

    /** Balise Bluetooth du kiosque : identifiant « essais » de la norme, et deux lettres. */
    const val FABRICANT_BLE = 0xFFFF
    val DONNEES_BLE = byteArrayOf('K'.code.toByte(), 'Q'.code.toByte())

    /**
     * Appel du téléphone au PC (voir `appel_ble.rs`) : « KC », numéro du
     * kiosque (3 octets, zéros si inconnu), puis les 4 lettres qui terminent
     * le nom du réseau. Le PC en déduit le nom et le mot de passe, et rejoint
     * sans chercher.
     */
    val APPEL_BLE = byteArrayOf('K'.code.toByte(), 'C'.code.toByte())

    /** Données de l'appel pour le réseau [nom] (« DIRECT-KQ-3FA92C-AB2Z » ou « DIRECT-KQ-AB2Z »). */
    fun donneesAppel(nom: String): ByteArray? {
        val reste = nom.removePrefix(PREFIXE_RESEAU)
        val (kiosque, suffixe) = if ('-' in reste) reste.substringBefore('-') to reste.substringAfter('-') else null to reste
        if (suffixe.length != 4) return null
        val numero = kiosque?.let { k ->
            if (k.length != 6) return null
            (0..2).map { (k.substring(it * 2, it * 2 + 2).toIntOrNull(16) ?: return null).toByte() }
        } ?: listOf(0, 0, 0).map { it.toByte() }
        return APPEL_BLE + numero.toByteArray() + suffixe.toByteArray(Charsets.US_ASCII)
    }

    /**
     * Force minimale de la balise (dBm) pour proposer l'envoi : le téléphone
     * doit être au guichet, pas dans la cage d'à côté. À régler pendant l'essai.
     */
    const val SEUIL_BLE_PAR_DEFAUT = -75

    /** Pas plus d'une proposition toutes les 10 minutes. */
    const val PAUSE_NOTIFICATION_MS = 10 * 60 * 1000L

    /**
     * Balise pas entendue au guichet depuis ce délai : le téléphone était
     * parti, et on peut de nouveau proposer l'envoi à son retour.
     */
    const val ABSENCE_AVANT_NOUVELLE_PROPOSITION_MS = 30 * 60 * 1000L

    /**
     * Un code par kiosque (voir `reception_directe.rs`) : la balise donne le
     * numéro du kiosque où se trouve le client ; le réseau est créé pour ce
     * kiosque, avec son mot de passe, et le kiosque voisin l'ignore.
     */
    const val SECRET_KIOSQUE = "photocopie-benin/kiosque/v1/8d41c7e2b9f05a36"

    /** Balise entendue depuis moins longtemps : le client est à ce kiosque. */
    private const val KIOSQUE_RECENT_MS = 10 * 60 * 1000L

    fun motDePasseKiosque(numero: String): String =
        java.security.MessageDigest.getInstance("SHA-256")
            .digest("$SECRET_KIOSQUE:$numero".toByteArray(Charsets.UTF_8))
            .take(8)
            .joinToString("") { "%02x".format(it) }

    fun noterKiosque(ctx: Context, numero: String, quand: Long) {
        prefs(ctx).edit().putString("kiosque", numero).putLong("kiosque_quand", quand).apply()
    }

    /** Numéro du kiosque entendu récemment, ou null (réseau « sans numéro »). */
    fun kiosqueRecent(ctx: Context): String? {
        val p = prefs(ctx)
        val quand = p.getLong("kiosque_quand", 0L)
        if (System.currentTimeMillis() - quand > KIOSQUE_RECENT_MS) return null
        return p.getString("kiosque", null)?.takeIf { it.length == 6 }
    }

    /** Adresse Bluetooth du PC (6 derniers octets de sa balise), pour le canal Bluetooth. */
    fun noterAdressePc(ctx: Context, mac: String, quand: Long) {
        prefs(ctx).edit().putString("adresse_pc", mac).putLong("adresse_pc_quand", quand).apply()
    }

    fun adressePcRecente(ctx: Context): String? {
        val p = prefs(ctx)
        if (System.currentTimeMillis() - p.getLong("adresse_pc_quand", 0L) > KIOSQUE_RECENT_MS) return null
        return p.getString("adresse_pc", null)
    }

    private fun prefs(ctx: Context) = ctx.getSharedPreferences("reglages", Context.MODE_PRIVATE)

    fun motDePasse(ctx: Context): String =
        prefs(ctx).getString("mdp", null)?.takeIf { it.length >= 8 } ?: MOT_DE_PASSE_PAR_DEFAUT

    fun definirMotDePasse(ctx: Context, mdp: String) {
        prefs(ctx).edit().putString("mdp", mdp).apply()
    }

    fun seuilBle(ctx: Context): Int = prefs(ctx).getInt("seuil_ble", SEUIL_BLE_PAR_DEFAUT)

    fun definirSeuilBle(ctx: Context, seuil: Int) {
        prefs(ctx).edit().putInt("seuil_ble", seuil).apply()
    }

    fun derniereNotification(ctx: Context): Long = prefs(ctx).getLong("derniere_notif", 0L)

    fun noterNotification(ctx: Context, quand: Long) {
        prefs(ctx).edit().putLong("derniere_notif", quand).apply()
    }

    fun dernierPassageAuGuichet(ctx: Context): Long = prefs(ctx).getLong("dernier_passage", 0L)

    fun noterPassageAuGuichet(ctx: Context, quand: Long) {
        prefs(ctx).edit().putLong("dernier_passage", quand).apply()
    }

    /** Dernier signal de balise entendu, affiché dans l'application pour régler le seuil. */
    fun noterSignal(ctx: Context, rssi: Int, quand: Long) {
        prefs(ctx).edit().putInt("dernier_rssi", rssi).putLong("dernier_rssi_quand", quand).apply()
    }

    fun dernierSignal(ctx: Context): Pair<Int, Long>? {
        val p = prefs(ctx)
        val quand = p.getLong("dernier_rssi_quand", 0L)
        return if (quand == 0L) null else p.getInt("dernier_rssi", 0) to quand
    }
}
