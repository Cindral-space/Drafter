# Shared measurements box

The bottom-right status field belongs to one shared system. Its caption is left of the field. Hover the viewport and type; clicking the field is optional. Other focused text editors keep their input. Right-click the caption to choose document units, decimal places and decimal comma. Preferences are saved with the document; old documents default to meters, three decimals and decimal point.

## Accepted syntax

The shared `cindral_tools::measurements` parser retains explicit units. Core geometry uses meters. A bare length uses document units; explicit units override them for that entry. Accepted metric suffixes: `m`, `cm`, `mm`; imperial: `ft`, `in`, `'`, `"`. Examples: `2m`, `150cm`, `25mm`, `2.5`, `3'`, `6"`, `3'6"`, `3'-6"`, `1/2"`, `3 1/2"`, `-3'6"`. Fractions require an imperial unit. Signed lengths flip direction where the adapter supports it; radii and resize reference lengths must be positive.

With decimal point, comma separates quantities: `2m,3m`. With decimal comma, semicolon separates them: `2,5m;3m`. Mixing decimal conventions or units inside one quantity is rejected. Rectangle also accepts an omitted final dimension (`4m,`); this is equivalent to entering only `4m`. Coordinates require three entries. Input is bounded to 4096 bytes; side/copy counts are bounded to 256, with curves requiring at least three sides.

| Tool | Caption and accepted entry |
| --- | --- |
| Line | Length. Signed length; `[x,y,z]` absolute coordinates or `<x,y,z>` displacement from the original start, using document/explicit units. |
| Rectangle | Dimensions. Width and height in the drawing plane; one quantity changes width and preserves the live/completed height. |
| Circle / Polygon | Radius. Positive length; `12s` sets sides before placement, during drawing or after completion. Bare whole counts are also accepted before the center. |
| Center-based Arc | Radius before the start point, Angle after it. Positive explicit length changes radius; bare numbers after the start are degrees. `90deg`, `-90`, radians (`1rad`), and `12s` segments. Completed radius/angle/segment edits replace the same arc. This tool has no chord/bulge placement mode. |
| Push/Pull / Offset | Distance. Signed length. |
| Move / Copy | Distance. Signed length. `3x` or `*3` creates three copies at one, two and three times the established displacement; `/3` creates three copies including the original destination, dividing the distance into thirds. Retyping replaces the entire array. Arrays require an established nonzero displacement. |
| Rotate | Angle. Degrees, explicit `deg`/`°` or `rad`; `8:12` converts a slope ratio to degrees. Negative angles reverse rotation. |
| Scale | Scale. Bare number is a factor; an explicit length sets the longest original bounding dimension for uniform scaling. Two/three entries address local X/Y/Z factors or target extents, leaving omitted axes at factor one. Zero and zero-extent target axes are rejected; negative factors reflect. Legacy `3x` and `150%` factors remain accepted. |
| Tape Measure | Distance. Measure two points, then enter a positive target length. A modal asks before resizing all loose geometry and child containers in the active editing context, or the selected container occurrence. Cancel preserves model and text. A confirmed re-entry replaces the previous resize as one undo step. |
| Select / Eraser / Paint / idle | No numeric entry. Selection remains independent of the field. Follow Me retains its existing signed distance entry through the same parser. |

Container Move/Copy/Rotate/Scale use the same parser, units, captions and history rules. Selected group copies retain private definitions; component copies retain sharing. Numeric scale axes follow the active context. This work adds numerical axis entry to the existing scale interaction; it does not add new graphical grips.

## State and history

`Empty → Live → Typing → Applied / Completed`. Live formatting is cached by numerical values and settings; unchanged frames do no new measurements formatting or allocation. Display uses the selected precision and prefixes rounded values with `~`. Angles and scale factors use the same decimal preference. Typing hides the live display without destroying it.

Backspace edits. Enter parses once, validates the adapter's supported form, and applies an absolute command. Invalid syntax or geometry keeps input and model, exposes a status error, and highlights the field. Escape first clears typed text; Escape again cancels the pending operation and its eligibility for remeasurement. Successful application clears typing. `Applied` also covers setting sides or setting an Arc start radius without completing geometry.

`Completed` retains the original operation anchors/targets, final values and undo slot. Additional typed entries replace that operation until a new placement click, tool change, context replacement or undo/redo. Pointer movement alone does not end eligibility. Line keeps its next-segment continuation anchor, while retyping changes its most recent segment. Invalid replacement restores the preceding completed geometry and preserves history.

Every geometry application uses core/document transactions. Completed replacements union removed/new dirty entities and replace the last history delta. Container replacements replace their graph transaction. Tape resizing batches root geometry and child placements as one transaction. The confirmation is required for every Tape resize, including re-entry.

