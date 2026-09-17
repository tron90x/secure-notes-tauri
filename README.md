# SecureNotes — Tauri + Rust (coffre chiffré)

App de prise de notes sophistiquée, 100% locale, chiffrée.

## Sécurité
- Base `*.snotevault` dans `~/Documents/SecureNotes/` : JSON chiffré **AES-256-GCM**
- Clé dérivée par **Argon2id (m=64MB, t=3, p=1)** + sel 16 octets ; mot de passe de base **min 8 caractères** avec jauge de force
- Chiffrement individuel optionnel par note (2e mot de passe, min 4) + **titre/contenu/pièce jointe masqués** tant que verrouillée ; exclue de la recherche par mot-clé
- Zéro-connaissance : mot de passe jamais stocké, clé en RAM seulement, **zeroize à chaque verrouillage**
- **Verrouillage auto après inactivité** (1 min … 1 h, défaut 5 min), appliqué côté backend à chaque commande + côté UI

## Fonctionnalités
1. **Splash animé** : logo pop + barre dégradé + particules canvas + orbes blur
2. **Vault** : créer base (nom + mdp fort) / lister / déchiffrer
3. **Profils** : nom + `created_at`, ajout/suppression (notes → corbeille)
4. **Notes** : titre, contenu, `created_at`, `modified_history[]`, ajout/modif/**corbeille**
5. **Corbeille** : soft-delete, restauration, destruction définitive (avec double confirmation), vidage global
6. **Versions** : snapshot du contenu à chaque modification (20 max), restauration en 1 clic
7. **Recherche avancée** : mots-clés + filtre profil + dates (les notes verrouillées sont ignorées par la recherche texte)
8. **Surlignage** : sélection + 5 couleurs, persisté, suppression au clic
9. **Note verrouillée** : mot de passe individuel à la création
10. **Pièce jointe** : fichier → base64 stocké dans DB chiffrée
11. **Wallpaper par note** : aurora/sunset/ocean/neon/sakura/carbon + toggle **glass** (appliqué à la vignette ET à la popup)
12. **Lecture agrandie** : bouton ＋, grand rectangle avec fond + marquages
13. **Modales système** : plus aucun `alert/confirm/prompt` natif
14. **📊 Dashboard** (bouton topbar, Échap pour fermer) : 8 KPIs, notes par profil,
    donut verrouillées/libres, courbe d'activité 30j, surlignages par couleur,
    fonds utilisés, heatmap 12 semaines, top 5 longues notes, panneau sécurité —
    calculé en 1 appel backend, graphiques canvas faits main (zéro dépendance)
15. **🏷 Tags** : 12 max/note, normalisés, barre de filtres, clic sur un tag pour filtrer, nuage dans le dashboard (verrouillées exclues)
16. **🕸 Graphe de liens** : syntaxe `[[Titre]]` cliquable en lecture, graphe canvas (drag, hover, clic pour ouvrir)
17. **🔥 Streak + objectif** : jours consécutifs, record, mots du jour vs objectif réglable
18. **📦 Backups chiffrés** : auto à l'ouverture (1/h max, 10 conservées), manuelles, restauration avec filet de sécurité
19. **🪞 Coffre leurre** : 2e mot de passe → fausse base plausible et modifiable, message d'erreur indiscernable, gestion depuis le dashboard
20. **🌍 FR / Kabyle** : sélecteur dès l'écran d'accueil (`frontend/lang.js`, choix mémorisé), écran des coffres traduit ; intérieur de l'app à traduire ensuite

## Tests
```bash
cd src-tauri && cargo test   # 6 tests : roundtrip AES-GCM, clé fausse, ciphertext altéré, Argon2, sels uniques, compat anciennes bases
```

## Structure
```
secure-notes-tauri/
  src-tauri/src/main.rs   → 21 commandes Tauri (vault, auto-lock, profils, notes, corbeille, versions, surlignages, recherche)
  src-tauri/src/crypto.rs → Argon2id + AES-GCM (+ tests)
  src-tauri/src/models.rs → VaultFile, Database, Profile, Note, Highlight, NoteVersion (+ test compat)
  frontend/               → splash, vault, main, modales, corbeille, versions, lecture agrandie
```

## Lancer
```bash
cd secure-notes-tauri
npm install
npx tauri dev
# build :
npx tauri build
```
