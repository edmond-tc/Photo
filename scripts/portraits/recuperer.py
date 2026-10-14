"""Portraits des auteurs (mot du jour) et des figures de la physique
(inspiration de la semaine), pour le Portail Amphi, sans Internet ensuite.

Lancé par .github/workflows/portraits.yml (les machines de GitHub ont accès
à Wikimedia). Ne garde QUE des images libres de Wikimedia Commons (domaine
public, CC0, CC BY, CC BY-SA), avec leur auteur et leur licence dans
credits.json. Les images non libres (« fair use ») sont ignorées : la page
affiche alors les initiales.
"""
import io, json, os, re, sys, time, unicodedata, urllib.parse, urllib.request
from PIL import Image

PERSONNES = {
    "Nelson Mandela": "Nelson_Mandela", "Amadou Hampâté Bâ": "Amadou_Hampâté_Bâ", "Jean de La Fontaine": "Jean_de_La_Fontaine",
    "Nicolas Boileau": "Nicolas_Boileau-Despréaux", "Confucius": "Confucius", "Voltaire": "Voltaire", "Blaise Pascal": "Blaise_Pascal",
    "Pierre Corneille": "Pierre_Corneille", "Lao-tseu": "Laozi", "Sénèque": "Seneca_the_Younger", "Thomas Edison": "Thomas_Edison",
    "Marie Curie": "Marie_Curie", "Albert Einstein": "Albert_Einstein", "René Descartes": "René_Descartes", "François Rabelais": "François_Rabelais",
    "Antoine Lavoisier": "Antoine_Lavoisier", "Jean Racine": "Jean_Racine", "Friedrich Nietzsche": "Friedrich_Nietzsche",
    "Abdus Salam": "Abdus_Salam", "Francis Allotey": "Francis_Allotey", "Cheikh Anta Diop": "Cheikh_Anta_Diop", "Thomas Mensah": "Thomas_Mensah",
    "Philip Emeagwali": "Philip_Emeagwali", "Max Planck": "Max_Planck", "Erwin Schrödinger": "Erwin_Schrödinger", "Werner Heisenberg": "Werner_Heisenberg",
    "Paul Dirac": "Paul_Dirac", "Ludwig Boltzmann": "Ludwig_Boltzmann", "Satyendra Nath Bose": "Satyendra_Nath_Bose", "Enrico Fermi": "Enrico_Fermi",
    "Emmy Noether": "Emmy_Noether", "Joseph-Louis Lagrange": "Joseph-Louis_Lagrange", "William Rowan Hamilton": "William_Rowan_Hamilton",
    "Joseph Fourier": "Joseph_Fourier", "Lawrence Bragg": "Lawrence_Bragg", "Daniel Bernoulli": "Daniel_Bernoulli", "Osborne Reynolds": "Osborne_Reynolds",
    "Subrahmanyan Chandrasekhar": "Subrahmanyan_Chandrasekhar", "Lise Meitner": "Lise_Meitner", "Chien-Shiung Wu": "Chien-Shiung_Wu",
    "Richard Feynman": "Richard_Feynman",
}
UA = {"User-Agent": "PortailAmphi/1.0 (https://github.com/edmond-tc/Photo; atinzed2@gmail.com)"}
LIBRES = re.compile(r"^(public domain|pd|cc0|cc[ -]by)", re.I)

def cle(nom):
    t = unicodedata.normalize("NFD", nom.lower())
    t = "".join(c for c in t if unicodedata.category(c) != "Mn")
    return re.sub(r"[^a-z0-9]+", "-", t).strip("-")

def api(base, params):
    url = base + "?" + urllib.parse.urlencode(dict(params, format="json"))
    with urllib.request.urlopen(urllib.request.Request(url, headers=UA), timeout=30) as r:
        return json.load(r)

def texte_brut(h):
    return re.sub(r"\s+", " ", re.sub(r"<[^>]+>", "", h or "")).strip()[:120]

os.makedirs("sortie", exist_ok=True)
credits = {}
for nom, titre in PERSONNES.items():
    try:
        pages = api("https://en.wikipedia.org/w/api.php", {"action": "query", "titles": titre, "prop": "pageimages", "piprop": "name", "redirects": 1})["query"]["pages"]
        fichier = next(iter(pages.values())).get("pageimage")
        if not fichier:
            print("pas d'image :", nom); continue
        info = api("https://commons.wikimedia.org/w/api.php", {"action": "query", "titles": "File:" + fichier, "prop": "imageinfo", "iiprop": "url|extmetadata", "iiurlwidth": 320})["query"]["pages"]
        page = next(iter(info.values()))
        if "imageinfo" not in page:
            print("pas sur Commons (non libre ?) :", nom); continue
        ii = page["imageinfo"][0]
        meta = ii.get("extmetadata", {})
        licence = (meta.get("LicenseShortName") or {}).get("value", "")
        if not LIBRES.search(licence):
            print("licence refusée :", nom, licence); continue
        with urllib.request.urlopen(urllib.request.Request(ii["thumburl"], headers=UA), timeout=60) as r:
            im = Image.open(io.BytesIO(r.read())).convert("RGB")
        # Recadrage carré centré sur le haut (le visage), 256 × 256.
        c = min(im.width, im.height)
        haut = 0 if im.height > im.width else (im.height - c) // 2
        im = im.crop(((im.width - c) // 2, haut, (im.width - c) // 2 + c, haut + c)).resize((256, 256), Image.LANCZOS)
        k = cle(nom)
        im.save(f"sortie/{k}.jpg", quality=82, optimize=True)
        credits[k] = {"nom": nom, "licence": licence, "auteur": texte_brut((meta.get("Artist") or {}).get("value", "")), "source": ii.get("descriptionurl", "")}
        print("ok :", nom, licence)
        time.sleep(0.5)
    except Exception as e:
        print("échec :", nom, e)
json.dump(credits, open("sortie/credits.json", "w"), ensure_ascii=False, indent=1)
print(len(credits), "portraits")
