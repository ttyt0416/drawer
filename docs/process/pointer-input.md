# Device input

Events preserve source kind, device/contact identity, phase, window logical coordinates, optional pressure and optional tilt/eraser/button attributes. The first accepted Down owns the stroke. Other contacts cannot move or finish it. Cancellation, focus loss or provider failure discards the pending preview. Device enable/stabilization, pen attributes and backend preference live in the local workspace, with staged Apply/Cancel, before-apply backup and restart restoration.

## Windows pen backends

Windows Pointer is the default. Mouse-only and driver-provided Wintab are explicit choices in Mouse and pen input. Exactly one native pen backend is retained. Switching or Retry finishes the current valid stroke once, closes the old provider before opening another and clears queued old-provider input. A failed Wintab initialization tries Pointer, then Mouse. Pointer initialization or runtime query failure leaves Mouse usable. Requested and effective backend plus the failure reason are displayed; retry is explicit, not repeated every frame.

All drawing mouse and pen messages enter the same root-window queue in Windows dispatch order. The UI still receives every original message. Physical client coordinates are divided by the current egui pixels-per-point. Native API tables, contexts, window owner and callback are UI-thread-bound through Rc; no unsafe Send/Sync implementations exist. Contexts and loaded DLLs drop before the retained root-window Arc.

Pointer resolves User32 APIs locally, identifies PT_PEN without pressure heuristics, reads coalesced history oldest first and preserves sourceDevice/pointerId. Optional pressure is normalized from 0..1024; absent pressure keeps the established full-pressure fallback. Only supported tilt fields are converted from degrees to radians. Pen inversion and barrel state come from explicit flags. Hover or an update without an owned Down never starts drawing. Capture loss, leave and cancelled contacts discard the pending preview.

Wintab dynamically loads only the installed System32 Wintab32.dll. No driver is shipped, installed or downloaded. Its Unicode API and integer-only LOGCONTEXTW/AXIS layout are implemented from the public ABI, not original drawing-program code. The context opens disabled, is enabled only for our focused root window and does not take over the system cursor. One WTPacket query consumes each matching serial message; no batch drain can move future pen samples ahead of mouse messages. Only negotiated supported packet fields are decoded from bounded initialized storage. Unsupported layouts, changed contracts, missing packets, invalid tip information and driver queue overflow cancel input and report failure. Pressure uses the reported min/max; orientation uses the reported circular resolution; inverted eraser uses TPS_INVERT. Tip state uses the explicit pressure-button mapping, with reported pressure as fallback. After switching/focus/cursor changes, a hover/lift is required before a new contact can begin. Native tablet coordinates retain their device resolution and normalize to the full tablet area; the default path maps them to the physical virtual desktop, including negative monitor positions, then to root client coordinates. User area mapping is described below.

Mouse ID 0 is the GUI aggregate, not a physical-device claim. OS-marked pen-compatible mouse events are suppressed while a native pen backend is active; real unmarked mice are retained. Touch is not guessed to be a pen. The headless egui fallback retains untyped touch IDs and ignores its immediate promoted pointer sequence. See windows-input-boundary.md for origin and queue limits.

## Optional attributes and verification limits

Tilt radians are finite and clamped to +/- pi/2. Selected tilt-angle mapping includes inverse view rotation, mirror and DPI. Eraser/button action is selected only for identified pens at Down and stays fixed through the stroke. The current shape and pressure mapping are preserved. Button-to-eraser is off by default; Wintab's button set can include the nib. Individual assignments belong to item 047.

Synthetic tests cover packet layouts, absent/invalid fields, pressure/tilt math, lift/cursor cancellation, initialization fallback/exclusivity, settings Apply/Cancel/backup/restart and in-flight stroke switching/retry. The packaged native smoke initializes Wintab or its fallback, switches Mouse, then restores Pointer, without injecting input or operating other applications. These checks do not verify actual pen pressure, latency, driver coordinate behavior, tilt or multi-device hardware. Physical pen validation remains pending as requested.

The previous RTS/octotablet adapter and its patched vendor tree were removed when these native backends replaced it. License inventory now describes the dependencies actually shipped. Source history preserves earlier provenance.

