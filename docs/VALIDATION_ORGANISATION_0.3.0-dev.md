# Validation — Organisation, recherche et backlinks

Périmètre : version de développement 0.3.0-dev, Linux x86_64, session X11, fenêtre Tauri et backend Rust réels.

## Contrôles

| Contrôle | Résultat |
| --- | --- |
| Tests Rust | 43 réussis |
| Tests frontend | 48 réussis |
| Clippy strict, tous les targets | Réussi |
| Syntaxe JavaScript et `git diff --check` | Réussies |
| Compilation de production sans bundle | Réussie |
| `npm run test:desktop -- --organization` | Parcours réussi |

## Parcours graphique

Les données sont créées dans un profil HOME/XDG temporaire, distinct des coffres personnels.

1. Création d'une cible et insertion de son lien stable dans une seconde note depuis le choix de note de l'éditeur.
2. Recherche combinant une expression entre guillemets et `modifiee:7j`, puis enregistrement d'un dossier intelligent.
3. Vérification que ni le nom du dossier ni le mot recherché n'apparaissent en clair dans le fichier chiffré.
4. Sélection à la souris d'un mot situé après le lien à libellé, puis surlignage. Une capture confirme que le bon mot est marqué malgré la différence de longueur entre l'identifiant source et le libellé affiché.
5. Navigation dans les liens sortants et contrôle des liens entrants sur la cible.
6. Renommage de la cible : la référence stable et le panneau retrouvent la même note avec son nouveau titre.
7. Création d'une note depuis une référence de titre absente, sans réécrire le contenu de la source.
8. Édition du contenu de la source : la note sort du dossier dont elle ne remplit plus les critères.
9. Modification du nom et de la recherche du dossier, puis navigation depuis la palette avec `/nom`.
10. Fermeture native, redémarrage et réouverture du dossier avec les critères enregistrés.
11. Contrôle du graphe en couche « Notes » : ses deux liens correspondent aux deux références du lecteur.
12. Suppression du dossier : les trois notes restent présentes.

## Cas complémentaires automatisés

- Expressions exactes, exclusions, opérateurs de profil/tag, dates, périodes relatives et pièces jointes.
- Rejet des dates et durées invalides, des guillemets non fermés et des noms de dossiers en double.
- Recalcul d'une période relative huit jours plus tard et sérialisation des dossiers.
- Exclusion des notes protégées des critères de contenu, y compris négatifs ; résultats de métadonnées expurgés.
- Liens entrants privés exclus, titres et aperçus de cibles protégées masqués.
- Homonymes signalés explicitement ; UUID supprimés jamais réinterprétés comme titres.
- Création de cible idempotente et limitée aux références réellement présentes dans une source libre.
- Résultats de contexte bornés, HTML rendu comme texte, réponses périmées ignorées après navigation ou verrouillage.
- Filtrage `/dossier` dans la palette, sélection de note sans fermer l'éditeur et copie d'une référence dans un champ sélectionné en lecture seule.
- Correspondance des positions de surlignage avec la source, y compris Unicode et balises imbriquées.
- Le filtrage d'une entité du graphe quitte correctement le contexte du dossier enregistré.

## Reproduire

```bash
npm test
cargo test --locked --manifest-path src-tauri/Cargo.toml
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
npm run tauri -- build --no-bundle
npm run test:desktop -- --organization
```

Le pilote annonce le répertoire contenant son rapport JSON, les journaux et les captures. Les paquets installables 0.2.0 conservent leur rapport de validation propre ; cette étape concerne le binaire de développement recompilé.
