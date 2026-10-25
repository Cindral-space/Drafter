# Pan Tool UX Review

## 1. Current Tool & Navigation Control Locations

| Control / Tool | Left Tools Panel (`tool_palette`) | Header Action Row (`workspace_header`) | Top Menus (`workspace_header`) | Inspector / Other UI | Keyboard / Pointer | Drawn / Routed By |
| --- | --- | --- | --- | --- | --- | --- |
| **Select, Eraser** | Yes (top row, 56×46 icon+label tile) | Active label only (`active_tool_label`) | `Tools` → *Selection* | — | `Space`, `E` | `crates/cindral-app/src/workspace_ui.rs`: `DrafterApp::tool_palette`, `tool_button`, `paint_tool_icon`, `DrafterApp::workspace_header` |
| **Line, Rectangle, Circle, Arc, Polygon** | Yes (rows 2–4, 56×46 icon+label tiles) | Active label + contextual `Work plane` / `Segments` (`header_tool_options`) | `Draw` menu | — | `L`, `R`, `C`, `A`, `N` | `crates/cindral-app/src/workspace_ui.rs`: `DrafterApp::tool_palette`, `DrafterApp::workspace_header`, `DrafterApp::header_tool_options` |
| **Push/Pull, Move, Copy, Rotate, Scale, Offset, Follow Me** | Yes (rows 4–7, 56×46 icon+label tiles) | Active label only | `Tools` → *Modeling* | — | `P`, `M`, unbound, `Q`, `S`, `F`, unbound | `crates/cindral-app/src/workspace_ui.rs`: `DrafterApp::tool_palette`, `DrafterApp::workspace_header` |
| **Measure (Tape Measure), Paint (Paint Bucket)** | Yes (rows 7–8, 56×46 icon+label tiles) | Active label + Paint swatch/picker/help/Eyedropper (`header_tool_options`) | `Tools` → *Utilities* | Materials tab (`materials_panel`) | `T`, `B` (`Shift+B` picker) | `crates/cindral-app/src/workspace_ui.rs`: `DrafterApp::tool_palette`, `DrafterApp::workspace_header`, `DrafterApp::header_tool_options` |
| **Section Plane** | No | No | No | `Appearance` tab → `Section planes` (`Place`) | Unbound | `crates/cindral-app/src/section_ui.rs`: `DrafterApp::section_panel` |
| **Pan** | **No** | **Yes** (`selectable_label("Pan")` after `Fit model` + active tool label) | `Camera` → `Pan` | — | `H`, or temporary `Shift+MMB` drag, or scroll while active | `crates/cindral-app/src/workspace_ui.rs`: `DrafterApp::workspace_header`; `crates/cindral-app/src/pan_ui.rs`: `DrafterApp::activate_pan`, `DrafterApp::pan_input` |
| **Orbit** | **No** | Active label only (when triggered via `O`) | **No** (`Help` text only) | — | `O` (modal primary-drag) or `MMB` drag | `crates/cindral-app/src/shortcuts.rs`: `DrafterApp::route_shortcuts`; `crates/cindral-app/src/lib.rs`: `DrafterApp::viewport_input` |
| **Zoom** | **No** | Active label only (when triggered via `Z`) | **No** (`Help` text only) | — | `Z` (modal primary-drag) or mouse wheel | `crates/cindral-app/src/shortcuts.rs`: `DrafterApp::route_shortcuts`; `crates/cindral-app/src/lib.rs`: `DrafterApp::viewport_input` |
| **Fit model (Zoom Extents)** | No | Yes (`Button::new("Fit model").frame(false)`) | `Camera` → `Fit model` | — | `Shift+Z` | `crates/cindral-app/src/workspace_ui.rs`: `DrafterApp::workspace_header`; `crates/cindral-app/src/lib.rs`: `DrafterApp::extents` |
| **Standard views** (Iso, Top, Bottom, Front, Back, Left, Right) | No | No | `Camera` → *Standard views* | — | `0` (Iso), `1` (Front), `3` (Right), `7` (Top); Bottom/Back/Left have no key | `crates/cindral-app/src/workspace_ui.rs`: `DrafterApp::workspace_header`; `crates/cindral-app/src/shortcuts.rs`: `DrafterApp::route_shortcuts` |
| **Projection toggle** (Perspective / Parallel) | No | No | `Camera` → *Projection* (two radio items) | — | `5` toggles (`Action::Projection`) | `crates/cindral-app/src/workspace_ui.rs`: `DrafterApp::workspace_header`; `crates/cindral-app/src/lib.rs`: `toggle_projection` |

