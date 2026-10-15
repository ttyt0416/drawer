# Local input preferences

Default IME behavior is Keep. Text fields can request Open/Closed; number fields can request Keep/Closed. An input session captures the previous open flag, defers a change during composition, and restores it on field/window exit. The adapter acquires only the focused window of the current Drawer UI thread. It never changes keyboard layout, conversion mode or sentence mode. Failed writes are checked by reading the flag back and restoring the preceding flag.

Own code retains unsafe_code=forbid. Windows-only pinned dependencies michiu_window 0.0.3 and winsafe 0.0.29 expose safe wrappers; windows 0.62.2 provides the handle type. The audited path uses GetFocus, ImmGetContext, ImmGetOpenStatus, ImmSetOpenStatus and ImmReleaseContext only. No IME composition/result-buffer queries or relay/network services are invoked. Native smoke probes only its own hidden Drawer window and restores the flag before returning.

The complete local tutorial is embedded and installed in HELP.md. Suppressible confirmations concern settings drafts only. Save/discard confirmation for unsaved drawings cannot be suppressed.

API sources:
- https://learn.microsoft.com/en-us/windows/win32/api/imm/nf-imm-immgetopenstatus
- https://learn.microsoft.com/en-us/windows/win32/api/imm/nf-imm-immsetopenstatus
- https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getfocus
- https://github.com/eto0202/michiu/tree/6235158b40e8678ab7634152c7ea939da9bcbfbb/crates/michiu_window/src/ime.rs
- https://docs.rs/winsafe/0.0.29/winsafe/struct.HWND.html
