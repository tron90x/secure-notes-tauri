# Validation — Imports, exports et échanges chiffrés

Périmètre : version de développement 0.3.0-dev, Linux x86_64, session X11, fenêtre Tauri/WebKit et backend Rust réels. Validation réalisée le 27 octobre 2026.

## Contrôles

| Contrôle | Résultat |
| --- | --- |
| Tests Rust, dépendances verrouillées | 55 réussis |
| Tests frontend | 62 réussis |
| Clippy strict, tous les targets | Réussi |
| Syntaxe JavaScript des modules frontend | Réussie |
| `git diff --check` | Réussi |
| Compilation de production sans bundle | Réussie |
| `npm run test:desktop -- --transfer` | 8 contrôles réussis |

## Parcours graphique

Le pilote utilise un profil HOME/XDG temporaire et des coffres de test isolés.

1. Démarrage de la vraie fenêtre WebKit et vérification du pont Tauri.
2. Sélection d'un dossier dans le dialogue natif ; import de notes Markdown/texte, de leurs références internes et d'une ressource binaire.
3. Ajout d'une pièce jointe depuis le sélecteur de fichiers GTK.
4. Export d'une sélection en ZIP Markdown puis texte ; inspection des archives téléchargées, des tags et des octets des pièces jointes.
5. Téléchargement d'une pièce jointe depuis le lecteur et comparaison binaire exacte.
6. Import en copies vers un profil de destination : conservation des originaux, nouvelles identités, références internes dirigées vers les copies et restauration des pièces jointes.
7. Échange d'une archive chiffrée entre deux coffres distincts : refus du mauvais mot de passe d'archive, import avec le bon, puis vérification du mot de passe individuel indépendant de la note protégée.
8. Réimport de la même archive : aucune duplication ni écrasement des identités existantes.

Le rapport de cette exécution contient huit contrôles et `error: null`. Ses artefacts locaux sont conservés dans `/tmp/opencode/securenotes-desktop-enw5lspb/` : `report.json`, journaux et captures. Ce répertoire temporaire n'est pas nécessaire pour reproduire le parcours.

## Cas complémentaires automatisés

- Aller-retour des métadonnées, liens et pièces jointes ; import des modifications externes avec conservation du contenu original comme version.
- Réattribution des identifiants et ajustement des positions de surlignage lors d'un import en copies.
- Conservation des exemples de code Markdown ; import des ressources locales sans téléchargement des URL externes.
- Regroupement de plusieurs ressources en pièce jointe ZIP.
- Rejet des chemins dangereux, liens symboliques ZIP, paquets incomplets, identifiants sources dupliqués et paramètres d'enveloppe invalides.
- Lecture native limitée à la racine choisie, liens symboliques ignorés et vérification de l'identité de session avant préparation.
- Conservation du plan d'import si l'écriture échoue, pour permettre une nouvelle tentative.
- Exclusion explicite des notes protégées des exports en clair ; contrôle des mots de passe pour l'échange chiffré.
- Effacement des secrets et des aperçus au verrouillage ; réponses tardives ignorées, annulation du sélecteur et fermeture native pendant un export.

Les [formats d'échange et limites](FORMATS_ECHANGE.md) précisent les garanties, les formats acceptés et les limites de taille. Les ZIP Markdown/texte sont en clair ; `.snotetransfer` est chiffré.

## Reproduire

```bash
npm test
cargo test --locked --manifest-path src-tauri/Cargo.toml
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
npm run tauri -- build --no-bundle
npm run test:desktop -- --transfer
```

Le parcours graphique demande une session X11 et les dépendances Python/GTK indiquées dans le README. Cette validation concerne le binaire de développement recompilé ; les paquets installables 0.2.0 disposent de leur rapport séparé.
