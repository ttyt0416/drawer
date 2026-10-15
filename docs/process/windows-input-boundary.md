# Windows message input boundary

Only the Drawer root window is recorded. A winit message callback reads the current thread's GetMessageExtraInfo at dispatch time, preserves mouse phase and signed client physical coordinates, then sends the original message unchanged to winit/UI. Drawing consumes this queue directly instead of trying to match metadata back to merged GUI pointer events. Physical coordinates are divided by the receiving egui pixels-per-point. Secondary display windows are not drawing targets.

Document input rejects OS-marked pen-compatible mouse events while a native pen provider is connected. Unmarked mouse events are always retained, including identical positions and adjacent timestamps. If native pen initialization is unavailable, compatible mouse input remains a fallback. Touch-marked mouse events retain Touch kind and do not draw. No distance/time/device-ID guessing is used. The source marker is per event, so there is no suppression timer to survive a contact end. Each source origin tracks its own mouse-button state; a promoted pen release cannot release the actual mouse. A move whose button flag is no longer down becomes Cancel. The bounded 65536-event queue discards overflow and blocks that mouse contact until release; pending document edits are discarded.

The new drawer-platform crate is the sole Drawer-owned unsafe exception. Its crate and manifest deny unsafe code except for the private Windows module. Application, document and handoff crates still inherit unsafe_code=forbid. There are exactly two unsafe expressions: borrowing the live MSG pointer furnished by winit's documented with_msg_hook callback, and a parameterless GetMessageExtraInfo query. The pointer is null-checked, borrowed only during the synchronous callback, never stored or exported. Only numeric owned event data crosses into safe consumers. No global hook, foreign process access, network operation or input injection is used. Clippy denies undocumented unsafe blocks.

Native smoke proves that the adapter attaches to its own running window; synthetic tests cover OS signature decoding, own/foreign window selection, signed coordinates, delivery order, DPI conversion, overflow, button-loss cancellation and pen+promotion pixel equivalence. Real tablet/driver duplication behavior is pending hardware validation. Unmarked promoted events cannot be distinguished from actual mice and are intentionally not guessed away.

Primary references:
- https://learn.microsoft.com/en-us/windows/win32/tablet/system-events-and-mouse-messages
- https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getmessageextrainfo
- https://docs.rs/winit/0.30.13/winit/platform/windows/trait.EventLoopBuilderExtWindows.html#tymethod.with_msg_hook
