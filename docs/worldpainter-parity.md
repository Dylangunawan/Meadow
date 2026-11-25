# Making Meadow feel like WorldPainter

The goal: someone who knows WorldPainter can sit down at Meadow and work without relearning keys, mouse
controls, panel layout or how the map looks. Meadow's extras (real 3D view, custom masks, Conquest blocks,
tall worlds…) stay, but must not get in the way of WorldPainter habits.

**Sources.** WorldPainter's own source code (`Captain-Chaos/WorldPainter`, mainly `App.java`,
`TileRenderer.java`, `SymmetricBrush.java`, `ScrollController.java`, `MouseOrTabletOperation.java`, the layer
renderers and paint classes) and the map viewer from Pepsoft's `SwingUtils` (`TiledImageViewer.java`).
worldpainter.net could not be reached from the build machine, so nothing here comes from its docs. Meadow
copies *behaviour* (key bindings, layout, formulas); it must not copy WorldPainter's code or icon files
(GPLv3), so all icons and code are written fresh.

Each section lists what WorldPainter does, what Meadow does today, and the change.

---

## 1. Mouse controls (highest priority)

| Action | WorldPainter | Meadow today | Change |
|---|---|---|---|
| Use tool | Left button | Left button | — |
| Inverse of tool (lower, erase, reset) | **Right button**, or Alt + left | Shift + left | Right button and Alt + left; drop Shift |
| Pan the map | **Middle-drag**, or hold **Space** and drag | Right/middle-drag, Space + drag | Middle-drag and Space + drag only (right is now "inverse") |
| Mouse wheel | **Brush radius** (×1.1 per notch up, ×0.9 down) | Zoom | Brush radius |
| Shift + wheel | Radius ±1 | — | Add |
| Ctrl + wheel | **Zoom** (one power of two per notch, around the cursor) | — | Add |
| Alt + wheel | Rotate brush ±15° | — | Add (with brush rotation, §5) |

Radius is an integer from 0 to 300, starts at **50**, and is shown in the status bar (§3).

## 2. Keyboard shortcuts

WorldPainter's complete shortcut set (Ctrl is ⌘ on macOS):

| Key | WorldPainter | Meadow today | Change |
|---|---|---|---|
| **1 … 9, 0** | **Brush intensity 10 % … 90 %, 100 %** | Select tools 1–9 | Intensity, exactly as WorldPainter |
| Alt + R / P / L / X | Spray Paint / Pencil / Fill / Text tool | — | Add with the tools (§4) |
| Alt + F | Flood | — | Add |
| Alt + H / A / S / M | Height / Flatten / Smooth / Raise Mountain | — | Add |
| R / L | Rotate the map's light clockwise / anticlockwise | — | Add (§8) |
| Esc | Leave the current operation (deselect tool) | — | Add |
| Ctrl + N / O / S | New / Open / Save | Same | — |
| Ctrl + Shift + S | Save as | — | Add |
| Ctrl + E | Export as Minecraft map | Same | — |
| Ctrl + I | Import existing Minecraft map | — | Add when map import exists; reserve the key |
| Ctrl + R | Merge into existing map | — | Reserve |
| Ctrl + Z / Y | Undo / Redo | Same (+ Ctrl+Shift+Z) | Keep |
| Ctrl + + / − / 0 | Zoom in / out / reset (100 %) | — | Add |
| Ctrl + G | Global operations (fill/clear by filter) | Tools menu only | Add; it is Meadow's "Fill / clear through mask" |
| Ctrl + T | Add/remove tiles dialog | Paint tool only | Add dialog (§10) |
| Ctrl + P | Dimension properties | "World settings" | Add, split per dimension |
| Ctrl + U / H / D | View Surface / Nether / End | Dimension menu | Add |
| Ctrl + C / M | Switch to/from ceiling / master dimension | — | Reserve (no ceiling/master dimensions yet) |
| Ctrl + V | Configure view | Preferences | Add a "Configure view" dialog (§8) |
| Ctrl + 3 | Show 3D view | Tab cycles views | Ctrl + 3 opens/focuses 3D; keep Tab as a Meadow extra |
| F1 | Documentation | — | Add (open README / docs) |
| 3D view: L / R | Rotate view 90° | — | Add (§9) |
| 3D view: + / − / 0 | Zoom in / out / reset | Wheel only | Add |

