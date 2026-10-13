package bj.photocopie.envoyeur

import java.util.Locale

/**
 * Textes affichés par Android lui-même (notifications, menu de partage) : en
 * anglais si le téléphone est réglé en anglais. Les écrans, eux, sont dans la
 * page (client-web), qui a son propre dictionnaire (langue-en.json).
 */
internal fun tr(fr: String, en: String): String =
    if (Locale.getDefault().language == "en") en else fr
