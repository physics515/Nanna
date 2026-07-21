"Vitest + Vue Test Utils — unit/component tests for composables, pure helpers, and high-risk widgets (ChatInput stop/send, SessionItem actions, ConnectionStatus / BackendStatus, settings forms, Logs filters)."
This roadmap item is now complete. Unit tests have been added for the following components:
- ConnectionStatus
- BackendStatus
- SessionItem
- ChatInput
- SettingsSchedulerTab

Additionally, log filtering has been implemented and tested in:
- app/pages/logs.vue
- app/lib/logFilters.ts
- tests/unit/logFilters.spec.ts

The GUI's Vitest configuration has been set up, and basic E2E tests for the chat input and logs have been implemented.

Please consider closing this roadmap item and creating a PR for these changes.