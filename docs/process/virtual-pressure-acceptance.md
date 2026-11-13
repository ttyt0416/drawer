# Item 295: virtual pressure extremes and recovery

`scripts/test-checklist.ps1 -Item 295` uses actual editor pen-event processing. Five steady raw pressures (0, 0.25, 0.5, 0.75, 1) pass through a nonlinear response with minimum0.25/maximum0.75 and fixed knots [0,0.1,0.25,0.8,1]. The independently expected center-pixel alpha is [0,0,64,255,255]. Zero output preserves pixels/history; nonzero output is one semantic Undo/Redo transaction.

Seven invalid values × Down/Move/Up give21 cases: -1,2, negative/positive finite maximum, NaN and both infinities. The raw pen-event boundary rejects these values; if a contact already exists, the pen-control pipeline cancels its entire pending stroke. Subsequent move/up cannot resume. A fresh valid press must draw and Undo to the exact baseline. Pure finite curve calculations clamp extreme inputs to 0/1, while non-finite curve calculations refuse them. These are two explicit boundaries, not a claim that raw driver events are silently clamped.

A valid abrupt [0,1,0,1,0.001,0.999,0.5,1] sequence is replayed twice through the production input replay path. Exact RGBA repeat, one stroke, full semantic Undo/Redo and contact cleanup are required. The receipt retains the versioned abrupt input record, steady expected/actual alpha and invalid raw input IEEE754 hexadecimal bits. NaN/Infinity are named bit records rather than illegal JSON numeric values.

This is virtual input evidence. Actual device minimum contact pressure, curves, maximum pressure and lift-off remain item296 physical tests.
