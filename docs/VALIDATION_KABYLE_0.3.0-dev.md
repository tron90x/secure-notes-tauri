# Validation — Interface kabyle

Périmètre : version de développement 0.3.0-dev, Linux x86_64, session X11, fenêtre Tauri/WebKit et backend Rust réels. Validation réalisée le 27 octobre 2026 après recompilation de production.

## Résultat

| Contrôle | Résultat |
| --- | --- |
| `npm run tauri -- build --no-bundle` | Réussi |
| `npm run test:desktop -- --kabyle` | Réussi, 12 entrées de contrôle, `error: null` |
| Inspection des captures | Effectuée : éditeur, lecteur, dialogue, tableau de bord, graphe, dossiers et transferts |

Les 12 entrées du rapport comprennent les deux démarrages réels, le sélecteur de pièce jointe et neuf contrôles spécifiques au parcours kabyle. Les données sont créées dans un profil HOME/XDG temporaire, isolé des coffres personnels.

## Parcours vérifié

1. Sélection de **Taqbaylit**, création d'un coffre et affichage en kabyle du refus d'un mot de passe trop court.
2. Création d'un profil et vérification de la présence des boutons de la barre principale dans les limites de la fenêtre.
3. Édition d'une note avec pièce jointe, état de brouillon chiffré, publication explicite et lecture.
4. Affichage des titres longs des liens entrants/sortants ; téléchargement de la pièce jointe avec comparaison binaire exacte.
5. Dialogue de copie du lien stable et confirmation de mise à la corbeille, annulée depuis le bouton du dialogue.
6. Tableau de bord : série d'écriture, textes d'activité, sécurité, couleurs et fonds traduits ; navigation dans le graphe, capture rapide et recherche dans la palette.
7. Enregistrement d'un dossier de recherche portant un nom long et affichage des compteurs traduits.
8. Note protégée : titre masqué, refus du mauvais mot de passe traduit et lecture après authentification correcte.
9. Export Markdown en ZIP, exclusion des notes protégées du format en clair, inspection de l'archive et comparaison exacte de la pièce jointe.
10. Import du ZIP en copie, aperçu des doublons et confirmation de l'ajout en kabyle.
11. Export chiffré, refus d'un mauvais mot de passe d'archive, aperçu masquant le titre protégé et réimport sans duplication.
12. Sélecteur natif de dossier avec titre **SecureNotes — Kter akaram**, import Markdown et conservation des caractères `ɣ ɛ ḍ ṭ`.
13. Fermeture native, redémarrage et déverrouillage : le choix du kabyle est conservé.

## Affichage et pilote

Les captures ont été inspectées à la largeur de fenêtre de 1280 pixels. Les textes longs des panneaux de liens, des dialogues et des aperçus d'import reviennent à la ligne ; les formulaires hauts restent parcourables par défilement. Le pilote vérifie les dimensions et la présence dans la fenêtre des boutons et contrôles ciblés.

Les ajustements apportés pendant cette validation concernent le pilote : activation accessible du téléchargement et de la palette, attente de la fermeture de la capture, et lecture des textes du tableau de bord/graphe dans l'arbre accessible lorsque WebKit omet l'identifiant de leur conteneur. Aucun défaut applicatif n'a été reproduit sur ce parcours.

Les contrôles des dialogues natifs suivent la langue du système de test. Le périmètre et le vocabulaire de la traduction sont décrits dans [TRADUCTIONS.md](TRADUCTIONS.md).

## Reproduire

Sur une session X11 avec les dépendances Python/GTK indiquées dans le README :

```bash
npm run tauri -- build --no-bundle
npm run test:desktop -- --kabyle
```

Le pilote affiche le répertoire de ses artefacts : rapport JSON, journaux et captures `kab-*.png`. L'exécution réussie de référence est conservée localement dans `/tmp/opencode/securenotes-desktop-5zpzpz_d/`.

Cette validation concerne le binaire de développement recompilé sans bundle ; elle ne constitue pas une validation de nouveaux paquets installables.
