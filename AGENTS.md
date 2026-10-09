# Android Bug Desk implementation notes

- Use Rust + Tauri 2, with React/TypeScript for the renderer. The user's Tauri decision overrides the Electron recommendation in the preserved original plan.
- Target macOS arm64/x64, Windows x64, and Linux x64. Packaging runs natively through GitHub Actions.
- Read `docs/tasks.md` and `docs/acceptance.md` before continuing. Report implemented, mock-verified and real-world-verified capabilities separately.
- Cloud verification remains subject to the current task's authorization. Never read another app's or coordinator's PAT. Do not perform real Yunxiao writes without a user-designated acceptance scope.
- Read `docs/git-delivery-policy.md` before GitHub operations. The public remote is `guozhiqiang123/AndroidBugDesk`; normal task pushes, candidate builds and accepted PR merges follow its gates. A pending public-history review blocks source pushes. Publishing releases, force pushes and destructive operations are outside this policy.
- Keep credentials, network, ADB and file access in Rust. Do not add generic shell commands, unrestricted file-path IPC or renderer HTTP permissions.
- Preserve unknown create/upload outcomes and remote workitem identity. An empty search is not evidence that creation failed. Do not solve retries by posting another Bug.
- Keep the original evidence ZIP immutable and store receipts separately. Unknown build fingerprints/mapping identity stay explicitly uncollected.
- When changing the draft structure, update `src/shared/domain.ts`, run `npm run schemas`, update fixtures, and verify both frontend and Rust validation. Unknown required fields must block writes.
- Run meaningful validation for changed behavior: `npm run check`, Rust formatting and Clippy. Logs should stay concise; retain full noisy logs outside source control.
- Do not mark the whole product accepted until the original plan's real media, USB, permission and installed-platform tests have passed.
