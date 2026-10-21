# Modeling workspace review

Reviewed and updated on 2026-10-21. The reference is SketchUp's desktop workspace organization: a compact tool palette, a right-hand tray, a large drawing area, and measurements at the lower right. Cindral retains its own name and original vector tool symbols.

## Findings and changes

| Finding | Change |
| --- | --- |
| Two left panels plus a right panel squeeze the canvas to about 657 px at 1280 px window width. | A narrow tool rail and a resizable, hideable inspector leave most of the window available for modeling. |
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

## Modern workspace redesign

The second pass replaces the caption grid with a 64 px dark tool rail. Original vector symbols use consistent strokes; hover text and accessible names identify every tool, and the current tool appears in the contextual toolbar. Copy, Scale, Offset and Follow Me have distinct symbols. The document header consolidates menus and exposes Save and Inspector directly. Drawing plane and segment controls appear only while drawing.

The inspector separates Model and Appearance into tabs, with rounded cards for selection and document information. Empty Outliner content explains what belongs there, and grouping actions are disabled until geometry is selected. File identity moves to the header, allowing a single status row in ordinary use. The new default environment and grid use neutral cool grays; loaded document styles still take precedence.

File shortcuts and startup recovery discovery run independently of menu visibility. A regression test saves a modified document through Ctrl+S with the application menu closed and verifies the file contents.

Validation captures: `.agent-out/ui-review/modern.png` (1280×800) and `modern-compact.png` (720×480). At compact heights the tool rail and inspector scroll to keep all controls accessible. Workspace tests, strict Clippy, formatting and diff checks are run for the redesign.

## Simplification pass

The next pass removes decorative inspector cards, the repeated Inspector heading, the empty selection card and document counters. Model and Appearance remain directly accessible; selection details appear only when relevant, and an empty Outliner starts collapsed. The header has one row during selection and modification, with drawing controls added only when needed. Save and Inspector use quiet text controls.

The primary rail contains common modeling tools; the overflow menu retains every drawing and modification action. The construction grid is off by default and can be enabled under View. World axes remain visible, and toggling the grid changes only the submitted line range, without rebuilding GPU buffers or changing document geometry.

Validation: native OpenGL captures at 1280×800 and 720×480, app and renderer tests, strict Clippy for both affected crates, formatting and diff checks. Captures are saved as `.agent-out/ui-review/clean.png` and `clean-compact.png`.

## Tool palette preference

Restored all 16 tools in the familiar two-column arrangement with visible captions, following user feedback about SketchUp familiarity. There is no overflow menu. The clean single-row header, flat inspector and optional construction grid remain. The palette scrolls at compact heights.

Validation: native captures at 1280×800 and 720×480, strict app Clippy, formatting and diff checks. Updated captures: `.agent-out/ui-review/full-tools.png` and `full-tools-compact.png`.

## Header preference

Restored the visible File, Edit, Camera, Draw, Tools, Window and Help menus plus the familiar toolbar, following feedback to preserve the earlier organization. Styling uses consistent menu spacing, quiet Undo/Redo and Fit model actions, a softly grouped XY/XZ/YZ plane selector, subtle separation between rows, and a restrained brand/document area. Inspector and Save remain directly accessible. The Camera menu retains the construction-grid toggle. Tool options, file shortcuts and startup recovery continue to use the existing handlers.

Validation: strict app Clippy, formatting and diff checks, the save-shortcut regression test, and native desktop/compact captures saved as `.agent-out/ui-review/header.png` and `header-compact.png`.

## Restored layout polish

Kept the visible menus, toolbar and all 16 tools in two columns. The palette now uses a light surface, darker vector strokes and 11 px captions in larger cells, with a soft violet active state. The header emphasizes the document title and exposes Unsaved, Modified or Saved state at both desktop and compact widths.

The inspector has a compact document summary and selection prompt. Selecting a group/component reveals the Outliner, while Appearance opens with style controls expanded. World-axis colors are subdued and a camera-aligned XYZ indicator sits in the canvas corner.

Save-state checks compare file contents against the last explicit save/open baseline only when the revision changes, and defer comparisons during active drags. Recovery autosaves remain separate from explicit save state. A regression test covers save, presentation edits, restoration, geometry edits, recovery autosave and reopen.

Validation: native desktop/compact captures, app and renderer tests, strict Clippy for affected crates, formatting and diff checks. Captures: `.agent-out/ui-review/polished.png` and `polished-compact.png`.

## Recovery dismissal

The recovery dialog uses a title-bar × to dismiss the prompt while preserving snapshots. Each snapshot has Recover and a compact × action, with a tooltip identifying permanent discard. The File menu uses the same compact discard control. Discarding the final snapshot dismisses the empty prompt. Validation uses isolated temporary preview entries, strict Clippy, formatting and diff checks.

## Contextual header menu review

The header retains File, Edit, Camera, Draw, Tools, Window and Help. Menu shortcuts now use a separate aligned column instead of embedded spaces. File separates document actions from import, groups export formats in a submenu, and exposes the existing Save As shortcut. Camera uses the same “Fit model” wording as the toolbar.

Save and Inspector occupy reserved space at the right of the toolbar; the active tool appears beside Undo, Redo and Fit model. Work-plane controls appear while drawing, segment counts appear for curves/polygons, and Paint labels its color control. At widths below 924 px (900 px inside header margins), contextual controls use a separate row so they cannot displace the persistent actions. Selecting the current work plane no longer cancels an in-progress drawing. Document names truncate into available space and retain their full tooltip.

Validation: native 1280×800 and 720×480 captures; a headless layout regression checks menu/action visibility, text clipping and contextual plane/segment controls at 720, 900, 960 and 1280 px; app library tests; strict app Clippy; formatting and diff checks. Review captures: `.agent-out/ui-review/menu-after.png` and `menu-compact.png`. Existing tool-palette sizing edits are preserved.

## Dropdown panels

Dropdowns now share 30 px rows, restrained hover feedback, white rounded surfaces, and consistent minimum widths. Active Draw/Tools commands use the selection tint. Tools separates Selection, Modeling and Utilities; Camera separates projection, standard views and display controls. Help groups navigation and modeling instructions. Recovery snapshots move into a submenu with truncated filenames, full-path tooltips and a bounded scrolling list, so File stays compact as snapshots accumulate.

Validation: native compact captures of File and Tools (`.agent-out/ui-review/dropdown-file.png` and `dropdown-tools.png`), plus headless input coverage for compact dropdown visibility, nested exports, choosing Circle, Escape dismissal, and clicking outside a submenu. App library tests, strict Clippy and formatting checks pass.
