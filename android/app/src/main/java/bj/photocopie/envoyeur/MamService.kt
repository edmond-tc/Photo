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

/**
 * Garde « Main à main » en vie pendant un transfert, même écran éteint ou
 * application en arrière-plan : sans lui, Android coupe le Wi-Fi ou le
 * processus au milieu d'un film de 2 Go. Une notification montre l'avancée.
 */
class MamService : Service() {
    companion object {
        private const val CANAL = "main_a_main"
        private const val ID = 4174

        fun demarrer(ctx: Context, texte: String) {
            try {
                ctx.startForegroundService(Intent(ctx, MamService::class.java).putExtra("texte", texte))
            } catch (_: Exception) {
                // Démarrage refusé (application en arrière-plan) : le transfert
                // continue tant que l'écran reste allumé.
            }
        }

        fun maj(ctx: Context, texte: String, pourcent: Int?) {
            val nm = ctx.getSystemService(NotificationManager::class.java) ?: return
            try { nm.notify(ID, notification(ctx, texte, pourcent)) } catch (_: Exception) {}
        }

        fun arreter(ctx: Context) {
            ctx.stopService(Intent(ctx, MamService::class.java))
        }

        private fun notification(ctx: Context, texte: String, pourcent: Int?): Notification {
            val nm = ctx.getSystemService(NotificationManager::class.java)
            if (nm?.getNotificationChannel(CANAL) == null) {
                nm?.createNotificationChannel(
                    NotificationChannel(CANAL, "Main à main", NotificationManager.IMPORTANCE_LOW)
                        .apply { description = "Avancée des envois et réceptions entre téléphones" }
                )
            }
            val ouvrir = PendingIntent.getActivity(
                ctx, 0, Intent(ctx, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )
            return Notification.Builder(ctx, CANAL)
                .setSmallIcon(R.drawable.ic_notification)
                .setContentTitle("Envoyeur Kiosque")
                .setContentText(texte)
                .setContentIntent(ouvrir)
                .setOngoing(true)
                .setOnlyAlertOnce(true)
                .apply { if (pourcent != null) setProgress(100, pourcent.coerceIn(0, 100), false) else setProgress(0, 0, true) }
                .build()
        }
    }

    private var eveil: PowerManager.WakeLock? = null
    private var wifi: WifiManager.WifiLock? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val texte = intent?.getStringExtra("texte") ?: "Main à main"
        val n = notification(this, texte, null)
        if (Build.VERSION.SDK_INT >= 29) startForeground(ID, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
        else startForeground(ID, n)
        if (eveil == null) {
            eveil = getSystemService(PowerManager::class.java)
                ?.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "envoyeur:main-a-main")
                ?.apply { acquire(3 * 60 * 60 * 1000L) }
        }
        if (wifi == null) {
            @Suppress("DEPRECATION")
            wifi = applicationContext.getSystemService(WifiManager::class.java)
                ?.createWifiLock(WifiManager.WIFI_MODE_FULL_LOW_LATENCY, "envoyeur:main-a-main")
                ?.apply { acquire() }
        }
        return START_NOT_STICKY
    }

    override fun onDestroy() {
        try { eveil?.takeIf { it.isHeld }?.release() } catch (_: Exception) {}
        try { wifi?.takeIf { it.isHeld }?.release() } catch (_: Exception) {}
        eveil = null
        wifi = null
        super.onDestroy()
    }

    override fun onBind(intent: Intent?): IBinder? = null
}
