# Codex Instructions

## Role

Codex investigates code, implements changes and verifies the result. Implementation
may be performed directly or delegated at the user's choice:
- **Direct mode:** Codex investigates and implements changes itself.
- **OpenCode mode:** Delegated to DeepSeek Flash via OpenCode CLI following the [AI workflow](docs/development/ai-workflow.md).
- **DSH mode:** Delegated to DeepSeek Harness via the `dsh_subagent` MCP server (DSH Web session on port 3080).

Before first implementing changes in a new session, ask the user which mode to use
if not already chosen, and keep the chosen mode until instructed otherwise.

Delegated tasks belong in `.work/ai/<work-id>/`, not tracked prompts in `docs/`.
In OpenCode mode, the lead agent may directly handle minor documentation,
configuration, typo and local build-error fixes.

### DSH (DeepSeek Harness) Delegation Rules

When **DSH mode** is selected:
1. **No direct code takeover:** The lead agent prepares the task in `.work/ai/<work-id>/tasks/`,
   starts DSH via `dsh_start`, and verifies the result. The lead agent MUST NOT write or
   modify product code for the delegated task directly, even for seemingly minor, simple,
   or localized edits.
2. **Mandatory wait & alert on `approval/asked`:**
   Whenever DSH pauses with an approval request (`approval/asked`, sandbox ACL, terminal command permissions):
   - **DO NOT** close the session (`dsh_close`), do not abort it, and do not take over the coding.
   - **Immediately alert the user in chat**: explicitly state that DSH is waiting for approval
     in DSH Web (`http://127.0.0.1:3080`) and describe the requested action/command.
   - Continue waiting for completion via `dsh_wait` until the user confirms the action in the Web UI.
3. **Attempt limit:** The fallback rule (Codex completing the task directly) applies ONLY after
   two completed, unsuccessful attempts by DeepSeek (verified defects in output/diff), NEVER
   because DSH was waiting for approval or needed more time.

After two unsuccessful DeepSeek attempts across any delegation mode, Codex finishes
the task directly under the [attempt-limit rule](docs/development/ai-workflow.md#лимит-попыток).

## Repository work

- Record a baseline before starting; do not mix existing user changes into the task.
- Restrict allowed paths and inspect the task-scoped diff.
- Do not treat an agent's answer or checklist as proof of completion.
- On Windows, do not run Rust checks/builds with bare `cargo`: use
  `scripts/cargo.ps1` for targeted commands and `scripts/build.ps1` for full
  debug/release builds so local toolchains and native dependencies apply.
- Commits and significant architectural decisions require explicit user approval.
- Verify current external information with available research tools; base technical
  conclusions primarily on primary sources.

## Sources of rules

For UI changes, consult the UI map and verification rules below. Update the
map when adding/moving screens. Respect the status of visual references:
proposals are not an approved general standard. Check relevant states and
scenarios; put the temporary report path in the specific task file.

- [Documentation map](docs/README.md)
- [UI map](docs/development/ui-map.md)
- [UI verification and approval](docs/development/ui-guidelines.md)
- [AI-assisted workflow](docs/development/ai-workflow.md)
- [Builds and checks](docs/development/README.md)
- [Architecture](docs/development/architecture.md)
- [Engineering decisions](docs/decisions/README.md)

Temporary prompts, reviews, logs and screenshots belong only in gitignored
`.work/ai/`. Add roadmaps, tasks, decisions and research to `docs/` only when
they remain useful after the current session.
