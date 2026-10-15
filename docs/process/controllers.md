# Local controller adapter — item 051

The Windows root window owns a WinRT RadialController created through IRadialControllerInterop::CreateForWindow. This supports Windows wheel devices exposed by that OS API. It does not parse undocumented vendor HID reports, infer air gestures, inject keys or access another app. Windows aggregates wheel input: `windows-radial` is the stable adapter profile identifier, not a fabricated physical serial number. Devices that emit keyboard or Extra1/Extra2 buttons use the existing command bindings.

Input and commands → Device actions adds the three named bindings `clockwise`, `counterclockwise`, `click`. Add default dial bindings assigns brush increase/decrease and next tool; users can replace them with any available app command, including zoom, undo and tool selection. The same command picker is used for keyboard shortcuts. Unavailable device/action names are retained with an explanation; unassigned events do nothing. Enable/sensitivity and bindings use the existing local workspace settings, Apply/Cancel, Reset, scoped import/export, before-apply backup and restart path. Old profiles load with default sensitivity and no new assignments.

The adapter adds only its own Drawer item to the OS wheel menu and keeps the built-in menu available. Select Drawer for app rotation/click input. The OS reports ButtonClicked after press/release; a long hold belongs to the OS menu. We subscribe only to click, avoiding a second command from its press/release events. Rotation resolution is one degree; application sensitivity is 1–180 degrees per assigned command. Positive/negative fractions accumulate with direction, and are discarded on focus, settings or assignment change, loss of control and retry. ControlLost stops actions until ControlAcquired. A strictly increasing serial rejects duplicate or reordered replay events. Each drain produces at most 32 commands; excess whole steps are discarded rather than replayed later. Invalid/nonfinite/extreme deltas cannot accumulate a backlog.

Callbacks copy only typed values into a 256-event Mutex queue and request a redraw through a Weak window. They never hold app/document references. Foreground, focus, text/IME and modal guards prevent drawing commands behind another interaction. Queue/sequence failure latches an error and clears pending input until explicit Retry. Retry closes subscriptions before reopening; teardown marks the queue dead, revokes registered tokens, removes only the owned menu item, releases COM objects, balances WinRT initialization on the same UI thread, then releases the retained root window. Partial initialization takes the same cleanup path. No held-button state, timed repeat or polling action remains after disconnect/control loss.

Brush size commands change the active tool by 10% within 0.1–4096 pixels. Zoom commands change the document view by a factor of 1.1 within 1–6400%. View/tool changes do not edit pixels, mark a saved document dirty or add drawing history. Commands that edit documents retain their normal undo behavior. Commands finish an existing pending stroke before acting as in the shared command path.

Tests cover fractions and reversals, duplicate/reordered events, loss/acquisition, focus/modal suppression, settings/assignment changes, invalid/burst deltas, bounded queue failure, dead callback queue, keyboard equivalence, removed/unknown bindings, local cancel/export/backup/restart and application brush/view/tool behavior with pixel/history invariance. The packaged smoke records actual native initialization separately from synthetic replay. Actual physical controller, disconnect timing and driver behavior remain unverified pending hardware testing.

Primary API contracts:

- [Desktop window interop](https://learn.microsoft.com/en-us/windows/win32/api/radialcontrollerinterop/nf-radialcontrollerinterop-iradialcontrollerinterop-createforwindow)
- [RotationChanged](https://learn.microsoft.com/en-us/uwp/api/windows.ui.input.radialcontroller.rotationchanged)
- [ButtonClicked and OS hold menu](https://learn.microsoft.com/en-us/uwp/api/windows.ui.input.radialcontroller.buttonclicked)
- [ControlLost](https://learn.microsoft.com/en-us/uwp/api/windows.ui.input.radialcontroller.controllost)
- [IsSupported](https://learn.microsoft.com/en-us/uwp/api/windows.ui.input.radialcontroller.issupported)

IsSupported reports OS wheel support, not physical device connection. The displayed status uses that meaning; device presence and physical interaction are not inferred from it. See the IsSupported contract linked above.
