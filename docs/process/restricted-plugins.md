# Restricted local plugins

Open **Restricted local plugins…** from the layer commands. Import a local JSON file or use the bundled inversion example, preview its output, then adopt a new raster layer as one Undo operation. Discard/Escape changes no document. A changed/closed page or frame invalidates adoption. Keep a verified copy stores immutable, content-addressed JSON in `UserStorage/plugins`; choose that file again after restart. Existing differing copies are preserved and refused.

This is a purpose-built, capability-free bytecode interpreter, not a native DLL, Python, JavaScript, WASI or general-purpose executable host. Its entire API is a frozen page's RGBA pixels and dimensions plus a separate output buffer. It receives no file handles, paths, network/process imports, callbacks, document pointers or allocator. The editor alone imports/keeps a user-selected program and adopts validated output. JSON unknown fields/opcodes/import declarations are rejected.

Format version 1: `{"version":1,"name":"Invert","instructions":[{"op":"Invert"},{"op":"End"}]}`.

| Instruction | Effect |
| --- | --- |
| `RequireSize {width,height}` | Refuse a mismatched frozen document size |
| `Set {index,rgba}` | Set one bounded output pixel (row-major index) |
| `CopyInput {from,to}` | Read a bounded frozen pixel and write output |
| `Invert` | Invert nontransparent output RGB; preserve alpha |
| `Jump {instruction}` | Set program counter; always consumes quota |
| `Abort` | Checked plugin failure; discard every output write |
| `End` | Return a complete output |

Programs are limited to 256 KiB and 4096 instructions. Inputs/outputs are limited to 1,048,576 pixels; memory is admitted against the editor's remaining workspace budget before preparation. Execution has at most 2,097,152 units; each opcode consumes a unit, inversion consumes a unit per pixel. There is no call stack, recursion, per-instruction allocation, infinite wait or unbounded loop. Cancellation is checked before execution and in bounded inversion batches. Missing End, invalid indexes/jumps, explicit failure, cancellation or quota exhaustion drops the isolated output. No native plugin code runs, so a plugin cannot inject a host panic instruction.

Verification covers unknown network/process/file/native/allocator operations, imports and extra fields, oversize programs, quota exhaustion after output writes, memory refusal, invalid indexes/jumps, Abort and cancellation with immutable input. Editor verification covers persisted local copies, preview isolation, adoption/Undo/Redo, stale result refusal, failures and Escape. The same fixture runs in the packaged smoke test. Tests use owned data and do not load third-party plugins.
