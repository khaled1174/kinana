# Native acceptance tests — batch6.2b

- `shell.e2e.mjs`: four actual macOS Tauri WebView tests: Arabic/RTL, public core bootstrap, navigation/language, and runtime CSP enforcement. One screen title, no marketing hero or engine card.
- `vault.e2e.mjs`: thirteen tests using a real imported vault and a live Claude proposal: bilingual seven-row screenshots and full word diff; cost preview/confirmation/stale-estimate rejection and live agent classification; audit level explanations; CLI-equivalent combined filters; pin/unpin; review/undo; category/reject/undo; local and ZIP preparation/cancel; exact raw text/details; bundle creation/editing; direct opener ACL denial and ID validation; malicious Markdown preview with loopback resource observation and unchanged source/dist.
- `release-isolation.mjs`: default user release versus e2e positive control, checking symbols/strings, frontend markers, Cargo graph, native window, liveness and sockets. macOS `nm`, `strings`, `lsof`, Swift/CoreGraphics required.

Use `npm ci` and build both native binaries first. Follow `../README.md` to prepare actual skills and one preparation model call, then run `test:ui`; every run clones that real baseline into a fresh temporary HOME. A second live model call is confirmed by the UI test and undone. Tests do not fabricate skills or intercept core IPC. Evidence is written to `../../docs/evidence/phase-6/batch-2b/`.

The isolation acceptance must run again against the final phase7 user artifact. The complete escaping-disabled CSP/IME/error/800×600 acceptance remains6.5. Native keyboard composition is not claimed by synthetic WebDriver events.

## Live agent calls — `KINANA_LIVE_AGENT=1`

Two tests run the real `claude` CLI on the owner's account: the live classification in `vault.e2e.mjs` and the classification step of `onboarding.e2e.mjs` (run with `KINANA_UI_EMPTY=1`). They run only when `KINANA_LIVE_AGENT=1` is set (owner decision 2026-11-17). Without it they are reported as skipped (pending) with a `SKIPPED:` line, and the rest of both suites still runs. One run of each with the variable set makes two agent calls in total.
