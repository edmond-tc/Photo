// Signe un installateur de Gestion Photocopie pour que le logiciel du PC le
// reconnaisse comme une mise à jour officielle, quel que soit le chemin par
// lequel il arrive (WhatsApp, Bluetooth, câble, clé USB).
//
// Usage : node scripts/signer-mise-a-jour.mjs <installateur.exe> <version>
// La clé privée (32 octets en hexadécimal) est lue dans CLE_SIGNATURE_MAJ,
// un secret GitHub : elle n'est jamais dans le dépôt.
//
// Ce qui est ajouté à la fin du fichier (88 octets, voir signature_maj.rs) :
//   version (16 octets, complétée par des zéros)
//   signature Ed25519 (64 octets) de
//     "GESTION-PHOTOCOPIE-MAJ|<version>|<SHA-256 en hexadécimal du fichier d'origine>"
//   marque "KQMAJ001" (8 octets)
// L'installateur Windows ignore ce qui suit ses propres données : il
// s'installe exactement comme avant (vérifié par le build).

import crypto from "node:crypto";
import fs from "node:fs";

const MARQUE = Buffer.from("KQMAJ001", "ascii");
const [fichier, version] = process.argv.slice(2);
const graine = (process.env.CLE_SIGNATURE_MAJ || "").trim();

if (!fichier || !/^[0-9A-Za-z.\-]{1,16}$/.test(version || "")) {
  console.error("Usage : node signer-mise-a-jour.mjs <installateur.exe> <version>");
  process.exit(2);
}
if (!/^[0-9a-fA-F]{64}$/.test(graine)) {
  console.error("CLE_SIGNATURE_MAJ absente ou invalide (64 caractères hexadécimaux attendus).");
  process.exit(3);
}

const contenu = fs.readFileSync(fichier);
if (contenu.length > MARQUE.length && contenu.subarray(-MARQUE.length).equals(MARQUE)) {
  console.error(`${fichier} est déjà signé.`);
  process.exit(4);
}

const cle = crypto.createPrivateKey({
  key: Buffer.concat([Buffer.from("302e020100300506032b657004220420", "hex"), Buffer.from(graine, "hex")]),
  format: "der",
  type: "pkcs8",
});
const empreinte = crypto.createHash("sha256").update(contenu).digest("hex");
const message = Buffer.from(`GESTION-PHOTOCOPIE-MAJ|${version}|${empreinte}`, "utf8");
const signature = crypto.sign(null, message, cle);

const champVersion = Buffer.alloc(16);
champVersion.write(version, "ascii");
fs.appendFileSync(fichier, Buffer.concat([champVersion, signature, MARQUE]));
console.log(`${fichier} signé : version ${version}, SHA-256 d'origine ${empreinte}`);
