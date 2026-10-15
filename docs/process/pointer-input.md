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

## Optional pen attributes

Tilt (radians), inverted eraser and any reported pen-button state are optional, separate from pressure. Non-finite tilt becomes absent and each finite axis is clamped to +/- pi/2. Selected tilt-angle mapping transforms the projected tilt vector through the inverse view transform, including mirror/DPI. Eraser/button actions are selected only for identified pens at Down, retain the current brush shape/pressure and remain fixed until Up/Cancel. Button mapping is off by default: the backend does not guarantee physical button numbering and may include the nib. Individual button assignments belong to item 047. Synthetic events exercise these mappings; a connected provider is not a physical hardware test.

The published octotablet 0.1.0 reports StylusButtonUp with pressed=true. A licensed local source copy in vendor/octotablet changes only that flag to false. Its upstream archive checksum, revision, every patched-tree file hash and MIT license are included in the component inventory. No original drawing-program code is included. Cargo's registry patch pins the corrected source; the license checker rejects changes or extra files in that tree. Format Drawer-owned crates with `cargo fmt -p drawer-app -p drawer-core -p xtask -p drawer-platform` to preserve upstream bytes. Two existing upstream compiler warnings (unused status, unnecessary transmute) remain in that dependency; Drawer-owned static checks pass with warnings denied.

OS-compatible mouse attribution is now implemented by the separate native message queue; see windows-input-boundary.md. Pointer/Wintab selection still belongs to item 046.
