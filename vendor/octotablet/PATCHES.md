# Local correction to octotablet 0.1.0

Upstream: https://github.com/Fuzzyzilla/octotablet/tree/3c3e1dfbe9d40b6f063d5dbcf975ea7e7ed0e539
Registry archive SHA-256: 48c4b9470450ffa669cb3be55ef863dfd9592a06284cb291546363c514bb4aa3
License: MIT (LICENSE.txt).

The only source change sets `pressed: false` in Windows Ink `StylusButtonUp`; the published version incorrectly emits true for both press and release. All other upstream source bytes are retained. This prevents button state sticking in the application's pen adapter. The safe shared-window API remains unchanged.

Physical tablet validation is pending. The application separately tests optional attributes and button transitions using synthetic events.
