# Une forêt de visites

Canopy conserve le contexte d'une navigation. Une visite a une URL, un titre et un parent facultatif. Chaque racine commence un arbre.

Après A puis B, revenir à A puis suivre C donne deux enfants de A. La visite B reste accessible. Suivre à nouveau une URL peut créer une autre visite, car le contexte a changé.

## SQLite possède la forêt et le curseur

La base contient les visites et la position courante. L'insertion d'une visite et le déplacement du curseur partagent une transaction. Les listes affichées par GTK sont des projections de cette base.

Deux structures ont été comparées avant l'implémentation. La première donnait la propriété de l'état à SQLite. La seconde maintenait aussi une forêt en mémoire. La première a été retenue pour éviter un second chemin d'écriture et la synchronisation du curseur.

L'alternative a apporté deux éléments au design retenu. L'intention de navigation reste explicite jusqu'au chargement effectif. Les tests vérifient le comportement avec WebKit et comparent la forêt avant et après réouverture de la base.

## Une intention précède une visite

Une nouvelle navigation retient son parent avant le chargement. Une restauration retient l'identifiant de la visite à réutiliser. Le moteur confirme ensuite l'URL au chargement du document principal.

Les redirections serveur appartiennent à une seule navigation. Un rechargement ou une restauration ne crée pas de visite. Un échec conserve la dernière position enregistrée et affiche une erreur.

Les identifiants persistants sont des ULID. Les URL ne sont pas des identifiants de visite. Les sous-ressources et les cadres intégrés ne deviennent pas des enfants.

## GTK et WebKit restent sur leur thread

wry construit la webview dans un conteneur GTK. Son extension Unix donne accès aux signaux natifs de WebKitGTK. Ces signaux permettent de distinguer le début, la validation et l'échec d'un chargement.

Les emprunts de l'état sont relâchés avant les appels qui peuvent déclencher un autre callback. Une seule webview charge la page active. Les autres visites restent dans SQLite.

Le retour du navigateur suit le parent dans la forêt. La liste arborescente permet de choisir une autre branche. La reprise d'une visite recharge son URL. Elle ne restaure pas un instantané du DOM, du scroll ou des formulaires.

## Le périmètre du MVP1

Le premier jalon vérifie la navigation, les bifurcations et la reprise après redémarrage. Le chrome utilise GTK pour limiter le travail d'intégration autour de WebKitGTK.

La mini-carte, le layout de graphe, l'extraction des pages, les résumés et l'agent attendent un prochain jalon. Le moteur n'a pas de trait générique. Une migration vers Servo ou CEF demandera de revoir l'intégration du rendu et du profil.

Les changements internes des applications web restent un cas distinct. Le MVP ne promet pas de transformer chaque changement d'état JavaScript en visite.
