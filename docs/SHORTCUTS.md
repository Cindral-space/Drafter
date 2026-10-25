# Keyboard shortcuts

The action registry in `crates/cindral-app/src/shortcuts.rs` owns bindings, aliases,
labels, categories, operation eligibility and unbound reasons. Menu hints, toolbar
tooltips and searchable F1 help use that registry. The tables below and in README
are checked against it by `documentation_matches_map`; set `UPDATE_SHORTCUT_DOCS=1`
when running that test to regenerate both tables. There is no rebinding UI.

<!-- shortcuts:start -->
| Category | Action | Binding | During operation |
| --- | --- | --- | --- |
| Tools | Select | Space | Yes |
| Tools | Line | L | Yes |
| Tools | Rectangle | R | Yes |
| Tools | Circle | C | Yes |
| Tools | Arc | A | Yes |
| Tools | Polygon | N | Yes |
| Tools | Push/Pull | P | Yes |
| Tools | Move | M | Yes |
| Tools | Rotate | Q | Yes |
| Tools | Scale | S | Yes |
| Tools | Offset | F | Yes |
| Tools | Eraser | E | Yes |
| Tools | Tape Measure | T | Yes |
| Tools | Paint Bucket | B | Yes |
| Materials | Choose paint material | Shift+B | Idle only |
| Navigation | Orbit | O | Yes |
| Navigation | Pan | H | Yes |
| Navigation | Zoom | Z | Yes |
| Navigation | Fit model | Shift+Z | Yes |
| Navigation | Iso | 0 | Idle only |
| Navigation | Front | 1 | Idle only |
| Navigation | Right | 3 | Idle only |
| Navigation | Top | 7 | Idle only |
| Navigation | Toggle projection | 5 | Idle only |
| Actions | Undo | Ctrl+Z | Yes |
| Actions | Redo | Ctrl+Y / Ctrl+Shift+Z | Yes |
| Actions | Select all | Ctrl+A | Idle only |
| Actions | Erase selection | Delete | Idle only |
| Actions | Make Group | G | Idle only |
| Files | Save | Ctrl+S | Yes |
| Files | Open | Ctrl+O | Yes |
| Files | Save As… | Ctrl+Shift+S | Yes |
| Actions | Cancel | Escape | Yes |
| Help | Shortcut help | F1 | Yes |
| Constraints | Lock X | ArrowRight | Yes |
| Constraints | Lock Y | ArrowLeft | Yes |
| Constraints | Lock Z | ArrowUp | Yes |
| Constraints | Clear axis lock | ArrowDown | Yes |
| Tools | Copy | Unbound: Use the existing Copy button; M activates Move | Yes |
| Tools | Follow Me | Unbound: No conventional default | Yes |
| Tools | Section Plane | Unbound: Use the existing section panel; no conventional default | Idle only |
| Measurements | Apply measurement | Enter | Yes |
| Measurements | Edit measurement | Backspace | Yes |
<!-- shortcuts:end -->

Shortcuts use egui's logical key (the character), never `physical_key` or scan
position. Holding a key does not repeat actions. Plain tool keys reject Ctrl,
Alt, Meta and Shift; only bindings explicitly listing Shift accept it. Ctrl action
bindings also retain the platform Command alias. Keyboard activation is scoped to
the hovered canvas; file actions retain their application scope. F1 opens help;
its search matches action names and categories. Escape or the close button releases
its text focus. Shortcuts are blocked by text editor focus, modal dialogs and any
nonempty measurement entry. Enter, Backspace and Escape belong to measurement
editing; Escape first clears the entry, then cancels an operation on a second press.
Letters following a numeric entry, including m, s and x, remain measurement text.
Switching tools cancels provisional geometry, preserves selection and editing
context, and refreshes the tool cursor and status guidance. Completed operations
remain committed. Navigation keys select a primary-drag mode; Escape returns to
Select for Orbit/Zoom. Pan stays active after release or Escape; choose another tool to leave it. **Camera → Pan**, the toolbar Pan button and **H** share activation. Pan uses a hand cursor, closed while left-dragging, and says “Drag to pan the view.” Shift+middle-drag shares the same picked-depth math without cancelling the current operation. Two-axis scroll pans in Pan mode; scroll otherwise zooms. See [Pan details and reference questions](PAN_TOOL.md). Middle-drag orbit remains.
Existing held Rotate copy (Ctrl; Option on macOS), Shift snapping and selection
modifiers remain gestures rather than activation shortcuts.

Changed shortcuts: **Polygon P → N; Fit F → Shift+Z**. P now activates Push/Pull;
F activates Offset. N is a free single letter chosen for Polygon, not a claimed
reference-app default. A status notice appears once per installation/user state
version. Its separate marker in `$XDG_STATE_HOME/cindral-drafter/shortcuts-v1`
(or `~/.local/state/cindral-drafter/shortcuts-v1`) uses temporary-file write,
file sync, atomic rename and directory sync on Unix. An interrupted write cannot
corrupt theme/document settings. Failure to persist keeps the notice visible and
reports the failure. Concurrent first launches may each show the notice.

Linux X11/Wayland: no Alt+Tab, Super/Meta+letter or desktop-reserved global chord
is assigned. These are commonly intercepted by desktop environments. Shift+Z
avoids those chords. Number views are idle-only so typed lengths keep priority.
No reference application, SDK, vendor text, code, icons or assets were used.

## Questions for the user

1. [verify] With a model off-screen in real SketchUp, press **Shift+Z**: does it fit the model? Cindral uses Shift+Z.
2. [verify] With loose geometry selected, press **G**: does it create a group or open component creation? Cindral creates a group.
3. [verify] During an unfinished Line operation, press **R**: does it cancel the line and activate Rectangle? Cindral cancels the provisional operation and switches.

## Validation limits

Headless egui input tests exercise the production routing, model preview rollback,
focus, measurements, aliases, repeat rejection, selection and help closure. Existing
GPU tests remain optional when no adapter exists. No native X11/Wayland window or
reference-app runtime comparison is claimed; no performance benchmark is needed
for this input-only change. Delete uses existing geometry and occurrence deletion
transactions; mixed loose/container deletions retain those separate undo steps.