---

## 2. Evaluation of Current Pan Placement

- **Grouping consistency & discoverability:** Poor. Every other modal canvas tool (16 Draw/Modify tools) lives in the left `tool_palette` as a 56×46 vector-icon tile. A user looking for a persistent hand/pan tool checks the left tool palette first. In the top sub-header row (`Undo | Redo | Fit model | [Pan] <Active Tool Label> <Tool Options> ... Inspector | Save`), `Undo`, `Redo`, and `Fit model` are one-shot actions (`egui::Button::frame(false)`), whereas `Pan` is a persistent modal tool rendered as a text-only `selectable_label` immediately adjacent to the active-tool readout (`workspace_ui.rs:427-446`). When Pan is active, the bar reads `Pan Pan` side by side (`selectable_label("Pan")` followed by `active_tool_label()`).
- **Orphan status:** Semi-orphan. `Fit model` sits next to `Pan` across a separator, so there is a faint camera adjacency, but `Orbit` (`O`) and `Zoom` (`Z`) have no buttons anywhere in the UI (neither in the header row, nor in `tool_palette`, nor even in the `Camera` menu).
- **Active-tool highlight (buggy dual highlight):** `Pan` in the header row highlights via `selectable_label(self.shortcuts.navigation == Some(Action::Pan), "Pan")`, using `selection.bg_fill`. However, `activate_pan()` (`pan_ui.rs:13-17`) calls `self.choose_modify(ModifyKind::Select)` before setting `self.shortcuts.navigation = Some(Action::Pan)`, and `tool_palette` (`workspace_ui.rs:688`) checks `self.modifying && self.modify.kind() == kind` **without** checking `self.shortcuts.navigation.is_none()`. As a result, **`Select` in the left tools panel (and in the `Tools` menu) remains highlighted as active at the same time as `Pan`**. Additionally, `Camera → Pan` in the menu bar (`workspace_ui.rs:173`) does not set `.selected(...)`.
- **Tooltip:** Works and shows the `H` shortcut. `shortcuts::tooltip` formats `Action::Pan` (`Entry { label: "Pan", bindings: [(H, ..., "H")] }`) as **`"Pan (H)"`** (`workspace_ui.rs:432-437`, `shortcuts.rs:163, 341-345`). Note that `Camera → Pan` in the menu uses `.shortcut_text("H")`, which also displays `H`.
- **Cursor:** Uses the system grab cursors (`egui::CursorIcon::Grab` when hovering in Pan mode, `egui::CursorIcon::Grabbing` during an active pan drag or `Shift+MMB` drag; `workspace_ui.rs:585-592`, `pan_ui.rs:101, 109`). Unlike the 16 Draw/Modify tools, `paint_tool_icon` has no `"Pan"` vector symbol (`workspace_ui.rs:1051-1192`), which is fine for system `Grab`/`Grabbing` cursors on the canvas but means Pan lacks an icon for `tool_button`.
- **Status-bar hint:** Shows `Pan` in accent color followed by `"Drag to pan the view."` (`workspace_ui.rs:952, 972-973`). Unlike `Orbit`/`Zoom` (`"Drag to navigate · Escape returns to Select"`), `Escape` in Pan mode only aborts an in-progress drag (`pan_ui.rs:55-58`) and does not exit Pan mode back to `Select`.
- **Light/dark themes and 1366×768:** Works in both Light and Dark themes (`configure_style` defines high-contrast `selection.bg_fill` and `selection.stroke` for both themes in `workspace_ui.rs:18-52`) and remains visible and unclipped at `1366×768` (verified down to `720×480` in `header_tests::header_keeps_actions_visible_and_options_contextual`, and at `1366×768` the `compact < 900.` wrap is not needed).

---

## 3. Recommendation

**Recommendation: (C) Put `Orbit`, `Pan`, and `Zoom` (plus `Fit`) together in a dedicated Navigation group at the bottom of the left tools panel (`tool_palette`), and remove the text-only `Pan` toggle from the header row.**

