# Architecture

```
crates/
  margin-core   no UI. git CLI wrapper, diff parser, graph layout, notes store,
                Markdown policy, Jira client, config/secrets
  margin-app    egui GUI (binary: margin)
  margin-mcp    MCP stdio server (binary: margin-mcp). No GUI deps, ~2.7 MB
```

The GUI and the MCP server never talk to each other directly. Both work on the same notes directory, and the GUI notices changes through a 1 Hz stat watcher on a background thread. This keeps the protocol surface at zero and lets several agents and windows work at once.

## Key decisions

| Decision | Why |
|---|---|
| **egui + glow** (not wgpu, Tauri, or Electron) | Small binary, fast cold start, ~0 idle CPU in reactive mode, dense Sublime-like UI. `system_fonts` memory-maps OS fonts on demand, so Korean, Japanese, and Chinese render without bundling CJK fonts. |
| **Shell out to `git`** (not libgit2 or gitoxide) | Enterprise parity: hooks, GPG/SSH signing, credential helpers and SSO, `includeIf`, LFS, sparse/partial clone, fsmonitor. Reads use porcelain v2, `-z`, and explicit formats. `GIT_OPTIONAL_LOCKS=0` avoids fighting your terminal for locks. `GIT_TERMINAL_PROMPT=0` means git never hangs. `MARGIN_GIT` can pin a vendored git. |
| **Background threads, no async runtime** | Each git or HTTP call runs on a short-lived thread and posts a message. Results carry a generation counter so stale diffs are dropped. Status, log, and refs load in parallel. |
| **Notes as files in the common git dir** | No database. Agents and humans can both edit. Notes are never committed by accident, survive checkouts, and are shared by worktrees. Writes are atomic (temp file + rename) and versioned (`if_version`). Appends retry on conflict. |
| **Markdown policy in core** | One `validate()` used by the GUI and MCP: no images, no HTML, and only http(s), mailto, relative, and `margin:` links. Size is capped at 256 KB. |
| **MCP hand-rolled** | Newline-delimited JSON-RPC, ~350 lines, no tokio. Supports protocol 2024-11-05 … 2025-11-25 and returns `structuredContent`. |
| **Two note kinds** | Commit notes are keyed by SHA and deliberately never move. Branch notes are keyed by branch name, so they survive rebase and amend for free, and the UI shows them on the branch's current tip (`tips` map: commit → local branches). No rewrite tracking or hooks are needed. |
| **GitHub via GraphQL** | One aliased query returns the latest PR for up to 25 branches. A second query loads comments and checks only for the branch on screen. The PR head is the branch's upstream name, so a local name that differs from the remote still works. The token comes from env, `gh auth token`, or the Keychain. |
| **ureq + rustls-platform-verifier** | Blocking HTTP (no runtime). Uses the OS trust store, so corporate TLS inspection works. Proxy settings come from the environment. |

## Known limitations (v0.1)

- By design, commit notes stay on the old SHA after a rebase or amend. If that commit becomes unreachable, its note is only visible through MCP `list_notes` for now (a GUI "detached notes" list is planned).
- Renaming a branch (`git branch -m`) leaves its branch note under the old name.
- PRs from forks are not detected. Margin queries the repo of `origin`.
- Times show in UTC. The commit list loads 2000 commits per page ("Load more").
- Git status refreshes on focus and after operations; there's no filesystem watching yet.
- No syntax highlighting, by choice for now. It would add weight (see roadmap).
- HTTPS auth needs a credential helper (osxkeychain or GCM). Margin never prompts for passwords.
- Dark theme only. Settings are edited in `config.json`.

## Roadmap

1. **Packaging:** a signed and notarized `.app` and universal binary, MDM-friendly (pkg), and a `margin` CLI shim like `smerge`.
2. **Git depth:** line-level staging, merge-conflict resolver, interactive rebase, blame, file history, server-side search (`git log -S/--grep`), FSEvents watcher.
3. **Notes:** a "detached notes" list for commit notes whose commit is gone, branch-rename handling, optional team sharing through `refs/notes/margin` push/fetch, export a branch's notes into a PR description.
4. **Tasks:** API cards for Linear and Asana (today they're link-out only), Jira transitions (start ticket → "In Progress" when the branch is created), worklogs, and comments. Providers for Linear, GitHub Issues, GitLab, and Azure Boards behind the existing `TaskProvider` trait.
5. **MCP:** Streamable HTTP transport (for remote agents), resource subscriptions for notes, an opt-in "write" tool tier (stage/commit) behind a config flag.
6. **Windows:** the code builds on Windows (CLI git, glow). Remaining work is packaging and credential-manager testing.
