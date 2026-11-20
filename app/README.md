# Kinana Desktop

Build the frontend and native binaries from the repository root with the
commands in the [project README](../README.md#البناء-والاختبارات).

The production frontend is emitted to app/dist. Native end-to-end tests use
the separate e2e frontend and Tauri feature. The e2e feature enables the
WebDriver plugins and is for an isolated test user only. Agent classification
tests that make live calls require the operator's explicit authorization.
