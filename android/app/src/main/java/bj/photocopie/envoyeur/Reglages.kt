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
     * Force minimale de la balise (dBm) pour proposer l'envoi : le téléphone
     * doit être au guichet, pas dans la cage d'à côté. À régler pendant l'essai.
     */
    const val SEUIL_BLE_PAR_DEFAUT = -75

    /** Pas plus d'une proposition toutes les 10 minutes. */
    const val PAUSE_NOTIFICATION_MS = 10 * 60 * 1000L

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
