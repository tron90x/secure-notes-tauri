# Traductions français / kabyle

Les libellés de l'interface sont regroupés dans `frontend/lang.js`. Le choix de langue est mémorisé ; il règle également la langue du document, son titre, les infobulles et les libellés d'accessibilité.

## Vocabulaire harmonisé

| Français | Kabyle |
| --- | --- |
| Coffre | Asenduq |
| Note | Tazmilt |
| Profil | Amaɣnu |
| Mot de passe | Awal uffir |
| Chiffrement | Awgelhen |
| Protection | Ammesten |
| Verrouiller | Sekkeṛ |
| Enregistrer | Sekles |
| Brouillon | Arewway |
| Sauvegarde | Aḥraz |
| Lien | Aseɣwen |
| Tag | Tabzimt |
| Pièce jointe | Afaylu yecudden |
| Dossier intelligent | Akaram n unadi |

« Akaram n unadi » décrit la fonction de recherche enregistrée du dossier. Les indications de chiffrement et de verrouillage sont distinctes de la simple conservation des données.

Les noms de profils, titres et contenus de notes, noms de fichiers et tags saisis restent tels quels. Les opérateurs de recherche (`tag:`, `modifiee:7j`, etc.), les préfixes de contenu (`#lieu/`, `#idée/`), les extensions et les noms d'algorithmes conservent leur syntaxe technique. Les touches `Entrée` et `Échap` sont citées comme repères du clavier.

Les messages applicatifs connus du backend sont traduits par correspondance exacte à leur réception. Les chemins inclus dans ces messages sont conservés. Les diagnostics des bibliothèques et du système restent intacts ; les contrôles des sélecteurs natifs suivent la langue du système.

## Vérification

`npm test` vérifie notamment la parité des clés français/kabyle, les paramètres `{count}`, `{name}`, etc., les clés des attributs HTML et du JavaScript, le changement de langue des attributs accessibles, le rendu dynamique et la traduction des erreurs sans perturber la détection des sessions expirées.

`npm run test:desktop -- --kabyle` vérifie les parcours dans la vraie fenêtre Tauri, avec des coffres de test isolés. Le [rapport de validation graphique](VALIDATION_KABYLE_0.3.0-dev.md) détaille les écrans et échanges vérifiés.
