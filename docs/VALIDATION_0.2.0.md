# Validation Linux — SecureNotes 0.2.0

Date : 25 octobre 2026. Périmètre : Linux x86_64, session X11, GTK 3 et WebKitGTK 4.1.

## Contrôles automatisés

| Contrôle | Résultat |
| --- | --- |
| `npm test` | 16 tests frontend réussis |
| `cargo test --locked` | 28 tests Rust réussis |
| `cargo clippy --locked --all-targets -- -D warnings` | Réussi |
| Vérification de syntaxe JavaScript et `git diff --check` | Réussie |
| Compilation de production et génération Debian/AppImage | Réussies |

## Parcours dans la fenêtre réelle

Le test `tests/desktop-smoke.py` utilise AT-SPI et des événements clavier/souris X11. Le backend Rust et le moteur WebKit sont réels. Chaque exécution utilise un HOME, des chemins XDG, des téléchargements et des documents isolés dans un répertoire temporaire.

Parcours validés :

- Démarrage et bridge Tauri ; changement de langue français/kabyle.
- Création d'un coffre et d'un profil.
- Rédaction et sauvegarde automatique d'un brouillon, récupération puis publication.
- Sauvegarde automatique d'une note existante et relecture après fermeture de l'éditeur.
- Sélection d'une pièce jointe avec le dialogue GTK natif et téléchargement restituant exactement les octets du fichier, y compris des octets binaires.
- Protection individuelle, refus d'un mot de passe erroné et lecture authentifiée.
- Édition protégée, changement de mot de passe, refus de l'ancien mot de passe et reverrouillage individuel.
- Retrait authentifié de la protection individuelle.
- Récupération d'un brouillon protégé avec la demande de mot de passe réellement cliquable.
- Fermeture native de fenêtre pendant une rédaction, redémarrage puis récupération du brouillon.
- Verrouillage/déverrouillage du coffre.
- Réglage effectif à une minute, vérifié dans le dashboard, puis expiration réelle après inactivité et conservation du brouillon.
- Absence des textes de test et des mots de passe dans le fichier de coffre sur disque.

L'AppImage a passé le parcours complet, en mode d'extraction du runtime AppImage. Le paquet Debian a été inspecté, extrait et son exécutable a passé le parcours rapide ; aucune installation système n'a été effectuée.

## Corrections issues de la validation

1. La modale système était masquée derrière la liste des brouillons : priorité d'affichage corrigée et récupération protégée retestée au clavier/souris.
2. WebKitGTK rendait certains sélecteurs avec du texte clair sur un fond natif blanc : apparence et contraste corrigés.
3. Le conteneur flex comprimait la zone de rédaction : hauteur minimale et défilement du formulaire corrigés. Le test graphique vérifie la hauteur de la zone de texte.

## Paquets produits

Dans `src-tauri/target/release/bundle/` :

| Fichier | Taille |
| --- | ---: |
| `deb/SecureNotes_0.2.0_amd64.deb` | 3 392 970 octets |
| `appimage/SecureNotes_0.2.0_amd64.AppImage` | 84 691 448 octets |

Métadonnées Debian vérifiées : paquet `secure-notes`, version `0.2.0`, architecture `amd64`, dépendances `libwebkit2gtk-4.1-0` et `libgtk-3-0`.

### SHA-256 de cette génération

```text
c3013a4d49db7d519fe421461cd210a241742c4c85506b97f4f3420b7d4caa09  deb/SecureNotes_0.2.0_amd64.deb
656ab3438c4ef2ab0a7e6c61aa78775eb6f7fecc20cd152aa7dc0e787a1ce0f9  appimage/SecureNotes_0.2.0_amd64.AppImage
```

Un fichier `SHA256SUMS` est aussi fourni dans le répertoire `bundle/`. Une nouvelle compilation peut produire des empreintes différentes.

## Reproduire

```bash
npm run test:desktop -- --binary src-tauri/target/release/bundle/appimage/SecureNotes_0.2.0_amd64.AppImage
```

Les rapports JSON, journaux et captures d'écran sont conservés dans le répertoire temporaire annoncé par le test. Le README décrit les dépendances et le mode rapide.