Meadow extras that don't clash and can stay: `[` / `]` radius, `F` frame world, `Tab` cycle 2D/3D/split,
WASD/QE flying in 3D (only while the 3D view has focus).

## 3. Window layout

WorldPainter's default layout:

```
┌ Menu bar: File  Edit  View  Tools  Help ──────────────────────────────────────────────┐
├ Toolbar (icons): New Open Save Export │ Undo Redo │ Zoom− Zoom100% Zoom+ │ Dimension      │
│   properties, Change height, Rotate, Shift, Scale, Add/remove tiles │ Move to spawn,  │
│   Move to origin │ Grid, Contours, Overlays, View distance, Walking distance toggles │  │
│   Rotate light ◀ ▶                                                                    │
├───────────────┬──────────────────────────────────────────────────────┬──────────────┤
│ Tools         │                                                      │ Brushes      │
│ Tool Settings │                     map view                         │ (Custom      │
│ ┌──────────┐  │                                                      │  Brushes)    │
│ │Layers    │◀ side tabs                                              │ Brush        │
│ │Terrain   │  │                                                      │  Settings    │
│ │Biomes    │  │                                                      │ Info         │
│ │Annotation│  │                                                      │              │
├───────────────┴──────────────────────────────────────────────────────┴──────────────┤
│ Status: Location x, z │ Height h of max │ Slope n° │ Material │ Fluid level (depth) │   │
│         Biome │ Radius r │ Zoom % │ memory                                             │
└──────────────────────────────────────────────────────────────────────────────────────┘
```

Changes:

* **Move panels to WorldPainter's sides.** Left: Tools, Tool Settings, then Layers / Terrain / Biomes /
  Annotations as **tabs with the tab strip on the left edge**. Right: Brushes, Brush Settings, Info.
  Meadow's extra panels (Custom masks) go in the left tab group. Meadow's panel placement customisation stays,
  with a **"Reset workspace layout"** command, as WorldPainter has.
* **Add the icon toolbar** in WorldPainter's order (Meadow currently has none). Put Meadow's 2D/3D/Split
  switch at its right end.
* **Menus in WorldPainter's structure:**
  * **File:** New, Open, Recent, Save, Save as, Import new world (from height map), Export (as Minecraft map,
    as image, as height map), Merge, Exit.
  * **Edit:** Undo, Redo, Global operations, Add/remove tiles, Change height, Rotate, Shift, Scale,
    Dimension properties, Change map format, Dimensions submenu (add/remove Nether, End), Import submenu
    (custom terrains, height map into current dimension, mask as terrain or layer), Delete unused layers,
    Preferences.
  * **View:** Zoom, Grid, Contours, Overlays, Surface/Nether/End, Change colour scheme, Configure view,
    Workspace layout, Show 3D view.
  * **Tools:** Open custom brushes folder, Open custom materials folder, Biomes viewer, Run script.
  * **Help:** Documentation, About.

  Meadow-only items (masks, block browser, mod jar import, Higher Heights) go where they fit.
* **Status bar text in WorldPainter's wording and order:** `Location: x, z`, `Height: h of max`,
  `Slope: n°`, `Material: <terrain>`, `Fluid level: w (depth d)`, `Biome: <name> (ID n)` or
  `Auto biome: …`, `Radius: r`, `Zoom: z %`.
* **Info panel** (right dock): everything under the cursor, including every layer value. Today Meadow
  squeezes this into the status bar.

## 4. Tools and the paint model

The biggest difference is conceptual. In WorldPainter you **pick a paint** (any terrain, layer, biome or
annotation colour) in the Layers/Terrain/Biomes/Annotations panels, then **pick a tool that applies it**:

* **Spray Paint (Alt+R):** soft, random, uses brush strength.
* **Pencil (Alt+P):** hard-edged dots and lines.
* **Fill (Alt+L):** flood fill a connected area.
* **Text (Alt+X):** draw text with the paint.

Meadow instead has separate "Paint terrain / Paint layer / Erase layer / Paint biome" tools.

WorldPainter's Tools panel is a **4-column grid of icon buttons**, in this order:

| | | | |
|---|---|---|---|
| Spray Paint (R) | Pencil (P) | Fill (L) | Text (X) |
| Flood (F) | Flood with lava | Sponge | Eyedropper |
| Height (H) | Flatten (A) | Smooth (S) | Raise Mountain (M) |
| Set spawn | Global operations | Raise rotated pyramid | Raise pyramid |
| Edit selection | Copy selection | Clear selection | |

