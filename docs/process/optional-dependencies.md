# Optional dependencies and basic editing

Item 281 is checked by `local_inference::verify::optional_dependencies` in both the Rust test suite and the packaged application smoke test. The fixture uses the production editor/input pipeline in an egui CPU context without a native GPU context.

| Isolated failure | Required result |
| --- | --- |
| Missing local model, no approved registration | Review/execution refuses; no worker or partial document result |
| Explicitly absent FFmpeg/FFprobe | Decoder guidance; adoption refuses; no partial movie |
| 64 MiB workspace budget, auxiliary preview requiring at least 96 MiB | Preview refuses before creating a worker |

After **each** failure the fixture dismisses the optional dialog, sends mouse press/release through the editor, checks one brush Undo entry, creates and reopens a recovery snapshot, saves a native document and compares its reopened semantic state. Failure itself must preserve the document and history exactly. The packaged installation test also removes developer tools/codecs from PATH. No external model, GPU SDK or decoder is required for this test or the basic document format.

This verifies graceful failure of optional computation and editing without GPU computation. It does not claim that a native window can start on a computer whose operating system provides no supported graphics presentation at all, or substitute for the physical device checks in items 296–300.
