# Device input

Events preserve source kind, device/contact identity, phase, window logical coordinates, optional pressure and optional tilt/eraser/button attributes. The first accepted Down owns the stroke. Other contacts cannot move or finish it. Cancellation, focus loss or provider failure discards the pending preview. Device enable/stabilization, pen attributes and backend preference live in the local workspace, with staged Apply/Cancel, before-apply backup and restart restoration.

## Windows pen backends

Windows Pointer is the default. Mouse-only and driver-provided Wintab are explicit choices in Mouse and pen input. Exactly one native pen backend is retained. Switching or Retry finishes the current valid stroke once, closes the old provider before opening another and clears queued old-provider input. A failed Wintab initialization tries Pointer, then Mouse. Pointer initialization or runtime query failure leaves Mouse usable. Requested and effective backend plus the failure reason are displayed; retry is explicit, not repeated every frame.

All drawing mouse and pen messages enter the same root-window queue in Windows dispatch order. The UI still receives every original message. Physical client coordinates are divided by the current egui pixels-per-point. Native API tables, contexts, window owner and callback are UI-thread-bound through Rc; no unsafe Send/Sync implementations exist. Contexts and loaded DLLs drop before the retained root-window Arc.

Pointer resolves User32 APIs locally, identifies PT_PEN without pressure heuristics, reads coalesced history oldest first and preserves sourceDevice/pointerId. Optional pressure is normalized from 0..1024; absent pressure keeps the established full-pressure fallback. Only supported tilt fields are converted from degrees to radians. Pen inversion and barrel state come from explicit flags. Hover or an update without an owned Down never starts drawing. Capture loss, leave and cancelled contacts discard the pending preview.

Wintab dynamically loads only the installed System32 Wintab32.dll. No driver is shipped, installed or downloaded. Its Unicode API and integer-only LOGCONTEXTW/AXIS layout are implemented from the public ABI, not original drawing-program code. The context opens disabled, is enabled only for our focused root window and does not take over the system cursor. One WTPacket query consumes each matching serial message; no batch drain can move future pen samples ahead of mouse messages. Only negotiated supported packet fields are decoded from bounded initialized storage. Unsupported layouts, changed contracts, missing packets, invalid tip information and driver queue overflow cancel input and report failure. Pressure uses the reported min/max; orientation uses the reported circular resolution; inverted eraser uses TPS_INVERT. Tip state uses the explicit pressure-button mapping, with reported pressure as fallback. After switching/focus/cursor changes, a hover/lift is required before a new contact can begin. Absolute device coordinates map to the physical virtual desktop, including negative monitor positions, then to root client coordinates. Detailed user mapping belongs to item 048.

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
