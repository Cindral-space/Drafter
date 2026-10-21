# Modeling workspace review

Reviewed and updated on 2026-10-21. The reference is SketchUp's desktop workspace organization: a compact tool palette, a right-hand tray, a large drawing area, and measurements at the lower right. Cindral retains its own name and original vector tool symbols.

## Findings and changes

| Finding | Change |
| --- | --- |
| Two left panels plus a right panel squeeze the canvas to about 657 px at 1280 px window width. | One 110 px tool palette and one resizable, hideable 280 px tray give the canvas about 888 px, approximately 35% wider. |
| Drawing tools, modification tools and camera controls lack a consistent hierarchy. | Modeling tools share a captioned icon palette; File, Edit, Camera, Draw, Tools, Window and Help menus organize other actions. |
| Every style, material and tag control appears at once. | Entity Info, Outliner, Tags, Materials and Styles & Scenes use collapsible tray sections with a shared scrollbar. |
| Dark chrome, background and grid compete with geometry. | Light chrome, a pale sky/ground environment, subdued grid and contrasting model edges establish a clearer hierarchy. Saved document styles remain authoritative when opened. |
| Long instructions can push measurements out of view. | Measurements get space first at the lower right; the contextual prompt truncates into the remaining width. Help holds the longer navigation instructions. |
| Numeric view shortcuts and scroll zoom only run in drawing mode. | Shared viewport input handles navigation in both drawing and modification modes. Drawing shortcuts work from Select; Space returns to Select. Text-field focus and typed dimensions retain priority. |
| A new window starts with a drawing tool and can create geometry unintentionally. | The native editor starts with Select active. |
| Empty scene tabs consume a full row. | Scene tabs appear after a scene exists; the tray creates the first scene. |
| White inference and measurement labels disappear against a light background. | Viewport label ink adapts to the document background. |
| An egui layout retry can consume a pending GPU upload before rendering it. | Preserve uploads through discarded layout passes. Native capture confirms that geometry survives initial layout. |

## Validation

- Native OpenGL captures of the previous and revised 1280×800 workspace, plus the revised 720×480 minimum size.
- Headless input regression covers zoom, switching from Select to Rectangle, and returning to Select with Space.
- Workspace formatting, strict Clippy and workspace tests.

This update uses the existing modeling engine. Orbit and pan remain middle-mouse gestures; dedicated drag-based Orbit/Pan tools are a possible later improvement. Tool icons include captions and hover labels; some symbols can be refined further as the interface evolves.

Reference: [SketchUp: Managing dialog boxes in Windows](https://help.sketchup.com/en/sketchup/managing-dialog-boxes-windows).
