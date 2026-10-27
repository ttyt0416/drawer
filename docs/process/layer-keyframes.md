# 199 — Layer keyframes, track names and labels

The command `layer-keyframes` opens a local editor for raster/vector/folder, object, gradient, world, cel-track/cel owners and an explicit 2D parent camera. Existing `timeline-editor` naming and label controls remain available. The new editor also renames cel tracks and adds, renames, moves and deletes saved labels.

## Data and evaluation

Native98 stores `Scene.keyframes`: unique stable owners, enabled flag, parent-camera link, captured base matrix/opacity, a static hold pose and ordered keys. Original pixels, high precision storage, paths, masks and typed cel definitions remain intact. Each pose has relative x/y translation, positive x/y scale, rotation in degrees, pivot and relative opacity. Initial activation captures the existing baked transform as its basis and inserts an identity-relative key at the **current** frame. It does not apply the baked transform twice or insert frame zero. Disabled owners retain their keys and freeze their current local pose. Reactivation resumes the original times/values without inserting a key. The parent camera remains a parent: disabling a child freezes the child's local pose, while its camera may still move.

Keys hold all pose properties together. The outgoing interpolation is fixed, linear or cubic Bezier. Outside the keyed interval evaluation clamps to its endpoint. Fixed segments change exactly on the next key. Bezier x is inverted by48 bounded bisections; both handles use normalized0..1 coordinates and x1<=x2, so time cannot reverse and interpolated positive scale/opacity cannot overshoot. Rotation uses stored continuous degrees (including multiple turns). Adding at an occupied time requires an explicit Replace choice; moving a key across a neighbour or outside the timeline is rejected.

The base source is already stored in its static document coordinates. Animation overlays that source; base opacity is multiplied by relative opacity. Scale range is0.01..1000, offsets/pivots are finite within1e9, rotation within36000 degrees, relative opacity0..1. Limits:1024 owners,10000 total keys,10000 timeline frames. This camera is a2D parent transform, separate from the3D scene camera.

## Content editing, Undo and display

Content editing is a view/session mode on Document, excluded from native data, dirty state and Undo. It shows all layers in their original base coordinates and permits edits of the selected owner or its descendants. Source changes to other animated owners are guarded. Exports always evaluate animation even while this mode is active. A visible canvas banner explains the current view. Preview caches include the view mode. Enable/disable, parent link, key edits, complete deletion and each content edit are independent document transactions. Deleting all keys requires confirmation of the displayed owner, count and complete first/last range; its evaluated local pose remains static and Undo restores all keys. Locked owner/parent properties reject key and content edits.

Rendering maps samples through inverse matrices without rewriting source data; masks, vector display caches and raster surface effects use base coordinates. Camera links on parent/child apply once. Folder/track parents compose with local source transforms. Selection bounds and source inspection use evaluated coordinates. Normal hidden/opacity/blend rules still apply.

## Time and lifecycle integration

FPS conversion includes retained keys on disabled owners, uses the same rational half-up rule as198 and requires a choice for every per-owner collision. Time insertion/deletion includes all owners with All, selected cel-track/cel owners through track selection, additional layer-owner checkboxes and the global camera through Globals. It rejects locked selected owners. Removed interval keys appear in the removal table; final key removal freezes a pose. Key edits and related timing changes undo atomically. Track/layer duplication remaps owner IDs; deletion prunes missing owners. Previous/next-key navigation includes the selected track/cels and global layer/camera keys. Old headers cannot disguise native98 layer animation.

## Verification

Core verification exercises mathematical interpolation/inversion, actual composition, activation at a nonzero frame, original pixel preservation, hold/resume, collisions/reversed time/invalid handles, independent base content Undo, export isolation, parent camera composition, complete deletion Undo, lock rejection, native reopen/old-header rejection and selected FPS/time edits. The app verifier dispatches the command and uses pointer events for activation, insertion and base editing. It is also called by the installed release smoke test.