Primary API contracts:
- https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getpointerinfo
- https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getpointerpeninfo
- https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getpointerpeninfohistory
- https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-pointer_pen_info
- https://developer-docs.wacom.com/docs/icbt/windows/wintab/wintab-reference/
- https://developer-docs.wacom.com/docs/icbt/windows/wintab/wintab-basics/
- https://learn.microsoft.com/en-us/windows/win32/api/libloaderapi/nf-libloaderapi-loadlibraryexw

## Drawing click and coordinate controls (047)

Local controls default to no extra click threshold, absolute coordinates, relative gain 1 and all numbered buttons unchanged. The safe core adapter runs before ownership for native and virtual input alike. Enabled pressure threshold is inclusive; crossings emit Down/Up while the physical contact remains, final Up/Cancel ends it. Missing pressure resolves to 1; real zero stays zero. Invalid numeric input cancels the affected contact. Repeated Down cancels and blocks until lift. At most 32 raw pen contacts are retained. Touch/mouse input is not transformed.

Relative mode starts at the last canvas center, integrates window-logical displacement times gain (0.05..10), clips to the receiving content rectangle and carries its drawing cursor across lift/repress without a new absolute jump. It changes no OS pointer position or menu click. Focus loss, settings/provider change, external stroke finish and retry clear its state. Before a threshold click is accepted the virtual cursor is still visible. Native tilt/eraser attributes and brush pressure calculations are separate.

Optional logical button bits are explicit API data, never inferred from position/pressure. Pointer exposes barrel bit 0. Wintab excludes the identified nib bit, preserving the driver's other logical bit numbers. Unknown numbering is None. The 32 slots can keep defaults, use the selected brush, erase or ignore drawing. Ignore > eraser > current-brush priority is deterministic. Ignore blocks until physical lift; the selected tip remains fixed for a logical stroke. Legacy any-button eraser remains available separately. These controls are application drawing policy, not driver settings or a promise of physical button numbering. Hardware testing remains pending.

## Tablet area, target and touch offset (048)

Area mapping is off by default. When enabled, it consumes explicit optional normalized tablet coordinates, crops a valid nonzero 0..1 source rectangle and maps it to the active canvas rectangle or a selected physical monitor rectangle. Pointer derives UV from ptHimetricLocationRaw and GetPointerDeviceRects; missing/degenerate API output remains absent. Wintab retains native out-origin/extent resolution, including signed extents, normalizes with its Y-axis orientation and computes default desktop coordinates in floating point before client conversion. Neither native backend guesses tablet UV from client cursor position.

The receiving root client's physical screen origin and egui pixels-per-point convert monitor output to window logical points, including negative desktop positions and additional UI zoom. Selected monitor physical position/size is persisted. A missing or changed rectangle does not silently select another monitor. Canvas mapping uses the current drawing view (including arranged tabs). Window/view/layout/scale or mapping changes finish the old valid stroke, clear stale input and require a fresh contact. Source-area exit/unsupported data cancels a pending preview; reentry while held does not start drawing. Relative policy/gain from item 047 runs after this mapping.

A bounded physical-pixel touch offset is divided by receiving scale and applied only to identified touch. It does not enable finger drawing or turn an unknown input into a pen. The settings window's mouse preview explicitly supplies virtual UV, calculates through the same mapper and changes no document. The independent replay has a tablet-area sample for pixel/history verification. Apply/Cancel, backup and restart include all mapping settings; invalid zero areas and nonfinite values are rejected. These are local application mappings; physical devices/driver mapping and touch calibration remain unverified.

Primary device-area API: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getpointerdevicerects

## Known touch and gestures (050)

Known native touches use an independent root-window WM_TOUCH adapter; it remains attached when the selected pen backend changes. Unknown egui touch and pressure-only contacts are not reclassified. Optional palm flags and explicit native batch IDs accompany the existing source/phase coordinates. Same-device contact identity and complete native-batch grouping drive two-finger view-only pan/zoom/rotation, with explicit single-finger drawing and OS-palm rejection settings. See touch.md for lifecycle, priority, callback ownership and verification limits. Touch-offset correction from item 048 runs before these policies. Untyped contacts still never draw.
