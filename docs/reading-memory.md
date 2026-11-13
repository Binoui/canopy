# La mémoire d'une exploration

Le scénario de référence commence par un commentaire qui contient trois liens. Les trois destinations restent dans l'arbre, une seule est ouverte, puis le navigateur est fermé. À la reprise, une recherche dans le texte enregistré retrouve l'article, son commentaire d'origine et les deux liens en attente.

## Un lien en attente est une visite à ouvrir

Une visite peut être en attente ou ouverte. Un clic qui ouvre normalement un onglet en arrière-plan crée un enfant en attente, sans changer la page courante ni charger la destination. L'activation de cet enfant conserve son identifiant. Le chargement effectif le marque comme ouvert.

Un échec conserve l'enfant en attente et la dernière page courante. Une redirection appartient au même enfant. Deux commentaires peuvent contenir la même destination et produire deux enfants distincts, chacun avec sa source.

Les visites du MVP1 restent ouvertes après migration. SQLite conserve la forêt, les états et le curseur. Les widgets affichent des projections de ces données.

## La source appartient au contexte du clic

Un enfant peut conserver l'URL de la page source, le texte du lien, un extrait du commentaire et une citation plus précise. La page source et la citation sont deux choses distinctes. Une citation peut désigner un commentaire particulier dans une page ou un autre permalien du même site.

Revenir à la source restaure la visite parent. Ouvrir une citation distincte ne transforme pas l'URL du parent en une autre page.

Les signaux natifs de WebKit décident de la destination et du mode d'ouverture. Un script limité au document principal fournit du contexte dans un monde JavaScript isolé. Ce script ne reçoit pas d'identifiant de visite ni de moyen d'écrire dans la base.

L'extrait est associé au lien seulement si la destination et le document correspondent encore. Une information manquante reste manquante. Elle ne doit pas être remplacée par le contexte d'un autre clic.

## Une capture est un texte daté

Les captures sont des textes locaux associés à une visite et à l'URL du document. Elles conservent leur date. Une nouvelle version peut s'ajouter quand le contenu change. Les captures automatiques identiques n'ont pas besoin d'être dupliquées.

La capture automatique concerne les pages ouvertes. Des vérifications différées et bornées permettent de récupérer du contenu qui apparaît peu après le chargement. Les liens en attente ne sont pas téléchargés pour être indexés. Une action explicite permet aussi de mémoriser du texte sélectionné dans une page ou un fil.

Chaque texte est limité à 200 000 caractères. L'extraction écarte les champs de formulaire et les zones éditables, même quand ils sont masqués. Un formulaire de mot de passe visible bloque la capture automatique. Un contrôle visible permet de désactiver la capture automatique.

Le document affiché possède une identité distincte de la navigation demandée. Une extraction asynchrone retient cette identité. Un résultat tardif ne doit pas devenir la capture d'une autre visite, même si les deux visites ont la même URL.

## La recherche utilise les textes enregistrés

FTS5 indexe le texte des captures, leur titre et leur contexte source. Les titres et les URL restent recherchables quand une visite ne possède pas de capture. Les résultats permettent de distinguer plusieurs textes mémorisés dans un même fil.

La lecture d'une capture utilise une vue de texte native et fonctionne sans recharger le site. Ouvrir la page vivante ou revenir à sa source reste une action séparée.

Le corpus Chrome importé reste un jeu de données indépendant. La fonctionnalité ne télécharge pas ses anciennes URL et ne prétend pas reconstituer les publications lues dans un fil à partir de l'historique.

## Les choix retenus

Deux propositions ont été comparées. La base retenue garde l'autorité de navigation côté natif, des captures par visite et une recherche FTS5. L'alternative a apporté l'isolation du script de contexte et des vérifications sur les entrées utilisateur.

La fusion des liens en attente par URL a été écartée, car elle perdrait les sources distinctes. Un modèle global de documents canoniques et le remplacement des anciennes captures ne sont pas nécessaires au scénario de reprise.

La reconnaissance précise des commentaires dépend du DOM du site. Quand le contexte fin est indisponible, le lien doit toujours conserver sa page parent. Les captures sont des textes, pas des copies complètes et interactives des pages. Le contenu d'un fil qui apparaît longtemps après le chargement peut être mémorisé par sélection explicite. Le défilement n'est pas enregistré en continu.
