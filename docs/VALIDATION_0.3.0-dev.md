# Validation — Palette et capture rapide, 0.3.0-dev

## Contrôles

| Contrôle | Résultat |
| --- | --- |
| Tests Rust | 33 réussis |
| Tests frontend | 34 réussis |
| Clippy strict, tous les targets | Réussi |
| Compilation de production sans bundle | Réussie |
| Parcours graphique `npm run test:desktop -- --commands` | Réussi |

Le parcours graphique utilise la fenêtre Tauri et le backend Rust réels, sous Linux X11. Les coffres, les chemins HOME/XDG et les fichiers temporaires sont isolés des données personnelles.

## Parcours validés dans Tauri

- Ouverture de la capture par `Ctrl+Shift+N`, création d'un premier profil et publication par `Ctrl+Entrée`.
- Enregistrement automatique du brouillon chiffré et génération d'un titre à partir de la première ligne.
- Ouverture de la palette par `Ctrl+K`, recherche d'une note et ouverture par Entrée.
- Actions `>capture` et `>brouillon`.
- Capture indépendante pendant une rédaction dans l'éditeur complet, avec retour à la rédaction d'origine.
- Transfert d'un brouillon rapide vers l'éditeur complet.
- Choix du profil de destination et navigation par `@profil`.
- Filtrage global par `#tag`, au-delà du profil courant.
- Exclusion des notes protégées de la recherche texte, présentation masquée dans les résultats récents et annulation de leur demande de mot de passe par Échap.
- Demande de déverrouillage lors d'un raccourci de capture depuis le coffre verrouillé, puis ouverture d'une capture vide après authentification.
- Fermeture native de la fenêtre pendant une capture et récupération du dernier brouillon après redémarrage.

## Cas couverts par les tests ciblés

- Métadonnées de recherche bornées, sans contenu, pièce jointe ou payload chiffré dans les réponses de la palette.
- Classement par correspondance du titre puis récence ; exclusion des notes à la corbeille et des tags des notes protégées.
- Échappement des libellés, navigation clavier cyclique, confinement et restauration du focus.
- Rejet des réponses de recherche anciennes et des sauvegardes reçues après expiration du coffre.
- Changements de profil validés pour les nouvelles captures, sans déplacement implicite d'une note existante.
- Conservation de la saisie en cas d'erreur disque et possibilité de réessayer.
- Sauvegarde des deux sessions avant fermeture et mise en file des deux brouillons avant verrouillage.
- Conservation d'un éditeur principal dont le nouveau mot de passe est encore incomplet.
- États d'enregistrement exposés via les régions de statut et leur nom accessible pour WebKitGTK/AT-SPI.

## Reproduire

```bash
npm test
cargo test --locked --manifest-path src-tauri/Cargo.toml
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
npm run tauri -- build --no-bundle
npm run test:desktop -- --commands
```

Le pilote graphique lit la disposition XKB active, restaure le groupe de clavier initial et attend que la saisie soit effectivement visible avant l'action suivante. Les rapports JSON, journaux et captures sont conservés dans le dossier temporaire annoncé à chaque exécution.

Cette validation concerne le binaire de développement 0.3.0-dev. Le rapport 0.2.0 reste la référence des paquets installables 0.2.0.