## Questions for the user — verify in SketchUp

1. Choose center-based Arc, click its center and type `2m` Enter: does that establish its radius/start point? Then establish a start and type `3m`: does it resize the radius while preserving sweep?
2. Finish a center-based Arc using `-90` Enter, then type `2m` Enter: does it retain the clockwise sweep?
3. Choose Circle without clicking and type `12` Enter: does it set twelve sides? Repeat with `12s` and Polygon.
4. Copy an object 2m, then type `3x` Enter: are there three copies in addition to the original? Replace with `/3`: does the final destination remain occupied, with two intermediate copies?
5. Select a 2m × 3m × 4m box, use a uniform Scale grip, and type `8m`: which dimension becomes 8m? Cindral currently uses the longest original dimension.
6. With a non-uniform Scale grip active, type `2,3,4` (or locale equivalent): which axes change and in what order? Cindral accepts active-context X/Y/Z.
7. Select one occurrence of a shared component, measure its 2m edge with Tape and type `4m`: does confirmation resize only that occurrence or every occurrence of its definition? Cindral resizes the selected occurrence; inside an editing context, shared definition geometry changes together.
8. Finish a rectangle, type a new dimension without Enter, press Escape once, then type another dimension: does the finished rectangle remain eligible for remeasurement? Cindral retains it until the second Escape.
9. Draw a rectangle, then type only `4m` Enter: does width change while height stays fixed? Repeat with `4m,` to compare omission syntax.

## Side-by-side manual script (10 steps)

Use a fresh file for each step, with meters, three decimals and decimal point. Create the trial geometry named in the step. Choose center-based Arc explicitly. Use the Cindral palette if shortcuts differ. These steps check behavior; uncertain reference details are the questions above.

1. Activate Line, click a start, move along red, type `2m` Enter, then `3'6"` Enter. Confirm the same segment changes and there is one undo step. Type `-2m` twice and check that repeated entries keep the reversed direction.
2. Start a fresh Line and enter `[2m,3m,0m]`; start another at a non-origin point and enter `<1m,0m,0m>`. Compare absolute endpoints and relative displacement. Undo each once.
3. Draw Rectangle by entering `2m,3m`, then re-enter `4m` and `2m,3m`. Confirm height preservation and one completed rectangle. Undo/redo once.
4. Choose Circle, type `12s`, place a center, move and enter `2m`, then `1m`, then `24s`. Confirm one circle with updated radius/segments. Repeat Polygon with `6s` and two radii.
5. Choose center-based Arc, place center, enter `2m`, move the end direction and enter `90deg`. Re-enter `-90`, `3m` and `12s`. Record the Arc questions above. Undo the arc once.
6. In a fresh file draw a `2m,3m` rectangle, then Push/Pull it using `1.5` Enter. Re-enter `2m`, then `1.5m`. Move-copy the resulting solid 12m along red, enter `3x`, then `/3` (the copies stay separated). Undo all three operations to empty geometry, then redo all three and compare the result.
7. Create a 2m × 3m × 4m box, select it, start Rotate, type `8:12`, then `-90`. Start Scale, type `2`, `8m`, then per-axis values appropriate to a non-flat box. Confirm repeated entries create one undo step per completed operation. Repeat on a selected group.
8. Draw a known 2m edge and measure it with Tape, type twice its length and inspect the confirmation. Cancel first and check that nothing changed. Repeat and confirm, then re-enter another length and confirm. Undo once to the original size; redo once. Repeat with one selected component occurrence and record sharing behavior.
9. During an operation type an invalid value such as `2mfoo` and press Enter. Check preserved text/model, error and highlight. Backspace/edit to a valid value and apply. Type another value, Escape once to clear text, Escape again to cancel. Rename an Outliner item or scene while a tool is active; verify its text is not captured by measurements.
10. Right-click Cindral's measurement caption, select centimeters/two decimals, then decimal comma. Draw Rectangle with `2,5m;150cm`, retype `300cm`, and inspect live rounding (`~` on inexact displays). Save/reopen to confirm preferences persist. Repeat using the matching locale/unit settings in SketchUp.

## Validation limits

Automated checks cover parsing/Unicode totality, cached state, focus routing, tool adapters, confirmation/cancel, replacement and exact undo/redo. Existing unrelated ignored audit gaps remain, including rejection of overlapping coplanar loose copies and construction guides outside document undo history. No SketchUp binary, vendor code/assets or SDK was used. No native window/GPU or full presented-frame 16 ms certification is claimed; unchanged measurements formatting has an explicit cached fast path. Numerical Scale does not introduce graphical grip selection. Large whole-context Tape resizing is intentionally a confirmed operation and can be expensive.
