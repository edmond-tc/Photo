package bj.photocopie.envoyeur

import android.annotation.SuppressLint
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothSocket
import android.content.Context
import org.json.JSONObject
import java.io.BufferedInputStream
import java.io.BufferedOutputStream
import java.io.DataInputStream
import java.io.IOException
import java.io.OutputStream
import java.nio.ByteBuffer
import java.util.UUID

/**
 * Le canal Bluetooth avec le PC du kiosque (voir `canal_bt.rs`) : une vraie
 * conversation, sans appairage. Le téléphone y donne le nom et le mot de
 * passe de son réseau, et le PC répond « relié, voici mon adresse » ; si
 * aucun Wi-Fi ne passe, la commande entière y passe, en plus lent.
 *
 * Trame : longueur (4 octets) puis en-tête JSON, saut de ligne, corps brut.
 */
class CanalBt(private val ctx: Context, private val dire: (String) -> Unit) {
    companion object {
        /** Le même que `canal_bt.rs` (SERVICE_UUID). */
        val SERVICE: UUID = UUID.fromString("6b1f0c2e-8a4d-4f3b-9c55-4b5051434f50")
        private const val TAILLE_MAX = 8 * 1024 * 1024
    }

    private var socket: BluetoothSocket? = null
    private var entree: DataInputStream? = null
    private var sortie: OutputStream? = null
    private var adresse: String? = null

    val ouvert: Boolean get() = socket?.isConnected == true

    /** Se relie au PC d'adresse Bluetooth [mac] (lue dans sa balise). Deux essais. */
    fun ouvrir(mac: String): Boolean {
        adresse = mac
        repeat(2) { essai ->
            if (relier(mac)) return true
            if (essai == 0) Thread.sleep(1500)
        }
        return false
    }

    @SuppressLint("MissingPermission")
    private fun relier(mac: String): Boolean {
        val adaptateur = ctx.getSystemService(BluetoothManager::class.java)?.adapter ?: return false
        if (!adaptateur.isEnabled || !BluetoothAdapter.checkBluetoothAddress(mac)) return false
        return try {
            fermer()
            adaptateur.cancelDiscovery()
            val s = adaptateur.getRemoteDevice(mac).createInsecureRfcommSocketToServiceRecord(SERVICE)
            s.connect()
            socket = s
            entree = DataInputStream(BufferedInputStream(s.inputStream))
            sortie = BufferedOutputStream(s.outputStream)
            val (r, _) = echanger(JSONObject().put("t", "bonjour"), null)
            r.optString("t") == "bonjour"
        } catch (e: Exception) {
            dire("⚠️ Canal Bluetooth : ${e.message}")
            fermer()
            false
        }
    }

    /** Un message, sa réponse. Un seul à la fois. */
    @Synchronized
    fun echanger(entete: JSONObject, corps: ByteArray?): Pair<JSONObject, ByteArray> {
        val o = sortie ?: throw IOException("canal fermé")
        val i = entree ?: throw IOException("canal fermé")
        val tete = entete.toString().toByteArray(Charsets.UTF_8)
        val c = corps ?: ByteArray(0)
        o.write(ByteBuffer.allocate(4).putInt(tete.size + 1 + c.size).array())
        o.write(tete)
        o.write('\n'.code)
        o.write(c)
        o.flush()
        val longueur = i.readInt()
        if (longueur < 0 || longueur > TAILLE_MAX) throw IOException("réponse invalide")
        val contenu = ByteArray(longueur)
        i.readFully(contenu)
        val fin = contenu.indexOf('\n'.code.toByte())
        if (fin < 0) throw IOException("réponse invalide")
        return JSONObject(String(contenu, 0, fin, Charsets.UTF_8)) to contenu.copyOfRange(fin + 1, contenu.size)
    }

    /** Comme [echanger], en se reliant de nouveau une fois si le canal a sauté. */
    @Synchronized
    fun echangerAvecReprise(entete: JSONObject, corps: ByteArray?): Pair<JSONObject, ByteArray> = try {
        echanger(entete, corps)
    } catch (e: IOException) {
        val mac = adresse ?: throw e
        dire("↻ Canal Bluetooth coupé : nouvelle liaison.")
        if (!relier(mac)) throw e
        echanger(entete, corps)
    }

    fun fermer() {
        try { socket?.close() } catch (_: Exception) {}
        socket = null
        entree = null
        sortie = null
    }
}
