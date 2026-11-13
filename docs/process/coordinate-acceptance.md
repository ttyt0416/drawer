# Item 288: cursor-to-ink coordinate acceptance

Run `scripts/test-checklist.ps1 -Item 288` against the packaged application. The same scenario is a release regression test.

The matrix uses five native egui DPI scales (1, 1.25, 1.5, 2, 2.5), five zooms (0.125, 1/3, 1, 1.375, 8), four rotations (0, 37, 90, 215 degrees), both horizontal flip states and a fractional pan. Three independently known document points per combination give 200 view combinations and 600 first presses.

An independent forward equation constructs f32 logical screen events from the known document points. It does not call the production forward/inverse pair. The real editor processes mouse Down/Up. The test reads the first raw document point from the production pending stroke, checks radial error ≤0.5 document pixels, verifies preview ink and the committed expected pixel, then compares complete semantic Undo/Redo states. It also verifies that the native DPI scale was applied to the egui context. Local evidence retains each expected/screen/actual point and the maximum measured error, executable hash and receipt hash.

This checks the application event-to-document pipeline. The physical OS cursor, real mixed-DPI monitor transitions and tablet driver mappings remain separate hardware checks in item 300.
