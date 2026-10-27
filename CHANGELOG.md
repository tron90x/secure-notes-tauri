# Journal des changements

## 0.3.0-dev — En développement

### Ajouts
- Icône originale carnet/cadenas, déclinée en PNG, ICO et ICNS et intégrée aux paquets, au démarrage et à l'en-tête de l'application.
- Import de fichiers/dossiers Markdown et texte, avec métadonnées simples, liens locaux et ressources sélectionnées.
- Export d'une sélection en ZIP Markdown/texte avec pièces jointes et manifeste d'aller-retour.
- Archives chiffrées `.snotetransfer` par Argon2id/AES-256-GCM, conservant la protection individuelle des notes.
- Préparation d'import validée en mémoire, aperçu des doublons, choix du profil et import atomique sans écrasement.
- Copies de notes libres avec nouveaux identifiants, liens internes et surlignages ajustés ; contenu externe modifié conservé avec une version d'origine.
- Dossiers intelligents chiffrés : création depuis une recherche, compteurs dynamiques, modification des critères et suppression sans supprimer les notes.
- Recherche combinant mots, expressions, exclusions, tags, profils, dates, modifications récentes et présence de pièces jointes.
- Panneau de liens entrants/sortants, aperçus, création de références absentes et choix explicite en cas d'homonymie.
- Références stables `[[identifiant|libellé]]`, choix de note depuis l'éditeur et copie d'un lien stable.
- Intégration des dossiers à la palette avec le préfixe `/` ; résolution commune au graphe et au lecteur.
- Palette `Ctrl+K` / `Cmd+K` : notes récentes, recherche dans les notes libres, profils, tags et commandes applicatives.
- Navigation clavier, gestion du focus et filtres `>commande`, `@profil`, `#tag`.
- Capture rapide `Ctrl+Shift+N` / `Cmd+Shift+N`, publication `Ctrl+Entrée`, choix et création du profil.
- Brouillons chiffrés et session de capture indépendante de la rédaction principale, avec transfert vers l'éditeur complet.
- Reprise de la capture après déverrouillage du coffre et effacement des champs/résultats au verrouillage.

### Corrections
- Révision et harmonisation du kabyle ; traduction des confirmations, états vides, infobulles, libellés accessibles, statistiques et messages applicatifs encore affichés en français.
- Sélecteur de dossier natif Tauri pour l'import, indépendant du dialogue de fichiers WebKit ; sélection liée à la session du coffre.
- Correspondance des positions de surlignage avec le texte source lorsque les liens affichent un libellé raccourci.
- Les UUID de cibles supprimées ne retombent pas sur un titre homonyme ; les notes protégées restent exclues des tests de contenu et de l'index des liens.
- Attente des deux brouillons avant verrouillage ou fermeture de fenêtre.
- Annulation des dialogues de mot de passe avec Échap, sans fermer leur écran parent.
- Rejet des réponses de recherche périmées et des retours de sauvegarde après expiration de session.
- Conservation du focus de la palette lorsqu'une capture se termine d'ouvrir en arrière-plan.

### Maintenance
- Intégration du plugin officiel de dialogue et mise à jour cohérente des dépendances Rust Tauri vers la famille 2.12.

## 0.2.0 — 2026-10-25

### Ajouts
- Édition authentifiée des notes protégées, changement et retrait du mot de passe, reverrouillage individuel.
- Sauvegarde automatique après une pause de saisie et récupération des brouillons chiffrés.
- Protection des modifications concurrentes et conservation des brouillons en cas d'échec d'enregistrement.
- Téléchargement, remplacement et suppression des pièces jointes.
- Tests automatisés frontend/backend et parcours Linux dans la fenêtre Tauri réelle, avec profil de test isolé.

### Corrections
- Échappement du contenu utilisateur et politique CSP.
- Écritures atomiques, restauration validée avant remplacement et sauvegardes isolées par coffre.
- Chiffrement individuel du titre, des pièces jointes, des tags et de l'historique.
- Restauration des versions, verrouillage automatique, recherches par dates et statistiques d'activité.
- Priorité d'affichage des demandes de mot de passe et confirmations au-dessus des autres modales.
- Lisibilité des listes déroulantes WebKitGTK et hauteur de la zone de rédaction.

### Compatibilité
- Lecture des anciens coffres et migration des anciens formats de notes protégées après authentification.
- Les nouvelles notes sont conservées comme brouillons jusqu'au clic sur « Enregistrer ».
- Les sauvegardes historiques conservent le format et la protection utilisés lors de leur création.
