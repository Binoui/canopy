# Canopy

Canopy est un prototype de navigateur Linux en Rust. Tes visites forment des arbres. Revenir sur une page puis suivre un autre lien conserve les deux branches.

## Lancer le navigateur

Installe Rust stable, GTK 3, WebKitGTK 4.1 et les fichiers de développement SQLite.

Sur Debian ou Ubuntu, installe les dépendances natives avec cette commande :

```sh
sudo apt install build-essential pkg-config libgtk-3-dev libwebkit2gtk-4.1-dev libsqlite3-dev
```

Sur Arch Linux, utilise cette commande :

```sh
sudo pacman -S --needed base-devel pkgconf gtk3 webkit2gtk sqlite
```

Depuis le dossier du projet, lance cette commande :

```sh
cargo run --locked
```

Pour conserver les données dans un dossier distinct, indique son chemin :

```sh
cargo run --locked -- --data-dir .data
```

Le fichier `rust-toolchain.toml` sélectionne Rust stable pour ce projet. Il ne change pas ta version par défaut dans les autres projets.

## Essayer une bifurcation

1. Appuie sur **Ctrl+L**, saisis une adresse et valide.
2. Suis un lien vers une deuxième page.
3. Reviens à la première page avec le fil d'Ariane ou l'arbre.
4. Suis un autre lien. Les deux visites apparaissent sous la première page.
5. Ferme le navigateur, puis relance-le avec le même dossier de données. L'arbre et la position courante sont conservés.

Cliquer sur une ancienne visite recharge son URL. La reprise ne restaure pas le scroll ni les formulaires.

Pour commencer un arbre indépendant, utilise **Ctrl+T** ou **Nouvel arbre**. **Alt+Gauche** revient au parent et **Ctrl+R** recharge la visite courante.

## Reprendre une lecture

1. Depuis un commentaire ou un article, ouvre plusieurs liens avec **Ctrl+clic**, le **clic milieu** ou un lien destiné à une nouvelle fenêtre. Les destinations apparaissent en attente dans l'arbre. La page source reste ouverte et les destinations ne sont pas téléchargées.
2. Active un enfant en attente pour le lire. La visite conserve son parent et le contexte du lien quand il est disponible.
3. Laisse **Mémoriser les pages** enregistrer le texte de la page ouverte, ou sélectionne un passage et appuie sur **Ctrl+S**. Le bouton **Mémoriser la sélection** effectue la même action.
4. Ferme le navigateur, puis relance-le avec le même dossier de données.
5. Recherche quelques mots du texte enregistré. Ouvre la capture pour relire le passage hors ligne, ou reviens à la source pour retrouver les liens encore en attente.

Les résultats recherchent les titres, les URL, le texte enregistré et le contexte source. Une capture possède sa date. Ouvrir la capture et charger la page vivante sont deux actions distinctes.

Décoche **Mémoriser les pages** pour désactiver la capture automatique. La mémorisation d'une sélection reste disponible. **Lire une capture** permet de relire les textes enregistrés pour la page courante.

Les champs de formulaire et les zones éditables sont exclus du texte enregistré. Les formulaires de mot de passe visibles bloquent la capture automatique.

La récupération du commentaire ou du permalien dépend du DOM du site. Si ce contexte fin manque, la page parent reste conservée. Le corpus Chrome importé reste séparé de la navigation vivante.

Pour un post qui apparaît plus tard dans un fil dynamique, sélectionne le passage et utilise **Ctrl+S**. La capture automatique ne mémorise pas en continu tout ce qui apparaît pendant le défilement.

Sans `--data-dir`, Canopy conserve la base et le profil WebKit dans `$XDG_DATA_HOME/canopy`, ou dans `~/.local/share/canopy`.

## Vérifier le projet

Vérifie le format, les tests du cœur et les diagnostics du compilateur :

```sh
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

Le test WebKit charge des pages servies localement. Installe Xvfb et xdotool, puis lance-le ainsi :

```sh
xvfb-run -a -s '-screen 0 1440x1000x24' cargo test --locked --features gui-smoke --test webkit_smoke
xvfb-run -a -s '-screen 0 1440x1000x24' cargo test --locked --features gui-smoke --test reading_smoke
```

Les choix de structure sont décrits dans [Une forêt de visites](docs/design.md) et [La mémoire d'une exploration](docs/reading-memory.md).
