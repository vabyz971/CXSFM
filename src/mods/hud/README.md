# HUD perso (hud)

HUD dessinés par CXSFM par-dessus le jeu, décrits dans un fichier TOML
et **rechargés à chaud** : tu enregistres, le résultat apparaît en jeu
en ~1 s. Cosmétique uniquement : le mod **lit** des valeurs du jeu, il
n'écrit jamais rien dedans.

- Fichier : `~/.config/cxsfm/hud.toml` (créé avec un exemple à la
  première activation de la tuile **HUD perso**).
- Fenêtre du mod (menu ouvert) : chemin du fichier, HUD actifs,
  erreurs/avertissements de la config, valeurs lues en direct, onglet
  **Explorateur** pour trouver de nouvelles valeurs.
- Les HUD restent visibles menu fermé et ne prennent jamais la souris
  ni le clavier.

## 1. Structure du fichier

```toml
version = 2
fps = 30          # rafraîchissement des HUD (5..60)
mode = "live"     # "live" = valeurs du jeu, "demo" = valeurs factices animées

[sources]         # les valeurs du jeu, par nom (voir §3)
speed = "UITachometer/m_currentSpeed"

[[hud]]           # autant de [[hud]] que tu veux
name = "Mon HUD"
  [[hud.widget]]  # les éléments du HUD, dessinés dans l'ordre
  type = "text"
  text = "{speed}"
```

Une erreur de **syntaxe** TOML garde la dernière config valide à
l'écran (l'erreur s'affiche dans la fenêtre du mod). Une clé ou un
widget invalide est seulement **ignoré**, avec un avertissement.

`mode = "demo"` sert à travailler la mise en page sans rouler : il
anime `speed`, `gear`, `max_gear`, `rpm`, `rpm_min`, `rpm_max`,
`redline` dans les mêmes unités que le jeu.

## 2. Positions, tailles, angles

Chaque `[[hud]]` est une **boîte** posée sur l'écran :

| Clé | Rôle | Défaut |
|---|---|---|
| `enabled` | affiche ce HUD | `true` |
| `anchor` | coin/bord de l'écran : `top_left`, `top`, `top_right`, `left`, `center`, `right`, `bottom_left`, `bottom`, `bottom_right` | `bottom_right` |
| `offset` | décalage `[x, y]` en pixels depuis l'ancre | `[0, 0]` |
| `size` | taille `[l, h]` de la boîte (avant `scale`) | `[300, 100]` |
| `scale` | zoom de tout le HUD | `1.0` |
| `opacity` | opacité de tout le HUD | `1.0` |
| `background` | fond de la boîte | transparent |
| `hide_when_missing` | cache le HUD quand aucune valeur n'est lue (menus, chargement) | `true` |

Le coin de la boîte qui correspond à l'ancre se pose sur ce coin de
l'écran, puis `offset` le déplace (`offset = [-48, -48]` avec
`bottom_right` = 48 px du bord bas-droit).

**Dans la boîte**, chaque widget se place avec `pos = [x, y]` (pixels,
origine en haut à gauche de la boîte, multipliés par `scale`). Un
widget peut **dépasser** de la boîte (`pos` négatif ou plus grand que
`size`) : la boîte sert de repère, pas de cadre.

**Angles** (arcs, aiguille, graduations, `rotation` de tous les
widgets) : en **degrés**, **0 = en haut**, **sens horaire**.
`90` = droite, `180` = bas, `-90` = gauche.

