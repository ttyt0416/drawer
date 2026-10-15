# Device input

Drawing events preserve source kind, a session device ID, contact ID, phase, window logical coordinates and optional pressure. The first accepted Down owns the stroke. Other contacts cannot move, release or finish it; they need a new Down after ownership ends. Cancel, repeated Down, focus loss and device removal discard the pending preview. Device-specific settings (enabled, optional stabilization override) are stored independently in the local workspace, with before-apply backup and staging/cancellation.

Mouse ID 0 denotes egui-winit's aggregate mouse, not a claimed physical mouse identifier. Untyped egui Touch preserves device/contact IDs and pressure but does not draw; pressure alone never identifies a pen. The immediate pointer sequence promoted by egui-winit is attributed to that touch, not a second real mouse. OS-promoted mouse attribution and backend selection have separate checklist items 043/046.

The Windows pen adapter uses octotablet 0.1.0's safe build_shared API with eframe's shared root window. Mouse emulation is disabled. Tool types identify pens without pressure heuristics; IDs are mapped without hash collisions for the tool lifetime. Down/Up/Pose are collected through Frame because the native library documents simultaneous frame events as unordered. Native logical coordinates are divided by egui's additional zoom factor. Native pen frames and GUI mouse batches retain their own delivery order; no claim is made about hardware cross-backend timestamp ordering. No other applications are hooked. The manager drops before COM initialization is released. Missing/declined native initialization leaves mouse input usable. This library version panics on some native build failures; the optional build is caught and reported.

Physical pen pressure/latency/tilt, multi-device driver behavior and promoted OS mouse input remain pending hardware validation. A successful connection or synthetic pen replay is not a physical tablet test. Pointer/Wintab switching is not marked complete here.

Primary API/source references:
- https://docs.rs/octotablet/0.1.0/octotablet/builder/struct.Builder.html
- https://docs.rs/octotablet/0.1.0/octotablet/events/enum.ToolEvent.html
- https://github.com/Fuzzyzilla/octotablet/tree/main
- https://github.com/emilk/egui/blob/0.36.2/crates/egui-winit/src/lib.rs
- https://learn.microsoft.com/en-us/windows/win32/tablet/realtimestylus-reference