### Why this option fits Cindral Drafter best
1. **Matches Drafter's modal tool model:** In Drafter, `Orbit` (`O`), `Pan` (`H`), and `Zoom` (`Z`) are all modal states stored in `self.shortcuts.navigation: Option<Action>` that take over primary-button canvas drags and show as the active tool in the header and status bar (`active_tool_label()`). Grouping them in `tool_palette` puts every modal canvas tool in one predictable place.
2. **Fixes the `Pan Pan` header duplication and frees horizontal space:** Removing the modal `Pan` toggle from the header row leaves that row cleanly divided into one-shot bar actions (`Undo`, `Redo`, `Fit model`), the active tool readout + contextual tool options (`Work plane` / `Segments` / Paint controls), and right-aligned workspace toggles (`Inspector`, `Save`).
3. **Fits 1366×768 without scrolling:** The left `tool_palette` currently has 16 buttons (8 rows × 49 px = ~392 px tall). Adding a separator and 2 rows of 2 buttons (`Orbit`, `Pan`, `Zoom`, `Fit`) adds ~106 px (~500 px total), which fits comfortably inside the ~690 px vertical canvas height available at `1366×768` without scrolling.

### Proposed Layout Sketch

```text
+---------------------------------------------------------------------------------+
| File  Edit  Camera  Draw  Tools  View  Window  Help        Modified  Untitled   |
+---------------------------------------------------------------------------------+
| Undo  Redo | Fit model | <Active Tool>  [Contextual Tool Options...]  Inspector Save |
+------------+--------------------------------------------------------------------+
| [Select] [Eraser] |                                                             |
| [Line]   [Rect]   |                                                             |
| [Circle] [Arc]    |                                                             |
| [Poly]   [Push/P] |                                                             |
| [Move]   [Copy]   |                       3D Viewport                           |
| [Rotate] [Scale]  |                                                             |
| [Offset] [Follow] |                                                             |
| [Meas.]  [Paint]  |                                                             |
| ----------------- |                                                             |
| [Orbit]  [Pan]    |                                                             |
| [Zoom]   [Fit]    |                                                             |
+-------------------+-------------------------------------------------------------+
```

### Required Code Changes & Effort
- **`crates/cindral-app/src/workspace_ui.rs`**:
  - In `tool_palette`: clear `Select`/modify button selection when `self.shortcuts.navigation.is_some()`; add a separator and 2×2 navigation grid (`Orbit`, `Pan`, `Zoom`, `Fit`); add `"Orbit"`, `"Pan"`, `"Zoom"`, `"Fit model"` vector icon arms to `paint_tool_icon`.
  - In `workspace_header`: remove the `selectable_label(..., "Pan")` next to `Fit model`; add `Orbit` and `Zoom` items (and `.selected(...)` state) to the `Camera` menu.
- **`crates/cindral-app/src/pan_tests.rs` & `workspace_ui.rs` (`header_tests`)**: update/add UI test assertions for palette buttons and single active-tool highlight.
- **Rough effort:** Small (~60–90 lines of Rust across 1–2 files, ~1 hour including tests).

---

## 4. Related Problems Noticed

1. **Dual active-tool highlight (`Select` + `Pan`/`Orbit`/`Zoom`):** `activate_pan()` (`pan_ui.rs:14`) and `Orbit`/`Zoom` shortcut routing (`shortcuts.rs:441`) call `self.choose_modify(ModifyKind::Select)` and then set `self.shortcuts.navigation = Some(...)`. Because `tool_palette` (`workspace_ui.rs:688`) and the `Tools` menu (`workspace_ui.rs:266`) check `self.modifying && self.modify.kind() == kind` without checking `self.shortcuts.navigation.is_none()`, the `Select` tool button stays highlighted while `Pan`, `Orbit`, or `Zoom` is active.
2. **`Orbit` (`O`) and `Zoom` (`Z`) are invisible in the UI:** Both exist as full modal navigation tools in `shortcuts::MAP` (`shortcuts.rs:162, 164`) and `viewport_input` (`lib.rs:494-515`), but neither appears in the left tools panel, the header row, or even the `Camera` dropdown menu (`workspace_ui.rs:169-231`). A mouse- or tablet-driven user cannot discover or activate modal Orbit/Zoom without pressing `O` or `Z` or reading `F1`.
3. **Inconsistent `Escape` behavior and status hint between `Pan` and `Orbit`/`Zoom`:** When `Orbit` or `Zoom` is active, the status bar says `"Drag to navigate · Escape returns to Select"` and pressing `Escape` exits navigation mode (`lib.rs:508-513`). When `Pan` is active, `pan_input` (`pan_ui.rs:42-58`) intercepts `Escape` to cancel only an in-progress drag and returns `true` before `viewport_input` can clear `self.shortcuts.navigation`, so `Escape` never exits persistent `Pan` mode back to `Select` (only `Space` or clicking another tool does), and the status hint (`"Drag to pan the view."`) omits how to exit.
