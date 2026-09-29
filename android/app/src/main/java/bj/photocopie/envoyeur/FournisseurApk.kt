package bj.photocopie.envoyeur

import android.content.ContentProvider
import android.content.ContentValues
import android.database.Cursor
import android.database.MatrixCursor
import android.net.Uri
import android.os.ParcelFileDescriptor
import android.provider.OpenableColumns
import java.io.File

/**
 * « Donner l'application à un ami » : prête le fichier d'installation de
 * CETTE application, en lecture seule, à l'application choisie pour le
 * partage (Quick Share, Bluetooth…). Sans internet, sans forfait.
 * Rien d'autre n'est lisible par ici.
 */
class FournisseurApk : ContentProvider() {
    companion object {
        const val AUTORITE = "bj.photocopie.envoyeur.apk"
        const val NOM = "EnvoyeurKiosque.apk"
        const val TYPE = "application/vnd.android.package-archive"
        val ADRESSE: Uri = Uri.parse("content://$AUTORITE/$NOM")
    }

    override fun onCreate(): Boolean = true

    private fun fichier(): File = File(context!!.applicationInfo.sourceDir)

    override fun openFile(uri: Uri, mode: String): ParcelFileDescriptor {
        if (uri != ADRESSE || mode != "r") throw SecurityException("Lecture seule du fichier d'installation.")
        return ParcelFileDescriptor.open(fichier(), ParcelFileDescriptor.MODE_READ_ONLY)
    }

    override fun getType(uri: Uri): String = TYPE

    override fun query(uri: Uri, projection: Array<out String>?, selection: String?, args: Array<out String>?, tri: String?): Cursor {
        val colonnes = projection ?: arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE)
        return MatrixCursor(colonnes).apply {
            addRow(colonnes.map { c ->
                when (c) {
                    OpenableColumns.DISPLAY_NAME -> NOM
                    OpenableColumns.SIZE -> fichier().length()
                    else -> null
                }
            })
        }
    }

    override fun insert(uri: Uri, valeurs: ContentValues?): Uri? = null
    override fun delete(uri: Uri, selection: String?, args: Array<out String>?): Int = 0
    override fun update(uri: Uri, valeurs: ContentValues?, selection: String?, args: Array<out String>?): Int = 0
}
