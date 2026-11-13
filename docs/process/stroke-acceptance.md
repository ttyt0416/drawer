# Item 287: deterministic basic-stroke acceptance

`Drawer.exe --verify-item 287 NEW_DIRECTORY` runs the production editor/input pipeline in a CPU-only egui context. It is owned synthetic mouse input, not physical pen evidence. `scripts/test-checklist.ps1 -Item 287` checks the packaged executable hash, runs this entry point hidden, validates its versioned receipt, binds the executable/receipt hashes in local evidence and deletes only its owned temporary directory after process exit.

The matrix contains seven basic tools × five cases: a single zero-distance click, a short line, a long line, the same long events with a single timestamp/bounded batching, and 25 separate rapid dots. Every case is replayed twice with exact RGBA comparison; long/fast results must match. The replay engine compares complete semantic Undo/Redo states and counts, resets the final contact and checks that no incomplete stroke remains. Paint/eraser paths check changed pixels in every interior footprint strip including endpoints. Half-alpha/taper/cursor checks reuse their independent expected-alpha assertions.

Eraser starts on opaque white. Smudge moving cases start on an owned blue/red boundary so real pigment transport can be observed; a stationary smudge or isolated dots on a uniform field intentionally add no pigment/history. Other tools start transparent. Seed descriptions and complete versioned input records travel with the acceptance receipt; the verification-only seeded path does not change the public replay record format or ordinary editor replay behavior.

Degenerate cases separately assert: a zero-length vector line renders the expected half-alpha dot and survives native reopen; a 1×32768 image rotates with exact endpoint pixels and Undo; a 1e-9 document-space vector translation undoes to the original geometry; and flattening the full 256-layer stack includes the known last-layer pixel and restores names/IDs/structure in one Undo.

All data is independently generated in the owned verification directory. No original product, external asset, physical mouse automation or computer-use tool is involved.
