# Icône originale et paquets Linux — 0.3.0-dev

Validation du 27 octobre 2026, Linux x86_64, session X11.

## Identité visuelle

L'icône originale associe un carnet blanc à reliure cyan et un cadenas sur fond bleu nuit/violet. Sa source vectorielle est `frontend/icon.svg` ; elle est aussi utilisée dans l'écran de démarrage, l'en-tête et comme favicon.

Les fichiers de `src-tauri/icons/` sont générés avec l'outil officiel Tauri :

```bash
npm run tauri -- icon frontend/icon.svg --output src-tauri/icons
```

La configuration déclare les PNG 32, 128, 256 et 512 pixels, le fichier Windows multi-résolution `icon.ico` et le fichier macOS `icon.icns`. Les formats Windows et macOS sont prêts pour leurs compilations natives ; les paquets produits ici ciblent Linux.

## Vérifications effectuées

- `npm test` : 66 tests réussis ; `git diff --check` : réussi.
- Exécutable de production recompilé ; paquets Debian, RPM et AppImage générés.
- Inspection des trois archives : le lanceur déclare `Icon=secure-notes-tauri` et contient les quatre résolutions PNG déclarées, identiques octet pour octet aux images générées.
- Icône racine `.DirIcon` de l'AppImage extraite et comparée au PNG original 512 pixels.
- Démarrage réel du binaire de production et de l'AppImage dans deux profils HOME/XDG isolés ; création d'un coffre et inspection de l'en-tête avec le nouveau logo.
- Lecture de la propriété X11 `_NET_WM_ICON` : les pixels de l'icône native 32 × 32 correspondent exactement au PNG généré pour les deux exécutables testés.

Les rapports graphiques ont trois contrôles réussis chacun, avec `error: null` :

- Binaire : `/tmp/opencode/securenotes-desktop-g9_5m1iv/`.
- AppImage : `/tmp/opencode/securenotes-desktop-k635669s/`.

Les fichiers extraits pour l'inspection de l'AppImage sont conservés dans `/tmp/opencode/securenotes-icon-package-j0yfmzoy/`. Aucune installation système n'est nécessaire pour ces contrôles.

## Paquets produits

Chemins relatifs à `src-tauri/target/release/bundle/` :

| Fichier | Taille en octets |
| --- | ---: |
| `deb/SecureNotes_0.3.0-dev_amd64.deb` | 4 352 606 |
| `rpm/SecureNotes-0.3.0-dev-1.x86_64.rpm` | 4 352 751 |
| `appimage/SecureNotes_0.3.0-dev_amd64.AppImage` | 85 547 512 |

Empreintes SHA-256 de cette génération :

```text
7e4a3a2c8ff5f1741189a1584e4f959e3e8b97484611c52774616984ad16d793  deb/SecureNotes_0.3.0-dev_amd64.deb
f176159b1b47db0f26475a6ae057283f550152e686d6d5de3df0fc7a62bf5082  rpm/SecureNotes-0.3.0-dev-1.x86_64.rpm
18ad9b1bdc988497ffd4692eb7f07b930b06267bb576d303f70418d13efe8883  appimage/SecureNotes_0.3.0-dev_amd64.AppImage
```

Sous Linux, les icônes du menu d'applications proviennent du lanceur `.desktop` et du thème d'icônes installé par le paquet. L'affichage d'une miniature sur le fichier AppImage lui-même dépend de l'intégration du gestionnaire de fichiers ; le binaire ELF brut n'est pas un lanceur de bureau.

## Reproduire

```bash
npm run tauri -- build --bundles deb,rpm,appimage
npm run test:desktop -- --icons
npm run test:desktop -- --icons --binary src-tauri/target/release/bundle/appimage/SecureNotes_0.3.0-dev_amd64.AppImage
```

La génération AppImage ayant dépassé le délai initial du pilote de commandes, son empaquetage a été terminé avec `npm run tauri -- bundle --bundles appimage --verbose`, à partir du binaire déjà compilé.
