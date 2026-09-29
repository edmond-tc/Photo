package bj.photocopie.envoyeur

import java.io.DataInputStream
import java.io.DataOutputStream
import java.io.IOException
import java.net.Socket
import java.nio.ByteBuffer
import java.security.KeyFactory
import java.security.KeyPairGenerator
import java.security.MessageDigest
import java.security.spec.ECGenParameterSpec
import java.security.spec.X509EncodedKeySpec
import javax.crypto.Cipher
import javax.crypto.KeyAgreement
import javax.crypto.spec.GCMParameterSpec
import javax.crypto.spec.SecretKeySpec
import org.json.JSONObject

/**
 * Canal chiffré de bout en bout entre deux téléphones (« Main à main »).
 *
 * 1. Chacun envoie une clé publique éphémère (courbe P-256) ; les deux
 *    calculent le même secret (ECDH). Personne d'autre ne peut le calculer.
 * 2. De ce secret sortent deux clés AES (une par sens) et le SYMBOLE affiché
 *    sur les deux écrans (animal, couleur, nombre). Un intrus placé au
 *    milieu obtiendrait un secret différent de chaque côté, donc deux
 *    symboles différents : le symbole identique en est la preuve.
 * 3. Chaque trame est chiffrée et authentifiée (AES-GCM) : un morceau
 *    abîmé ou modifié en route est refusé, jamais écrit.
 *
 * Trame : longueur (4 octets) puis contenu chiffré. Contenu clair : un
 * octet de type (JSON ou DONNEES) puis la charge.
 */