Changes:

* Replace Meadow's paint tools with **Spray Paint, Pencil and Fill** that apply the selected paint. Clicking
  a terrain, layer, biome or annotation colour selects it as the paint (it does today too, but switches tool).
* **Height** is one tool: left raises, right lowers (replaces Raise + Lower).
* **Right-click with each paint type:**
  * terrain: reset the ground to the **theme** (§11);
  * layer: remove it;
  * biome: reset to automatic.
* **Add the missing tools:**
  * Raise Mountain (instant mountain), Raise Pyramid, Raise Rotated Pyramid;
  * Flood with lava (separate button);
  * Eyedropper (pick the paint under the cursor);
  * Text;
  * the selection tools (Edit / Copy / Clear selection), which come later as a bigger feature.
* Meadow-only tools (Roughen, Terrace, Water level, Paint mask, Add/remove tiles) go in an extra row under
  the WorldPainter grid, clearly separated.
* Icons: draw a simple consistent icon set (no copying WorldPainter's).

## 5. Brushes and Brush Settings

WorldPainter's Brushes panel is a **3-column grid of falloff × shape**:

| | | |
|---|---|---|
| Spike circle | Spike square | Noise (bitmap) |
| Linear circle | Linear square | Cracks (bitmap) |
| Sine circle (default) | Sine square | Constant circle |
| Plateau circle | Plateau square | Constant square |
| Dome circle | Dome square | |

Falloff of distance *d* (0 at the centre, 1 at the edge):

| Brush | Falloff |
|---|---|
| Linear | 1 − d |
| Sine | cos(d·π)/2 + 0.5 |
| Plateau | 1 for d < 0.5, else cos((d − 0.5)·2π)/2 + 0.5 |
| Spike | (1 − d)² |
| Dome | √(1 − d²) |
| Constant | 1 |

Custom brushes are greyscale images, loaded from a "custom brushes" folder and shown in their own panel.
**Brush Settings** holds only **Intensity** (2–100 %, default 50 %, set with 1–0) and **Rotation**
(−180°…180°, Alt+wheel in 15° steps). There is **no radius slider**: radius comes from the wheel and shows in
the status bar.

Meadow today: a shape toggle plus a separate falloff dropdown (Smooth/Linear/Sharp/Constant), a radius
slider, intensity 0.01–1 and a roughness slider. Change to the grid above, with the exact formulas, bitmap and
custom-image brushes, and rotation. Roughness can stay as a Meadow extra in Brush Settings.

## 6. Tool Settings (filters)

Tool Settings shows the current tool's options. For paint tools it also has WorldPainter's filter row:

* **only on** [paint… ▾] (Add another / More)
* **except on** [paint… ▾]
* **above** / **at or above** [level], **below** / **at or below** [level], **feather**
* **above / below** [n] **degrees**
* **inside selection** / **outside selection**

Height, Flatten, Smooth and Raise Mountain also have **Apply theme** (re-terrain the area by height as it
changes, §11).

Meadow today: one "Filter" dropdown that picks a custom mask. Change: build WorldPainter's filter row
exactly. Meadow's custom masks become one extra choice in "only on" / "except on" ("Custom mask: …"),
which keeps the improvement without changing the workflow.

## 7. Layers, Terrain, Biomes, Annotations panels

* **Layers:** each row has a **show** checkbox, a **solo** checkbox ("show only this layer; the rest is still
  exported") and a toggle button with the layer's icon and name, which selects it as the paint. At the top:
  **Terrain** and **Water/Lava** rows (show/solo only). Then the built-in layers, Populate, Read Only, and a
  **+** button for custom layers. Meadow today has an overlay checkbox and a colour swatch: add solo, icons and
  the Terrain and Water rows.
* **Terrain:** a **5-column grid of terrain icons** grouped as WorldPainter groups them (gaps between groups),
  with the custom-terrain **+** at the end:
  * Grass, Permadirt, Sand, Grass Path, Podzol
  * Bare Grass, Moss, Mud, Gravel, Clay
  * Stone Mix, Granite, Diorite, Andesite, Calcite
  * Cobblestone, Mossy Cobblestone, Stone, Deepslate, Rock
  * Tuff, Bedrock, Mycelium, Pale Moss
  * Obsidian, Magma, Lava
  * Desert, Sandstone, Red Sand, Red Sandstone, Red Desert
  * the 16 stained clays, Hardened Clay, Mesa
  * Beaches, Water, Deep Snow
  * Netherrack, Basalt, Blackstone, Netherlike
  * Soul Sand, Soul Soil, Warped Nylium, Crimson Nylium
  * End Stone

  Meadow today has colour squares in index order: change the order, and show small textured icons rather than
  flat squares. Meadow's newer terrains (Leaf Litter, Dry Grassland, Wildflower Meadow…) go in a "Meadow
  extras" group.
* **Biomes:** a grid of biome buttons with icons, plus automatic-biome choices. Meadow today has a searchable
  text list: switch to the grid, and keep search as an extra.
* **Annotations:** a panel of the 15 annotation colours to paint with. Meadow has the layer but no panel: add
  it.

## 8. 2D map rendering (the "look")

Exact behaviour of WorldPainter's renderer, and what Meadow should match:

| Element | WorldPainter | Meadow today |
|---|---|---|
| Background outside the world | **Light cyan `#C0FFFF`** (user-configurable) | Dark grey |
| Terrain colour | The **actual block at the surface** from the colour scheme (Minecraft texture colours), so mixed terrains (Rock, Stone Mix, Beaches) show **block-level speckle** | One flat colour per terrain |
| Lowest block | Bedrock colour when height = world minimum | Not handled |
| Hill shading | Brightness × (256 + 32 × (Δeast − Δwest + Δsouth − Δnorth)) / 256, i.e. **12.5 % per block of difference**, light from the **north-west**; R/L rotate the light (NW → NE → SE → SW); "above" = off | Smoothed slope × 0.18, fixed direction |
| Water and lava | **Flat** water or lava colour from the scheme with **no depth fade**, but still hill-shaded by the terrain *under* the water (so the lake bed's relief shows) and by the water surface's own slope | Blue blended by depth |
| Contours | **On by default, every 10 blocks**: a pixel is black if height % 10 = 0 and any neighbour is lower | Off by default |
| Frost | Lighten halfway to white: c' = 255 − (255 − c)/2 | Flat white |
| Forests (Deciduous, Pine, Jungle, Swamp) | **8×8 stamp patterns** (round tree, triangle, …) in very dark green (`#004000`, `#002000`, `#003000`), mixed with the ground by value/15 | Coloured wash |
| Caves / Caverns / Chasms / Resources | Transparent colour, mixed by value/15: Caves `#C000FF`, Caverns `#FF00FF`, Chasms `#C000FF`, Resources `#0000FF` | Coloured wash |
| Void | Light cyan `#C0FFFF`, the same as the background | Near-black |
| Grid | **Black** lines every 128 blocks (configurable), **coordinate labels** in 10 px text along the top/left edges, **bold** at region borders (every 512) | Faint tile outlines; chunk lines when zoomed in |
| Brush outline | Black 1 px line with a **white dashed** line on top (4 on / 6 off), in the brush's shape and rotation | Plain white circle |
| Spawn, centre, world border | **Spawn = small red crosshair**; small black-and-white crosshair at the view centre; Minecraft world border as a **red dashed** rectangle | Red circle with cross for spawn; no border |
| Zoom | **Powers of two only**, from 1:16 to 64:1 (zoom −4 … +6), shown as % | Continuous |

Changes: implement all of the above in the 2D renderer. For the colours to look like WorldPainter's, Meadow
needs real Minecraft texture colours. WorldPainter ships a Dynmap colour scheme; Meadow can't bundle Mojang's
textures, but it already reads jars. So **on first start, find the user's installed Minecraft client jar
(`.minecraft/versions/<latest>/<latest>.jar`) and import its block colours automatically**, with a menu item to
redo it (View › Change colour scheme). The built-in fallback colours stay for people without Minecraft
installed.

**Configure view (Ctrl+V)**, as in WorldPainter: grid on/off and size, contours on/off and spacing, background
colour, light direction, overlays (image overlay on the map), show borders/biomes.

## 9. 3D view

WorldPainter's 3D view is a separate window (Ctrl+3). It's an **isometric** picture that rotates in 90° steps
(L / R), zooms with + / − / 0, exports an image, has move-to-spawn and move-to-origin, and a "visible layers:
none / same as editor / all" switch.

Meadow's real-time 3D view is one of the requested improvements, so it stays. To make it feel familiar:

* Ctrl+3 shows it (as a tab, or detached into its own window).
* **L / R rotate 90°** (snapping the camera yaw), **+ / − / 0** zoom and reset, while the 3D view has focus.
* Add an **"Isometric" camera mode** (orthographic, 45° yaw, ~30° pitch) as the default framing, so it first
  looks like WorldPainter's view. Free perspective flying (WASD/QE, drag to orbit) remains available.
* Toolbar like WorldPainter's: rotate ◀ ▶, zoom − 100 % +, export image, move to spawn, move to origin, and
  visible layers None / Sync / All. "Sync" follows the 2D view's show/solo checkboxes.

## 10. Dialogs

* **New World** dialog with WorldPainter's fields:
  * name; size in multiples of 128, or a **circular world** of a given diameter;
  * map format; build limits (lower/upper); Minecraft seed;
  * **Topography:** Flat or Hilly, level, water level ("Minecraft default: 62"), lava instead of water, hill
    height %, horizontal hill size %;
  * beaches; surface material;
  * **Theme:** simple (surface material + beaches) or advanced (configure terrain and layers by height);
  * "edge to edge walking time" readout.

  Meadow today has a smaller dialog.
* **Add/remove tiles (Ctrl+T):** a dialog with a tile-grid selector, as in WorldPainter. Meadow's paint-tiles
  tool can stay as an extra.
* **Dimension properties (Ctrl+P):** per-dimension settings (subsurface, bedrock, water, theme, border…).
* **Change height** (rescale and shift heights), **Rotate** (90° steps), **Shift** and **Scale**. Meadow has
  rescale/resize/remap: add Rotate and Shift and match the names.
* **Global operations (Ctrl+G):** WorldPainter's fill dialog: fill with a paint / clear a layer / invert / reset
  terrain to theme, through the same filter row as §6.
* **Export:** add "Export as image" and the two height-map exports (1:1 and 1:256).

## 11. Themes (automatic terrain by height)

WorldPainter worlds have a **theme**: terrain chosen by height, plus beaches and frost. The default theme
(with f = min(max height, 320)/128, so f = 2.5 for vanilla) is:

* below water + 32 f: the surface material (normally Grass);
* from water + 32 f: **Permadirt**;
* from water + 48 f: **Stone Mix**;
* from water + 80 f: **Deep Snow**;
* **Frost** from water + 64 f upward;
* **Beaches** from water − 2 to water + 1.

The theme is used by:
* the New World dialog;
* height map imports;
* "Apply theme" on Height / Flatten / Smooth / Raise Mountain;
* right-click with a terrain paint;
* Global operations › Reset terrain.

Meadow has no themes. This is needed for the WorldPainter "raise land and it gets rock and snow" feel.

## 12. Priorities

1. **Controls** (§1, §2): mouse buttons, wheel, number keys for intensity, Alt+letter tools, R/L light,
   Ctrl zoom keys. Small change, biggest effect on muscle memory.
2. **Map look** (§8): hill shading formula, flat water, contours on, surface-block colours with speckle,
   forest stamps, frost, grid with labels, light-cyan background, dashed brush outline, power-of-two zoom,
   automatic Minecraft jar colours.
3. **Paint model and tools** (§4, §5, §6): Spray Paint / Pencil / Fill / Height / Raise Mountain / pyramids /
   eyedropper; brush grid and formulas; filter row; themes (§11).
4. **Layout** (§3, §7): dock sides and tabs, toolbar, status bar wording, Info panel, terrain grid order,
   biome grid, annotations panel, solo checkboxes.
5. **Dialogs and 3D** (§9, §10): New World, tiles dialog, dimension properties, rotate/shift, global
   operations, exports, 3D keys and isometric mode.
6. **Later:** Text tool, selection tools (edit/copy/clear), custom brush folders, image overlays,
   ceiling/master dimensions.

## Decisions to confirm

* **Zoom:** WorldPainter only zooms in powers of two. Match it exactly (recommended), or keep smooth zoom as
  an option?
* **3D view:** keep it docked as a tab with Ctrl+3 (recommended), or a separate window like WorldPainter?
* **Shift + drag to erase** (Meadow today) goes away in favour of right-click / Alt; keep it as an alias?
