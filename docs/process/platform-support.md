# Actual platform support and item 285

| Target | Status |
| --- | --- |
| Windows x64 / MSVC | Current packaged and tested build |
| macOS arm64 | Port/build/runtime verification pending; no supported binary supplied |
| Linux x64 | Port/build/runtime verification pending; no supported binary supplied |

The current host has only `x86_64-pc-windows-msvc` installed in Rust. The ordinary WSL inventory was access denied; the elevated read-only inventory succeeded with an empty list. No installed Linux distribution was found through that inventory. No macOS toolchain/SDK or host is attached. No operating system features, VM images, drivers or host networking were changed.

Common native documents, mouse replay and storage logic live in Rust core modules; Windows device/OS code has a separate platform crate. That separation alone does **not** establish macOS/Linux support. Target-specific dependencies, local path/data-root rules, clipboard/dialog/file operations, graphics startup/failure and device fallbacks still require porting and compiling on those targets. Item 285 stays unchecked.

`scripts/test-platform.py` is a runnable real-host acceptance harness. On each target, supply a locally built executable, exact target triple and an evidence path under the checkout. It checks the actual OS/architecture before launching the owned app with isolated local data, controlled PATH and a timeout, then requires real document/mouse/save/recovery receipt flags. It kills/reaps a timed-out app and removes only its bounded, non-linked temporary directory. A receipt is partial evidence and never automatically grants platform support.

Example on this Windows host:

```text
python -X utf8 -B scripts/test-platform.py --exe dist/Drawer-windows-x64/Drawer.exe --expect-target x86_64-pc-windows-msvc --evidence docs/process/platform-windows-evidence.json
```

After real builds exist on all targets, transfer the same native documents in both directions (including Unicode, case-sensitive paths, missing fonts/model sources and permissions), test actual mouse/graphics failure handling and attach those receipts before marking 285 complete. Physical pen checks remain separate items 296–300. No other target is marketed as supported while these checks are absent.
