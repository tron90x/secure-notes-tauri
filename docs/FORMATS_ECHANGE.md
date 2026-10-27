# Import et export des notes

Le bouton **Import / Export** et les commandes correspondantes de la palette échangent des notes entre outils ou installations. Les échanges sont préparés localement, dans un coffre ouvert.

## Exporter une sélection

La sélection initiale reprend la note en lecture ou les notes de la vue courante. Le profil, le filtre de titre et les boutons de sélection permettent de la modifier.

Trois formats sont proposés :

| Format | Contenu |
| --- | --- |
| Archive `.snotetransfer` | Notes, profils, dates, tags, versions, surlignages, apparence et pièces jointes, sous chiffrement authentifié |
| ZIP Markdown | Fichiers `.md` lisibles, métadonnées de présentation, pièces jointes et manifeste d'aller-retour |
| ZIP texte | Fichiers `.txt`, pièces jointes et même manifeste d'aller-retour |

Les notes protégées individuellement utilisent l'archive chiffrée et conservent leur propre mot de passe. Une ancienne note dont seul le contenu était protégé doit être déverrouillée une fois dans l'application pour migrer son chiffrement avant le transfert.

Les ZIP Markdown et texte sont explicitement non chiffrés. Ils contiennent uniquement les notes libres sélectionnées. Le passage à un format non chiffré retire les notes protégées de la sélection et l'indique dans l'interface.

## Importer

1. Choisir des fichiers Markdown/texte, un dossier, un ZIP compatible ou un fichier `.snotetransfer`.
2. Saisir le mot de passe du fichier `.snotetransfer`, le cas échéant.
3. Pour des fichiers, cliquer sur **Préparer l'import**. Pour un dossier, le dialogue natif prépare directement l'aperçu. Les données, chemins, références de fichiers et métadonnées sont validés avant toute écriture dans le coffre.
4. Lire les compteurs et choisir les profils d'origine ou un profil de destination.
5. Cliquer sur **Importer les notes préparées**.

Les préparations restent en mémoire jusqu'à validation, fermeture ou verrouillage. L'aperçu montre les 100 premières notes au maximum, avec le nombre total à importer. Une erreur d'enregistrement conserve la préparation afin de réessayer.

Le choix d'un dossier utilise le dialogue natif Tauri. Sa lecture reste limitée au dossier choisi, ignore les liens symboliques et fichiers spéciaux, et ne retient pas le verrou du coffre. Si le coffre est verrouillé ou changé pendant la sélection/lecture, l'ancienne sélection ne peut pas être préparée dans la nouvelle session.

### Identifiants et doublons

Par défaut, les identifiants sont conservés. Une note déjà présente dans le coffre, y compris dans sa corbeille, est ignorée sans être remplacée. Le résultat distingue les notes ajoutées et les identifiants conservés.

Le mode **Importer des copies** crée de nouveaux identifiants pour les notes libres et réécrit leurs liens internes vers les copies du même lot. Les positions de surlignage et les références des versions sont ajustées. Les notes protégées conservent leurs identifiants, puisque leur contenu ne peut pas être réécrit sans leur mot de passe individuel.

Les profils d'origine sont réutilisés par identifiant puis par nom, ou créés s'ils manquent. Le mode profil de destination regroupe les notes ajoutées dans le profil choisi.

## Markdown et texte externes

- Les fichiers sont lus en UTF-8 ; le BOM UTF-8 et les fins de ligne Windows sont normalisés.
- Le titre vient des métadonnées simples, d'un premier titre Markdown `# ...` ou du nom du fichier.
- Les champs simples `title`, `tags`, `created`/`created_at`/`date` et `profile` sont reconnus dans un en-tête. Les en-têtes tiers restent dans le contenu pour conserver leurs informations supplémentaires.
- Les liens Markdown locaux simples, sans ancre, vers d'autres notes sélectionnées deviennent des références stables de l'application. Les liens externes, les ancres, le code et les structures complexes gardent leur texte ; aucune URL n'est téléchargée.
- Une ressource locale référencée et sélectionnée devient une pièce jointe. Plusieurs ressources sont regroupées dans une pièce jointe `resources.zip`, avec leurs chemins relatifs sûrs. L'application conserve ainsi ses fichiers tout en utilisant son modèle d'une pièce jointe par note.
- Les fichiers non utilisés et les métadonnées système ignorées sont comptés dans l'aperçu. Les sources sélectionnées ne sont jamais modifiées.

## ZIP de l'application, version 1

```text
manifest.json
notes/0001-titre.md       # ou .txt
notes/0002-autre_note.md
attachments/0001-fichier.bin
```

Le manifeste `securenotes-portable`, version 1, contient les profils et les métadonnées des notes, leur source originale, les chemins des documents/pièces jointes et l'empreinte SHA-256 de chaque présentation exportée.

- Le Markdown contient un en-tête compatible YAML utilisant des valeurs JSON, et des liens Markdown relatifs vers les notes du lot. Les pièces jointes sont également référencées en fin de document.
- Un document inchangé restaure sa source originale et ses métadonnées, sans ajouter l'en-tête de présentation au contenu.
- Si un document a été édité à l'extérieur, son contenu est repris, les surlignages sont réinitialisés et la source d'origine est conservée comme version, dans la limite des 20 versions.
- Les archives avec un dossier enveloppe et des métadonnées `__MACOSX`/`.DS_Store` sont reconnues. Un paquet doit avoir un seul manifeste SecureNotes.

Le ZIP est lu en mémoire : aucun fichier n'est extrait vers le système de fichiers par le backend. Les chemins absolus, traversées de répertoires, entrées dupliquées, liens symboliques et pièces jointes déclarées mais absentes sont refusés. Les méthodes ZIP stockée et Deflate sont prises en charge.

## Archive chiffrée, version 1

L'enveloppe JSON utilise :

- `format: "securenotes-transfer"`, `version: 1` ;
- `algorithm: "argon2id-aes256gcm-v1"` ;
- sel aléatoire de 16 octets, clé de 32 octets dérivée par Argon2id, 64 Mio, 3 passes, parallélisme 1 ;
- AES-256-GCM avec nonce aléatoire de 12 octets ;
- sel, nonce et contenu chiffré encodés en base64.

Le mot de passe d'archive comporte au moins huit caractères et n'est jamais enregistré. Les titres, profils et données des notes sont à l'intérieur du contenu chiffré. Les champs chiffrés des notes individuellement protégées sont transportés sans déchiffrement et restent indépendants du mot de passe d'archive et du nouveau coffre.

## Limites et atomicité

- 1000 notes par opération ; 4096 fichiers maximum.
- 96 Mio pour les fichiers sélectionnés ; 64 Mio pour les données décompressées, sérialisées ou converties du paquet.
- Métadonnées des notes libres : 12 tags, 20 versions, 10000 surlignages, 100000 entrées de modification maximum.
- 10000 références ou structures Markdown par note lors des conversions.

La validation complète précède l'import. L'ajout utilise la persistance atomique du coffre : une erreur conserve les données précédentes et la préparation pour une nouvelle tentative. Les brouillons, objectifs et dossiers intelligents relèvent des sauvegardes du coffre ; les échanges sélectionnés portent sur les notes et leurs profils.
