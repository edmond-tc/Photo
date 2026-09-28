package bj.photocopie.envoyeur

import android.annotation.SuppressLint
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.bluetooth.BluetoothManager
import android.bluetooth.le.BluetoothLeScanner
import android.bluetooth.le.ScanFilter
import android.bluetooth.le.ScanResult
import android.bluetooth.le.ScanSettings
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.os.Build

/**
 * Écoute de la balise Bluetooth du kiosque, application fermée.
 *
 * Android fait le travail lui-même (balayage confié au système, avec un
 * filtre) et réveille `BalayageRecepteur` seulement quand il entend NOTRE
 * balise : pas de service qui tourne, pas de batterie consommée pour rien.
 */
object Balayage {
    private const val ACTION = "bj.photocopie.envoyeur.BALISE"

    private fun intention(ctx: Context): PendingIntent = PendingIntent.getBroadcast(
        ctx,
        1,
        Intent(ctx, BalayageRecepteur::class.java).setAction(ACTION),
        // Modifiable : Android y ajoute les résultats du balayage.
        PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_MUTABLE,
    )

    /** Rend un message d'état, affiché dans l'application. */
    @SuppressLint("MissingPermission")
    fun demarrer(ctx: Context): String {
        val adaptateur = ctx.getSystemService(BluetoothManager::class.java)?.adapter
            ?: return "Pas de Bluetooth sur ce téléphone : pas de proposition automatique."
        if (!adaptateur.isEnabled) {
            return "Bluetooth éteint : le téléphone ne peut pas entendre le kiosque. L'envoi reste possible en ouvrant l'application."
        }
        val scanner = adaptateur.bluetoothLeScanner ?: return "Bluetooth indisponible pour l'instant."
        val filtre = ScanFilter.Builder()
            .setManufacturerData(Reglages.FABRICANT_BLE, Reglages.DONNEES_BLE)
            .build()
        val reglages = ScanSettings.Builder()
            .setScanMode(ScanSettings.SCAN_MODE_LOW_POWER)
            .build()
        return try {
            scanner.stopScan(intention(ctx))
            val code = scanner.startScan(listOf(filtre), reglages, intention(ctx))
            if (code == 0) "Écoute du kiosque active." else "Écoute du kiosque refusée (code $code)."
        } catch (e: SecurityException) {
            "Autorisation Bluetooth manquante : touchez « Autoriser »."
        }
    }
}

class BalayageRecepteur : BroadcastReceiver() {
    override fun onReceive(ctx: Context, intent: Intent) {
        val resultats: List<ScanResult> = if (Build.VERSION.SDK_INT >= 33) {
            intent.getParcelableArrayListExtra(BluetoothLeScanner.EXTRA_LIST_SCAN_RESULT, ScanResult::class.java)
        } else {
            @Suppress("DEPRECATION")
            intent.getParcelableArrayListExtra(BluetoothLeScanner.EXTRA_LIST_SCAN_RESULT)
        }.orEmpty()
        val meilleur = resultats.maxOfOrNull { it.rssi } ?: return
        // Numéro du kiosque le plus proche (octets après « KQ » dans la balise).
        resultats.maxByOrNull { it.rssi }
            ?.scanRecord?.getManufacturerSpecificData(Reglages.FABRICANT_BLE)
            ?.takeIf { it.size >= 5 && it[0] == 'K'.code.toByte() && it[1] == 'Q'.code.toByte() }
            ?.let { d ->
                val numero = (2..4).joinToString("") { "%02X".format(d[it]) }
                Reglages.noterKiosque(ctx, numero, System.currentTimeMillis())
            }
        val maintenant = System.currentTimeMillis()
        Reglages.noterSignal(ctx, meilleur, maintenant)

        // Au guichet seulement.
        if (meilleur < Reglages.seuilBle(ctx)) return

        // Une seule proposition par visite : tant que le téléphone reste
        // près du guichet (le gérant, un client qui attend), la balise est
        // entendue sans cesse et on ne redit rien. Il faut s'être éloigné
        // un moment pour être de nouveau prévenu en revenant.
        val precedent = Reglages.dernierPassageAuGuichet(ctx)
        Reglages.noterPassageAuGuichet(ctx, maintenant)
        if (maintenant - precedent < Reglages.ABSENCE_AVANT_NOUVELLE_PROPOSITION_MS) return
        if (MainActivity.visible) return
        if (maintenant - Reglages.derniereNotification(ctx) < Reglages.PAUSE_NOTIFICATION_MS) return
        Reglages.noterNotification(ctx, maintenant)
        proposerEnvoi(ctx, meilleur)
    }

    private fun proposerEnvoi(ctx: Context, rssi: Int) {
        val gestionnaire = ctx.getSystemService(NotificationManager::class.java) ?: return
        gestionnaire.createNotificationChannel(
            NotificationChannel("kiosque", "Kiosque à côté", NotificationManager.IMPORTANCE_HIGH)
        )
        val ouvrir = PendingIntent.getActivity(
            ctx,
            2,
            Intent(ctx, MainActivity::class.java)
                .setAction(MainActivity.ACTION_CHOISIR)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        val notification = Notification.Builder(ctx, "kiosque")
            .setSmallIcon(R.drawable.ic_notification)
            .setColor(0xFF1F4FD1.toInt())
            .setContentTitle("Kiosque photocopie à côté")
            .setContentText("Touchez pour envoyer un document au kiosque.")
            .setSubText("signal $rssi dBm")
            .setContentIntent(ouvrir)
            .setAutoCancel(true)
            .build()
        try {
            gestionnaire.notify(1, notification)
        } catch (_: SecurityException) {
        }
    }
}

/** Le balayage confié au système s'arrête au redémarrage du téléphone : on le relance. */
class DemarrageRecepteur : BroadcastReceiver() {
    override fun onReceive(ctx: Context, intent: Intent) {
        Balayage.demarrer(ctx)
    }
}
