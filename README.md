# SecureNotes — Tauri + Rust (coffre chiffré)

App de prise de notes sophistiquée, 100% locale, chiffrée.

Version de développement **0.3.0-dev** — détails dans [CHANGELOG.md](CHANGELOG.md). Le [rapport de validation Linux 0.2.0](docs/VALIDATION_0.2.0.md) décrit la version installable précédente.

**[Télécharger la préversion Linux 0.3.0-dev](https://github.com/tron90x/secure-notes-tauri/releases/tag/v0.3.0-dev)** : Debian/Ubuntu/Linux Mint (`.deb`), AppImage et RPM, avec la nouvelle icône originale. Les empreintes SHA-256 sont fournies avec les téléchargements.

La palette et la capture ont leur [rapport de validation dédié](docs/VALIDATION_0.3.0-dev.md).
L'organisation et les backlinks ont également leur [rapport de validation](docs/VALIDATION_ORGANISATION_0.3.0-dev.md).
Les imports, exports ZIP et échanges chiffrés ont leur [rapport de validation de portabilité](docs/VALIDATION_TRANSFERTS_0.3.0-dev.md).
L'interface kabyle a son [rapport de validation dans la fenêtre Tauri réelle](docs/VALIDATION_KABYLE_0.3.0-dev.md).
L'icône originale et les paquets Linux recompilés ont leur [rapport de vérification](docs/VALIDATION_ICONES_0.3.0-dev.md).

## Portabilité

**Import / Export**, dans la barre supérieure ou la palette, permet d'importer des fichiers/dossiers Markdown et texte, puis d'exporter une sélection en ZIP Markdown, ZIP texte ou archive chiffrée `.snotetransfer`.

L'aperçu précède l'import, conserve les notes existantes par défaut et permet des copies pour les notes libres, avec réécriture des liens internes. Les dates, tags, versions, surlignages et pièces jointes sont conservés dans les formats d'aller-retour. Une archive chiffrée transfère aussi les notes protégées sans changer leur mot de passe individuel.

Les [formats d'échange et limites](docs/FORMATS_ECHANGE.md) décrivent les métadonnées, les ressources locales, les modifications externes et le chiffrement.

## Dossiers intelligents et liens

### Recherche et vues enregistrées

La recherche principale combine les mots (ET), les expressions `"entre guillemets"` et les exclusions `-mot` ou `-"expression"`. Les filtres du formulaire se combinent avec les opérateurs :

| Exemple | Effet |
| --- | --- |
| `tag:travail -tag:archive` | Tags exacts inclus/exclus |
| `profil:"Mon profil"` | Nom exact du profil, insensible à la casse ; un identifiant est aussi accepté |
| `apres:2026-10-01 avant:2026-11-01` | Créations à partir du 1er octobre inclus, avant le 1er novembre exclu, en UTC |
| `modifiee:7j` | Création ou dernière édition enregistrée dans les sept derniers jours glissants |
| `avec:piece-jointe` | Notes libres avec une pièce jointe |
| `sans:tag` | Notes libres sans tag |
| `statut:verrouillee` / `statut:libre` | Filtre sur la protection individuelle |

Le bouton **?** rappelle la syntaxe. Les critères de contenu, y compris les exclusions, ne permettent pas de tester le contenu ou les pièces jointes d'une note protégée.

**Enregistrer la recherche** crée un dossier intelligent dans la barre latérale. Chaque dossier conserve ses critères dans le coffre chiffré, sans dupliquer les notes. Ses résultats et son compteur sont recalculés à l'ouverture ou après modification des notes ; les périodes relatives restent relatives. Le bouton ✎ permet de modifier nom et critères. Supprimer un dossier conserve les notes. Jusqu'à 50 dossiers sont pris en charge.

La palette accepte **`/nom du dossier`** et propose les actions d'accès aux dossiers et d'enregistrement de recherche.

### Références et backlinks

- Le panneau **Liens et contexte** montre les notes libres qui citent la note courante et ses références sortantes. Les aperçus sont disponibles dans les lignes et au survol.
- Une référence de titre sans cible peut créer une note dans le même profil. Les références ambiguës proposent les différentes cibles au lieu d'en choisir une arbitrairement. Une cible référencée par identifiant et placée dans la corbeille peut être restaurée.
- **Choisir une note**, dans l'éditeur, insère un lien `[[identifiant|libellé]]`. L'identifiant est stable après renommage ; le libellé reste lisible dans la lecture et les aperçus. **Lien stable**, en lecture, affiche une référence sélectionnable à copier avec Ctrl+C/Cmd+C.
- Les anciens `[[Titre]]` restent reconnus et dépendent du titre courant. Les `[[identifiant]]` sont également acceptés. Un identifiant UUID supprimé n'est jamais réinterprété comme le titre d'une autre note.
- Le graphe et les backlinks partagent la même résolution. L'index exclut les notes protégées ; une cible protégée référencée par identifiant reste masquée et demande son mot de passe à l'ouverture.
- Les surlignages utilisent les positions du texte source, même lorsque le libellé affiché est plus court que la référence stable. Une sélection à l'intérieur d'un libellé couvre la référence entière.
- Le panneau affiche au maximum 50 liens entrants et 50 références sortantes, avec les totaux complets. Les listes de cibles ambiguës affichent jusqu'à huit choix.

## Palette et capture rapide

- **Ctrl+K / Cmd+K** ouvre la palette, également accessible depuis le bouton « Commandes ». Utilisez ↑/↓ puis Entrée pour choisir, Échap pour fermer et Tab pour parcourir ses contrôles.
- Recherchez des notes par titre ou contenu, dans tous les profils. Sans requête, la palette propose les actions courantes et les notes récentes. La recherche est insensible à la casse et combine les mots saisis.
- Les préfixes **`>`** (commandes), **`@`** (profils) et **`#`** (tags) limitent les résultats. Les commandes donnent accès à la création, aux brouillons, au dashboard, au graphe, aux sauvegardes, à la corbeille, à la recherche avancée et au verrouillage.
- Les résultats de recherche ne contiennent que des métadonnées : une note et sa pièce jointe sont chargées à son ouverture. Les notes protégées apparaissent masquées dans les notes récentes et restent exclues de la recherche texte.
- **Ctrl+Shift+N / Cmd+Shift+N** ouvre une capture compacte dans l'application. Choisissez le profil, saisissez une idée et utilisez **Ctrl+Entrée / Cmd+Entrée** pour publier la note. Le titre facultatif est déduit de la première ligne si nécessaire.
- La capture utilise une session indépendante de l'éditeur principal et sauvegarde automatiquement un **brouillon chiffré dans le coffre**. Échap ou « Fermer » conserve le brouillon. Le bouton « Éditeur complet » permet de le développer, notamment pour lui ajouter une protection individuelle ou une pièce jointe.
- Un profil peut être créé directement depuis la capture si le coffre n'en contient aucun. Si le coffre est verrouillé, le raccourci demande son ouverture puis reprend la capture ; aucun texte de note n'est accepté avant le déverrouillage.
- Les raccourcis fonctionnent lorsque la fenêtre de l'application a le focus. La palette et la capture effacent leurs champs et résultats lors du verrouillage.

## Sécurité
- Base `*.snotevault` dans `~/Documents/SecureNotes/` : JSON chiffré **AES-256-GCM**
- Clé dérivée par **Argon2id (m=64MB, t=3, p=1)** + sel 16 octets ; mot de passe de base **min 8 caractères** avec jauge de force
- Chiffrement individuel optionnel par note (2e mot de passe, min 4) : **titre, contenu, pièce jointe, tags, versions et surlignages chiffrés ensemble**, révélés après authentification ; note exclue de la recherche par mot-clé
- Zéro-connaissance : mot de passe jamais stocké, clé en RAM seulement, **zeroize à chaque verrouillage**
- **Verrouillage auto après inactivité** (1 min … 1 h, défaut 5 min), vérifié côté backend chaque seconde et à chaque commande ; activité de saisie transmise par l'UI
- Écritures atomiques via un fichier temporaire synchronisé ; état en mémoire validé uniquement après réussite de l'enregistrement
- Texte utilisateur échappé au rendu et politique CSP limitant l'exécution des scripts aux ressources de l'application

### Compatibilité des notes protégées

Les anciennes notes, dont seul le contenu était chiffré individuellement, restent lisibles. Leur titre et leur pièce jointe sont masqués dans les réponses à l'interface ; leur migration vers le chiffrement individuel complet est enregistrée lors du prochain déverrouillage de la note. Les sauvegardes antérieures conservent leur format d'origine.

### Édition sécurisée et sauvegarde automatique

- **Modifier une note protégée** demande son mot de passe et ouvre une session d'édition temporaire. Le backend conserve une clé individuelle en mémoire, effacée à la fermeture de l'éditeur ou au verrouillage du coffre.
- Le menu de protection permet de **définir, changer ou retirer le mot de passe individuel**. Les changements sont appliqués par la sauvegarde automatique, comme le reste du formulaire.
- Après **1,2 seconde sans nouvelle saisie**, les notes existantes sont enregistrées. Les nouvelles notes restent des **brouillons chiffrés** jusqu'au clic sur « Enregistrer ».
- L'indicateur de l'éditeur distingue les modifications en cours, l'enregistrement réussi, le brouillon enregistré et les erreurs. Une erreur conserve la saisie pour réessayer.
- Le bouton **Brouillons** permet de reprendre, récupérer comme nouvelle note ou supprimer explicitement un brouillon. Un brouillon protégé demande le mot de passe utilisé au moment de son enregistrement, y compris après un changement de protection.
- Les brouillons sont inclus dans le coffre chiffré ; ceux des notes protégées sont aussi chiffrés avec la clé individuelle. Chaque enregistrement écrit d'abord le brouillon, puis la note, pour conserver une copie récupérable si la validation de la note échoue.
- **Fermer** conserve les modifications enregistrées et sauvegarde les brouillons des nouvelles notes. La fermeture de la fenêtre attend l'enregistrement ; si celui-ci échoue, l'éditeur reste ouvert. Le verrouillage masque immédiatement les données et tente de conserver le dernier brouillon valide.
- Les sauvegardes automatiques d'une même session d'édition partagent **un snapshot initial**, afin de conserver un historique utile de 20 versions. Une révision vérifiée côté backend empêche l'écrasement d'une modification concurrente ; le brouillon peut alors être récupéré comme nouvelle note.

## Fonctionnalités
1. **Splash animé** : logo pop + barre dégradé + particules canvas + orbes blur
2. **Vault** : créer base (nom + mdp fort) / lister / déchiffrer
3. **Profils** : nom + `created_at`, ajout/suppression (notes → corbeille)
4. **Notes** : titre, contenu, `created_at`, `modified_history[]`, ajout/modif/**corbeille**
5. **Corbeille** : soft-delete, restauration, destruction définitive avec confirmation, vidage global ; les notes d'un profil supprimé sont restaurées dans « Notes restaurées »
6. **Versions** : snapshot initial par session d'édition (20 max), restauration en 1 clic pour les notes libres ; historique des notes protégées chiffré individuellement
7. **Recherche avancée** : mots-clés + filtre profil + dates UTC, jour de fin inclus (les notes verrouillées sont ignorées par la recherche texte)
8. **Surlignage** : sélection + 5 couleurs, persisté, suppression au clic
9. **Note verrouillée** : création, édition authentifiée, changement/retrait du mot de passe et reverrouillage individuel
10. **Pièce jointe** : fichier → base64 stocké dans DB chiffrée, téléchargement en lecture, remplacement/suppression en édition, y compris pour les notes protégées
11. **Wallpaper par note** : aurora/sunset/ocean/neon/sakura/carbon + toggle **glass** (appliqué à la vignette ET à la popup)
12. **Lecture agrandie** : bouton ＋, grand rectangle avec fond + marquages
13. **Modales système** : plus aucun `alert/confirm/prompt` natif
14. **📊 Dashboard** (bouton topbar, Échap pour fermer) : 8 KPIs, notes par profil,
    donut verrouillées/libres, courbe d'activité 30j, surlignages par couleur,
    fonds utilisés, heatmap 12 semaines, top 5 longues notes, panneau sécurité —
    calculé en 1 appel backend, graphiques canvas faits main (zéro dépendance)
15. **🏷 Tags** : 12 max/note, normalisés, barre de filtres, clic sur un tag pour filtrer, nuage dans le dashboard (verrouillées exclues)
16. **🕸 Graphe de liens** : syntaxe `[[Titre]]` cliquable en lecture, graphe canvas (drag, hover, clic pour ouvrir)
17. **🔥 Streak + objectif** : jours consécutifs, record, mots du jour vs objectif réglable ; les mots comptent la création et les augmentations positives du nombre de mots des notes libres, y compris l'édition d'anciennes notes, sans double comptage des versions dans l'activité
18. **📦 Backups chiffrés** : auto à l'ouverture (1/h max, 10 conservées par coffre), manuelles, validation avant restauration et copie de sécurité obligatoire
19. **🪞 Coffre leurre** : 2e mot de passe → fausse base plausible et modifiable, message d'erreur indiscernable, gestion depuis le dashboard
20. **🌍 FR / Kabyle** : sélecteur dès l'écran d'accueil (`frontend/lang.js`, choix mémorisé), écrans des coffres et des notes, confirmations, corbeille, historique, tableau de bord, palette, dossiers et transferts traduits ; messages applicatifs du backend traduits à l'affichage

## Tests

Le [guide des traductions](docs/TRADUCTIONS.md) décrit le vocabulaire kabyle harmonisé et la gestion des textes techniques.

```bash
npm test
cargo test --locked --manifest-path src-tauri/Cargo.toml
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
```

Les tests Rust couvrent la cryptographie, la compatibilité et la migration des notes, les sauvegardes/restaurations dans des répertoires temporaires, l'édition protégée, les brouillons, les conflits de révision, la persistance, les versions, l'expiration, les pièces jointes, la recherche et les statistiques. Les tests frontend utilisent le runner natif Node.js et un DOM simulé pour vérifier le rendu, la sauvegarde automatique, les écritures concurrentes, les erreurs, la récupération, la fermeture de fenêtre, les formulaires, les dates, l'activité et les téléchargements.

### Validation graphique Linux

Sur une session **X11**, avec `python3-pyatspi`, `python3-xlib`, `python3-gi` et les introspections GTK 3 installés :

```bash
npm run tauri -- build --no-bundle
npm run test:desktop
# Parcours rapide, sans attendre le verrouillage automatique d'une minute :
npm run test:desktop -- --quick
# Parcours de la palette et de la capture rapide :
npm run test:desktop -- --commands
# Dossiers intelligents, backlinks, renommage et surlignages :
npm run test:desktop -- --organization
# Import natif, exports ZIP et échange chiffré entre coffres :
npm run test:desktop -- --transfer
# Interface kabyle, dialogues, affichage et imports/exports :
npm run test:desktop -- --kabyle
# Icône native de la fenêtre et démarrage avec le logo original :
npm run test:desktop -- --icons
# Tester un exécutable extrait d'un paquet :
npm run test:desktop -- --binary /chemin/vers/secure-notes-tauri
```

Ce test ouvre la vraie fenêtre Tauri et pilote le clavier et la souris. Il utilise un profil HOME/XDG isolé dans `/tmp/opencode`, crée uniquement des coffres de test et conserve un rapport JSON, les journaux et des captures dans le dossier temporaire indiqué. Il couvre aussi le sélecteur natif de fichiers, le téléchargement binaire des pièces jointes et la fermeture native de fenêtre.

Un AppImage peut être passé à `--binary` : le test utilise alors le mode d'extraction du runtime AppImage et suit le processus de la fenêtre enfant. Le verrouillage d'une minute est vérifié après lecture du réglage effectif dans le dashboard.

## Structure
```
secure-notes-tauri/
  src-tauri/src/main.rs   → commandes Tauri, sessions, persistance, sauvegardes, recherche, statistiques et graphe
  src-tauri/src/crypto.rs → Argon2id + AES-GCM (+ tests)
  src-tauri/src/editor.rs → sessions d'édition, brouillons chiffrés, protection et sauvegarde automatique (+ tests)
  src-tauri/src/commands.rs → recherche légère pour la palette (+ tests)
  src-tauri/src/search.rs  → langage de recherche, filtres compilés et dossiers intelligents (+ tests)
  src-tauri/src/links.rs   → références stables, résolution et backlinks (+ tests)
  src-tauri/src/transfer.rs → Markdown/texte, ZIP, archives chiffrées et import atomique (+ tests)
  src-tauri/src/models.rs → VaultFile, Database, Profile, Note, Highlight, NoteVersion (+ test compat)
  frontend/               → splash, vault, main, modales, corbeille, versions, lecture agrandie
  tests/frontend.test.cjs  → tests de non-régression frontend
  tests/commands.test.cjs  → clavier, capture, navigation et isolation des sessions
  frontend/commands.js    → contrôleur de la palette et de la capture rapide
  frontend/organization.js → dossiers intelligents et panneau de contexte
  frontend/transfer.js    → sélection, aperçu d'import et téléchargement des échanges
  tests/transfer.test.cjs → formats, mots de passe, doublons et réponses tardives
  tests/organization.test.cjs → filtres enregistrés, contexte, références et surlignages
  tests/desktop-smoke.py   → parcours de validation dans la fenêtre Linux réelle
```

## Lancer
```bash
cd secure-notes-tauri
npm install
npx tauri dev
# build :
npx tauri build
# Paquets Linux Debian, RPM et AppImage :
npm run tauri -- build --bundles deb,rpm,appimage
# Regénérer seulement les paquets à partir du binaire déjà compilé :
npm run tauri -- bundle --bundles deb,rpm,appimage
```

Les paquets sont écrits dans `src-tauri/target/release/bundle/deb/`, `src-tauri/target/release/bundle/rpm/` et `src-tauri/target/release/bundle/appimage/`.

L'icône vectorielle originale est dans `frontend/icon.svg`. Pour régénérer les icônes des exécutables et des paquets : `npm run tauri -- icon frontend/icon.svg --output src-tauri/icons`.
