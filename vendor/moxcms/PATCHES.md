# Drawer local clock correction

Based on moxcms0.8.1 from the locked crates.io archive. Original license/copyright files are preserved.

Only src/dat.rs is changed: now() no longer calls Default on a pre-epoch SystemTime, because Default calls now() recursively. Signed UTC seconds, floor division for negative dates, and a1..9999 metadata year clamp preserve correct pre-1970 dates without stack overflow. The existing post-epoch calendar algorithm and WebAssembly fixed-time fallback are retained. Five deterministic calendar cases accompany the fix. Actual installed-app runs with1960 and2099 wall-clock API results validate integration.
