---
description: Owns SunSwitcher runtime diagnostics, debug stderr behavior, and bounded rate-limited Windows release logging beside the executable.
---

# Production diagnostics

`src/diagnostics.rs` is the single owner of runtime diagnostic output. Production/runtime modules use `crate::runtime_log!` (the `sunswitcher` binary uses the exported `sunswitcher::runtime_log!`) instead of writing directly to stderr, so the same diagnostic events have one output policy.

Debug builds and non-Windows builds keep diagnostics on stderr for development. A Windows release build has no console subsystem and sends diagnostics through a non-blocking bounded channel to a dedicated logger thread. The logger writes `sunswitcher.log` next to the running executable, never writes the file more frequently than once per second after the first pending batch, and keeps the file at or below 1,000,000 bytes by retaining only the newest complete log tail when the bound would be exceeded. The keyboard/UI hot paths never perform log-file I/O; if the bounded log queue is full, diagnostics are dropped rather than blocking user input.

The production executable is `src/bin/sunswitcher.rs`, built as `sunswitcher.exe`. Its Windows release crate uses the GUI subsystem so launching it does not create a console window; the debug build retains the console shutdown handler for Ctrl+C development runs.