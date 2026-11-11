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
| **Detached notes & renames** (`core::detached`) | Runs off the UI thread whenever refs or the note set change. Renames come from the reflog (`Branch: renamed refs/heads/a to refs/heads/b`; git moves a branch's reflog on rename, so chains are visible). A rename is followed only if it happened after the note was last written, which guards against reused names. Reachability uses `cat-file --batch-check` plus `for-each-ref --contains`, skipping commits already in the loaded log. |
| **Auto re-attach by `git patch-id --stable`** | Identifies "the same change" independent of SHA, message, and line offsets. Candidates are non-merge commits on refs that aren't ancestors of the old commit. Commits with the same author email and author timestamp are checked first (rebase, amend, and cherry-pick preserve them, so usually only 1 is checked). If none match, the 200 most recent candidates are scanned. Only a unique match moves a note; several matches become suggestions. A move onto an existing note appends to it. |
| **Re-attach journal** (`core::journal`) | `<notes>/reattach.json` records each automatic move: the moved text, the destination's previous content, and its version right after the move. Undo restores both sides exactly when the destination is untouched. If it was edited, undo keeps the edits and only restores the source. Undone `(from, to)` pairs go on a reject list that `auto_reattach` honours. The last 100 entries are kept. |
| **Command palette** | Commands are rebuilt from repo state each time the palette opens: refs, remotes, stashes, worktrees (`git worktree list --porcelain`), and git-flow config (one `git config --get-regexp`). Each command is a list of steps (pick, input, confirm) plus a closure that produces actions, so new commands are a few lines each. The fuzzy matcher favours word starts and consecutive runs. Groups are ordered by their best match. |
| **Native git flow** (`core::gitflow`) | Plain git commands against the `gitflow.*` config keys. Merges use `--no-ff`. Each finish step is skipped if already done (`merge-base --is-ancestor`, existing tag), so a finish interrupted by a conflict resumes after you commit. The branch is deleted only after it's verified to be merged into every target. |
| **Margin Pro licensing** (`core::license`) | Keys are `MARGIN1.<payload>.<sig>`: base64url JSON (name, email, updates_until) signed with Ed25519 (`ed25519-compact`, verify-only in the app). There's no server. The public key is compiled in from `MARGIN_LICENSE_PUBKEY`. A key covers builds up to `updates_until`, compared with the build date that `build.rs` embeds (`SOURCE_DATE_EPOCH`-aware). The trial starts at first launch (`state.json`). The app and MCP server share the same status check; the app re-checks on window focus and drops GitHub data when Pro turns off. Issuing is done by `crates/margin-license`. |
| **Icons without an SVG crate** | GitHub and note icons are Octicons path data, rasterised by a ~150-line nonzero scanline filler (4×4 supersampling, cubic/quadratic/arc flattening) at the exact pixel size, then cached as tinted textures. resvg would add 1–2 MB. |
| **GitHub via GraphQL** | One aliased query returns the latest PR for up to 25 branches. A second query loads comments and checks only for the branch on screen. The PR head is the branch's upstream name, so a local name that differs from the remote still works. The token comes from env, `gh auth token`, or the Keychain. |
| **ureq + rustls-platform-verifier** | Blocking HTTP (no runtime). Uses the OS trust store, so corporate TLS inspection works. Proxy settings come from the environment. |

## Known limitations (v0.1)

- By design, commit notes stay on the old SHA after a rebase or amend. They show up under *Detached notes* with a suggested new commit; re-attaching is manual.
- Rename detection needs reflogs (on by default in non-bare repos). With `core.logAllRefUpdates=false`, a terminal rename leaves the note detached; re-attach it from the list.
- PRs from forks are not detected. Margin queries the repo of `origin`.
- Times show in UTC. The commit list loads 2000 commits per page ("Load more").
- Git status refreshes on focus and after operations; there's no filesystem watching yet.
- No syntax highlighting, by choice for now. It would add weight (see roadmap).
- HTTPS auth needs a credential helper (osxkeychain or GCM). Margin never prompts for passwords.
- Dark theme only. Settings are edited in `config.json`.

## Roadmap

1. **Packaging:** a signed and notarized `.app` and universal binary, MDM-friendly (pkg), and a `margin` CLI shim like `smerge`.
2. **Git depth:** line-level staging, merge-conflict resolver, interactive rebase, blame, file history, server-side search (`git log -S/--grep`), FSEvents watcher.
3. **Notes:** optional team sharing through `refs/notes/margin` push/fetch, export a branch's notes into a PR description.
4. **Tasks:** API cards for Linear and Asana (today they're link-out only), Jira transitions (start ticket → "In Progress" when the branch is created), worklogs, and comments. Providers for Linear, GitHub Issues, GitLab, and Azure Boards behind the existing `TaskProvider` trait.
5. **MCP:** Streamable HTTP transport (for remote agents), resource subscriptions for notes, an opt-in "write" tool tier (stage/commit) behind a config flag.
6. **Windows:** the code builds on Windows (CLI git, glow). Remaining work is packaging and credential-manager testing.