class MamCanal private constructor(
    private val socket: Socket,
    private val entree: DataInputStream,
    private val sortie: DataOutputStream,
    private val cleEnvoi: SecretKeySpec,
    private val cleReception: SecretKeySpec,
    /** Empreinte commune aux deux téléphones, d'où vient le symbole. */
    val empreinte: ByteArray,
) : AutoCloseable {

    companion object {
        const val TYPE_JSON: Byte = 1
        const val TYPE_DONNEES: Byte = 2

        /** Morceau de fichier par trame : assez gros pour la vitesse, assez petit pour la mémoire. */
        const val TAILLE_MORCEAU = 256 * 1024

        /** Garde-fou : une trame annoncée plus grosse est une attaque ou une panne. */
        private const val TRAME_MAX = TAILLE_MORCEAU + 1024

        /**
         * Poignée de main. `serveur` : le téléphone qui envoie (il a créé le
         * réseau). Les deux côtés calculent les mêmes clés, dans le même ordre.
         */
        fun ouvrir(socket: Socket, serveur: Boolean): MamCanal {
            socket.tcpNoDelay = true
            socket.sendBufferSize = 1 shl 20
            socket.receiveBufferSize = 1 shl 20
            val entree = DataInputStream(socket.getInputStream().buffered(1 shl 16))
            val sortie = DataOutputStream(socket.getOutputStream().buffered(1 shl 16))

            val generateur = KeyPairGenerator.getInstance("EC")
            generateur.initialize(ECGenParameterSpec("secp256r1"))
            val paire = generateur.generateKeyPair()
            val maCle = paire.public.encoded

            sortie.writeShort(maCle.size)
            sortie.write(maCle)
            sortie.flush()
            val longueur = entree.readUnsignedShort()
            if (longueur !in 32..512) throw IOException("Clé de l'autre téléphone invalide.")
            val saCle = ByteArray(longueur).also { entree.readFully(it) }

            val publique = KeyFactory.getInstance("EC").generatePublic(X509EncodedKeySpec(saCle))
            val accord = KeyAgreement.getInstance("ECDH")
            accord.init(paire.private)
            accord.doPhase(publique, true)
            val secret = accord.generateSecret()

            // Toujours dans l'ordre « envoyeur, receveur » : les deux côtés
            // calculent exactement les mêmes valeurs.
            val (cleServeur, cleClient) = if (serveur) maCle to saCle else saCle to maCle
            fun deriver(etiquette: String): ByteArray = MessageDigest.getInstance("SHA-256").run {
                update(etiquette.toByteArray())
                update(secret)
                update(cleServeur)
                update(cleClient)
                digest()
            }
            val versClient = SecretKeySpec(deriver("mam/v1/cle/vers-receveur"), "AES")
            val versServeur = SecretKeySpec(deriver("mam/v1/cle/vers-envoyeur"), "AES")
            return MamCanal(
                socket, entree, sortie,
                cleEnvoi = if (serveur) versClient else versServeur,
                cleReception = if (serveur) versServeur else versClient,
                empreinte = deriver("mam/v1/symbole"),
            )
        }

        private val ANIMAUX = listOf(
            "🐘" to "éléphant", "🦁" to "lion", "🐢" to "tortue", "🐟" to "poisson",
            "🦒" to "girafe", "🐓" to "coq", "🐐" to "chèvre", "🦜" to "perroquet",
            "🐊" to "crocodile", "🦋" to "papillon", "🐝" to "abeille", "🐒" to "singe",
            "🦓" to "zèbre", "🐌" to "escargot", "🦉" to "hibou", "🐬" to "dauphin",
        )
        private val COULEURS = listOf(
            Triple("rouge", "#d93025", "rouge"), Triple("orange", "#f28b00", "orange"),
            Triple("jaune", "#f2c200", "jaune"), Triple("vert", "#1e8e3e", "vert"),
            Triple("bleu", "#1a73e8", "bleu"), Triple("violet", "#8e44ad", "violet"),
            Triple("rose", "#e0529c", "rose"), Triple("marron", "#8d5524", "marron"),
        )
    }

    private var compteurEnvoi = 0L
    private var compteurReception = 0L
    private val chiffreur = Cipher.getInstance("AES/GCM/NoPadding")
    private val dechiffreur = Cipher.getInstance("AES/GCM/NoPadding")

    /** Le symbole commun : même animal, même couleur, même nombre des deux côtés. */
    fun symbole(): JSONObject {
        val (emoji, animal) = ANIMAUX[(empreinte[0].toInt() and 0xFF) % ANIMAUX.size]
        val (couleur, teinte, _) = COULEURS[(empreinte[1].toInt() and 0xFF) % COULEURS.size]
        val nombre = 10 + ((empreinte[2].toInt() and 0xFF) shl 8 or (empreinte[3].toInt() and 0xFF)) % 90
        return JSONObject()
            .put("emoji", emoji).put("animal", animal)
            .put("couleur", couleur).put("teinte", teinte)
            .put("nombre", nombre)
    }

    private fun nonce(compteur: Long): ByteArray =
        ByteBuffer.allocate(12).putInt(0).putLong(compteur).array()

    @Synchronized
    fun envoyer(type: Byte, charge: ByteArray, debut: Int = 0, longueur: Int = charge.size) {
        chiffreur.init(Cipher.ENCRYPT_MODE, cleEnvoi, GCMParameterSpec(128, nonce(compteurEnvoi++)))
        chiffreur.updateAAD(byteArrayOf(type))
        val chiffre = chiffreur.doFinal(charge, debut, longueur)
        sortie.writeInt(chiffre.size + 1)
        sortie.writeByte(type.toInt())
        sortie.write(chiffre)
    }

    fun envoyerJson(message: JSONObject) {
        envoyer(TYPE_JSON, message.toString().toByteArray(Charsets.UTF_8))
        vider()
    }

    @Synchronized
    fun vider() = sortie.flush()

    /** Une trame déchiffrée : (type, contenu). Lève une erreur si elle a été altérée. */
    fun recevoir(): Pair<Byte, ByteArray> {
        val longueur = entree.readInt()
        if (longueur !in 18..TRAME_MAX) throw IOException("Trame invalide ($longueur octets).")
        val type = entree.readByte()
        val chiffre = ByteArray(longueur - 1).also { entree.readFully(it) }
        dechiffreur.init(Cipher.DECRYPT_MODE, cleReception, GCMParameterSpec(128, nonce(compteurReception++)))
        dechiffreur.updateAAD(byteArrayOf(type))
        return type to dechiffreur.doFinal(chiffre)
    }

    fun recevoirJson(): JSONObject {
        val (type, contenu) = recevoir()
        if (type != TYPE_JSON) throw IOException("Message attendu, données reçues.")
        return JSONObject(String(contenu, Charsets.UTF_8))
    }

    override fun close() {
        try { socket.close() } catch (_: Exception) {}
    }
}
