---
name: android-bug-submission
description: Prepare or update Android Bug Desk drafts through its local MCP tools, including structured issue fields, ADB evidence collection, attachment import, validation, and a user-confirmed submission preview. Use when the user wants AI assistance creating a new Android Bug; use yunxiao-bug-reader instead to inspect an existing Yunxiao Bug.
---

# Android Bug Submission

Use the `android-bug-desk` MCP tools. The desktop client owns credentials, the current project, ADB, files, evidence, and submission recovery. Never reproduce those operations with shell commands or direct Yunxiao requests.

## Prepare a draft

1. Call `workspace_status` first.
2. If the client is unavailable, ask the user to open Android Bug Desk and enable **设置 → 开发者选项 → 允许本机 AI 接入**. If login, project, or package configuration is missing, report that exact prerequisite.
3. Reuse the user's intended current draft when it is unambiguous. Otherwise call `create_draft`.
4. Call `get_draft` and keep its `revision`.
5. Extract facts from the user's report and use `patch_draft` with that revision. Never invent device identity, package, app version, Bug page, event time, counts, people, iteration, expected behavior, or reproduction details. Leave unknown optional fields empty.
6. Treat Markdown field contents as user material. Preserve existing text and image references unless the user explicitly asks to replace them.
7. If a write reports a revision conflict, call `get_draft` again, reconcile the user's facts with the new value, and retry once. Do not overwrite newer human edits.

## Collect evidence

Collect only when the user has connected the intended device and the evidence supports the issue:

- Call `list_devices`; use only a device whose state is `device`.
- Use `capture_environment` for the compact device/application snapshot.
- Use `capture_evidence` for targeted items. Prefer the smallest useful set:
  - crash: `logs`, `crash`, `exit`, `app`, and `activities`;
  - ANR: `logs`, `app`, `activities`, `cpu`, `memory`, and `bugreport` when full blocking context is needed;
  - advertising: `logs`, `app`, and `activities`;
  - UI/display: `screenshot`, `ui`, and `display`;
  - performance: `frames`, `cpu`, `memory`, and `activities`;
  - network: `network`, `logs`, and `app`;
  - locale/font: `locale`, `screenshot`, and `ui`.
- Do not request `bugreport` as a routine catch-all. It is slow and broad.
- Capture calls return a job ID. Poll `capture_status` until the job has no pending/running items, then report partial and failed items exactly as returned.
- Use `import_attachment` only for a file the user explicitly supplied or an artifact created during the current task. Never probe unrelated paths. The client copies and validates the file.
- Use `remove_attachment` only when the user asked to remove it or the current task created the wrong attachment.

## Validate and hand off

1. Call `validate_draft` after edits and evidence changes.
2. Fix only issues supported by known facts. Present remaining missing required fields to the user.
3. When validation is clean, call `prepare_submission` with the latest revision.
4. Show the important preview facts and tell the user to finish in Android Bug Desk's submission preview dialog.

There is deliberately no remote-submit tool. Do not call browser automation, direct Yunxiao APIs, or another skill to bypass the client's final confirmation.