**Tous les widgets** acceptent `pos`, `rotation` et `opacity`
(`0.0..1.0`, multipliée par l'opacité du HUD), ainsi que `alert` +
`alert_color` (couleur partagée, voir §3 bis).

## 3. Sources : brancher une valeur du jeu

```toml
[sources]
speed    = "UITachometer/m_currentSpeed"
gear     = "UITachometer/m_gear"
max_gear = "UITachometer/m_maxGears"
rpm      = "UITachometer/m_rpmFillImage/m_FillAmount"
rpm_min  = "UITachometer/m_minRpmFillAmount"
rpm_max  = "UITachometer/m_maxRpmFillAmount"
# zone rouge mesurée depuis l'autre bout de la jauge → 1 - valeur
redline  = { path = "UITachometer/m_rpmRedZoneImage/m_FillAmount", scale = -1.0, offset = 1.0 }
```

- Forme courte : `nom = "Classe/référence/…/champ"`.
- Forme longue : `nom = { path = "…", scale = 1.0, offset = 0.0 }` →
  valeur affichée = `valeur × scale + offset` (inverser, convertir une
  unité, décaler un rapport…).
- `speed_mph` existe toujours en plus (dérivée de `speed` en km/h).
  `m_currentSpeed` suit déjà l'unité choisie dans le jeu.
- **Trouver un chemin** : onglet **Explorateur** → cherche une classe
  (ex. `Tachometer`) → clique → les champs s'affichent avec leur
  valeur ; `→` suit une référence ; **Relire** après avoir roulé pour
  voir ce qui bouge ; **survole un champ** pour lire son chemin.
- Le RPM du jeu n'est pas en tr/min : c'est le **taux de remplissage**
  de la jauge (`rpm`), entre `rpm_min` et `rpm_max`. Donne ces deux
  sources comme bornes aux widgets (voir plus bas) et la jauge suit la
  plage propre à chaque voiture.

### Bornes `min` / `max` / `redline`

Partout où un widget accepte `min`, `max`, `redline`, tu peux mettre
**un nombre** (`max = 8000`) **ou un nom de source** (`max = "rpm_max"`).
Un widget dont la source ou une borne manque n'est pas dessiné.

## 3 bis. États et couleur partagée (`[states]`, `alert`)

Un **état** est une condition nommée, calculée une fois par image et
réutilisable par autant de widgets que tu veux. Exemple : tout passe
en rouge quand le régime est dans la zone rouge.

```toml
[states]
rouge  = { source = "rpm", above = "redline" }             # rpm ≥ redline
shift  = { source = "rpm", above = "redline", blink = 6 }  # pareil, clignote 6×/s
lent   = { source = "speed", below = 30 }                  # vitesse ≤ 30
plage  = { source = "speed", above = 80, below = 120 }     # 80 ≤ vitesse ≤ 120
neutre = { source = "gear", below = 0.5, above = -0.5 }    # rapport N
```

| Clé | Rôle | Défaut |
|---|---|---|
| `source` | valeur testée (obligatoire) | — |
| `above` | actif si valeur **≥** ce nombre ou cette source | aucune |
| `below` | actif si valeur **≤** ce nombre ou cette source | aucune |
| `blink` | clignotement en Hz (`0` = fixe) | `0` |

Sans `above` ni `below`, l'état est actif quand la valeur n'est pas 0.

Puis, sur **n'importe quel widget** :

```toml
  [[hud.widget]]
  type = "text"
  text = "{speed}"
  alert = "rouge"          # nom d'un état de [states]
  alert_color = "#E03030"  # couleur tant que l'état est actif (défaut : rouge)
```

La couleur **principale** du widget change : remplissage d'une barre
ou d'un arc, aiguille, texte ; pour `ticks`, traits **et** chiffres.
Les fonds (`track_color`) et `redline_color` gardent leur rôle. Un nom
d'état inconnu est signalé dans les avertissements.

## 4. Les widgets

### `bar` — barre horizontale

| Clé | Rôle | Défaut |
|---|---|---|
| `source` | valeur affichée (obligatoire) | — |
| `min`, `max` | bornes (nombre ou source) | `0`, `1` |
| `redline` | à partir de cette valeur, la barre passe en `redline_color` ; la zone est rappelée en sombre sur le fond | aucune |
| `pos`, `size` | coin haut-gauche, `[longueur, épaisseur]` | `[0,0]`, `[200,8]` |
| `segments` | `0` = continue, sinon nombre de pavés | `0` |
| `fill` | `"left"` (depuis la gauche) ou `"right"` (depuis la droite jusqu'à la valeur) | `"left"` |
| `rotation` | rotation autour de `pos` (`90` = barre verticale vers le bas, `-90` = vers le haut) | `0` |
| `color`, `redline_color`, `track_color` | remplissage, zone rouge, fond | blanc, rouge, blanc 16 % |

### `arc` — cercle, demi-cercle, arc, disque, camembert

| Clé | Rôle | Défaut |
|---|---|---|
| `pos` | **centre** | `[50, 50]` |
| `radius`, `thickness` | rayon, largeur de l'anneau | `50`, `6` |
| `start`, `end` | angles de début/fin (`0→360` = cercle complet, `-90→90` = demi-cercle haut, `90→270` = bas) | `-135`, `135` |
| `rotation` | décale `start` et `end` | `0` |
| `filled` | `true` = disque / part pleine au lieu d'un anneau | `false` |
| `color`, `track_color` | couleur (fixe ou remplissage), fond de l'arc | blanc, blanc 16 % |
| `source`, `min`, `max`, `redline`, `redline_color` | optionnel : l'arc se remplit selon la valeur, comme une barre | statique |
| `fill` | `"start"` (depuis `start`) ou `"end"` (depuis `end` jusqu'à la valeur) | `"start"` |

Sans `source`, c'est une forme fixe en `color` (décor, cadran, rond).

### `needle` — aiguille

| Clé | Rôle | Défaut |
|---|---|---|
| `source`, `min`, `max` | valeur et bornes (obligatoire : `source`) | —, `0`, `1` |
| `pos` | centre de rotation | `[50, 50]` |
| `start`, `end` | angle pour `min` et pour `max` | `-135`, `135` |
| `rotation` | décale `start`/`end` | `0` |
| `length` | rayon de la pointe | `50` |
| `inner` | rayon où l'aiguille commence (laisse un trou au centre) | `0` |
| `tail` | queue derrière le centre | `0` |
| `width` | épaisseur | `3` |
| `color`, `redline`, `redline_color` | couleur, passe en rouge à `redline` | blanc, aucune, rouge |

### `ticks` — graduations et chiffres d'un cadran

| Clé | Rôle | Défaut |
|---|---|---|
| `pos`, `radius` | centre, rayon extérieur des traits | `[50,50]`, `50` |
| `start`, `end`, `rotation` | comme `arc` | `-135`, `135`, `0` |
| `count` | nombre d'intervalles majeurs (`8` → 9 grands traits), ou `"auto"` = un grand trait par unité de chiffre | `8` |
| `minor` | sous-divisions par intervalle (`4` → 3 petits traits entre deux grands) | `4` |
| `major_length`, `minor_length` | longueur des traits (vers l'intérieur) | `10`, `5` |
| `major_width`, `minor_width` | épaisseur | `2`, `1` |
| `color`, `minor_color` | couleur des grands / petits traits | blanc 63 % |
| `labels` | chiffres aux grands traits | `true` |
| `label_min`, `label_max`, `label_decimals` | chiffres de `label_min` à `label_max` ; `label_max` = nombre **ou source** | `0`, `count`, `0` |
| `label_margin` | ajouté à `label_max` (marge au-delà du max) | `0` |
| `label_radius` | rayon des chiffres | sous les traits |
| `label_size`, `label_color` | taille, couleur | `14`, blanc 63 % |

Les chiffres restent droits (lisibles), seuls leurs emplacements
suivent le cercle.

**Cadran automatique selon la voiture** : avec une source qui donne
le régime max en milliers, le cadran s'adapte tout seul (7000 tr/min
+ marge 2 → chiffres 0 à 9, 9 intervalles) :

```toml
[sources]
max_rpm_k = { path = "Classe/…/champ_regime_max", scale = 0.001 }

  [[hud.widget]]
  type = "ticks"
  count = "auto"
  label_max = "max_rpm_k"
  label_margin = 2
```

Le chemin du régime max est à trouver avec l'Explorateur (les
sources actuelles `rpm*` sont des taux de remplissage, pas des tr/min).
Si la source manque, le widget n'est pas dessiné.

### `text` — texte et valeurs

| Clé | Rôle | Défaut |
|---|---|---|
| `text` | texte, avec des valeurs `{nom}` | `""` |
| `pos` | point d'alignement (haut du texte) | `[0, 0]` |
| `size` | taille | `16` |
| `align` | `"left"`, `"center"`, `"right"` (par rapport à `pos`) | `"left"` |
| `mono` | police à chasse fixe (chiffres qui ne « dansent » pas) | `false` |
| `rotation` | rotation autour de `pos` | `0` |
| `color` | couleur | blanc |

Valeurs dans le texte :

- `{nom}` → la source, sans décimale (`{speed}` → `139`) ;
- `{nom:2}` → avec 2 décimales (`{rpm:2}` → `0.46`) ;
- `{gear}` → `R`, `N`, `1`, `2`… (`{gear:0}` donne le nombre brut) ;
- `{speed_mph}` → vitesse convertie en mph ;
- un nom inconnu reste affiché tel quel (`{boost}`) : une faute de
  frappe se voit tout de suite.

Pas d'italique (egui n'en a pas) : pour un effet incliné, une petite
`rotation` sur le texte s'en approche.

## 5. Exemple complet : compteur à aiguille

Cadran gradué 0 → 8, zone rouge en fin de course, aiguille, rapport
dans un rond vert au centre, grosse vitesse en bas à droite. Colle ce
`[[hud]]` dans ton fichier (avec les `[sources]` du §3), passe l'autre
HUD à `enabled = false` si besoin, enregistre.

```toml
[[hud]]
name = "Compteur aiguille"
enabled = true
anchor = "bottom_right"
offset = [-40.0, -40.0]
size = [380.0, 340.0]

  # fin trait du cadran
  [[hud.widget]]
  type = "arc"
  pos = [170.0, 170.0]
  radius = 150
  thickness = 2
  start = -140
  end = 125
  color = "#FFFFFF50"

  # graduations + chiffres 0..8
  [[hud.widget]]
  type = "ticks"
  pos = [170.0, 170.0]
  radius = 148
  start = -140
  end = 125
  count = 8
  minor = 4
  major_length = 12
  minor_length = 6
  label_min = 0
  label_max = 8
  label_size = 16
  color = "#FFFFFFA0"
  minor_color = "#FFFFFF50"
  label_color = "#FFFFFFA0"

  # zone rouge : de la redline jusqu'au bout du cadran
  [[hud.widget]]
  type = "arc"
  source = "redline"
  min = "rpm_min"
  max = "rpm_max"
  fill = "end"
  pos = [170.0, 170.0]
  radius = 146
  thickness = 6
  start = -140
  end = 125
  color = "#E0304A"
  track_color = "#00000000"

  # aiguille
  [[hud.widget]]
  type = "needle"
  source = "rpm"
  min = "rpm_min"
  max = "rpm_max"
  redline = "redline"
  pos = [170.0, 170.0]
  start = -140
  end = 125
  inner = 34
  length = 140
  width = 4
  color = "#FFFFFF"
  redline_color = "#E0304A"

  # rond central : fond sombre + anneau vert
  [[hud.widget]]
  type = "arc"
  pos = [170.0, 170.0]
  radius = 28
  start = 0
  end = 360
  filled = true
  color = "#14201CD0"

  [[hud.widget]]
  type = "arc"
  pos = [170.0, 170.0]
  radius = 22
  thickness = 3
  start = 0
  end = 360
  color = "#2EE59D"

  # rapport au centre
  [[hud.widget]]
  type = "text"
  text = "{gear}"
  pos = [170.0, 150.0]
  size = 30
  align = "center"
  color = "#2EE59D"

  # libellés
  [[hud.widget]]
  type = "text"
  text = "ABS"
  pos = [108.0, 150.0]
  size = 13
  align = "center"
  color = "#FFFFFF70"

  [[hud.widget]]
  type = "text"
  text = "TCR"
  pos = [108.0, 166.0]
  size = 13
  align = "center"
  color = "#FFFFFF70"

  [[hud.widget]]
  type = "text"
  text = "km/h"
  pos = [372.0, 228.0]
  size = 16
  align = "right"
  color = "#FFFFFF90"

  # grosse vitesse, légèrement inclinée
  [[hud.widget]]
  type = "text"
  text = "{speed}"
  pos = [376.0, 246.0]
  size = 84
  align = "right"
  rotation = -4
```

Pour la retoucher : tout le cadran partage `pos = [170, 170]` et les
mêmes `start`/`end` — change-les ensemble pour déplacer ou ouvrir le
cadran ; `scale` du `[[hud]]` agrandit l'ensemble.

## 6. Dépannage

- **Rien ne s'affiche** : fenêtre du mod → la ligne d'état dit
  `UITachometer : introuvable` hors course (normal, nouvelle recherche
  toutes les 5 s) ; sinon lis les avertissements de la config.
- **« mode live sans [sources] »** : ajoute une section `[sources]`
  (§3), ou supprime `hud.toml` pour régénérer l'exemple.
- **Une valeur reste `{nom}` à l'écran** : nom absent de `[sources]`
  ou faute de frappe.
- **Une barre/arc/aiguille manque** : sa source ou une borne
  (`min`/`max` en nom de source) n'est pas lue — vérifie la dernière
  ligne de la fenêtre du mod, qui liste toutes les valeurs.
- **Mise en page sans rouler** : `mode = "demo"`.
