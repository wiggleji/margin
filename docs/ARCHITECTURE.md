# Architecture

```
crates/
  margin-core   no UI. git CLI wrapper, diff parser, graph layout, notes store,
                conflicts, interactive rebase, insights, GitHub and Jira clients,
                self-update, config/secrets, per-OS helpers
  margin-app    egui GUI (binary: margin)
  margin-mcp    MCP stdio server for coding agents (binary: margin-mcp). No GUI deps.
```

Dependencies point one way: `margin-app` and `margin-mcp` use `margin-core`; the core never knows a window exists. The GUI and the MCP server never talk to each other directly. Both work on the same notes directory, and the GUI notices changes through a 1 Hz stat watcher on a background thread. This keeps the protocol surface at zero and lets several agents and windows work at once.

Most features come as a pair: the logic in a `margin-core` module (testable without a window, often against a real temporary git repository) and the screen in a `margin-app` module of the same or a similar name.

| Feature | Logic (`margin-core`) | Screen (`margin-app`) |
|---|---|---|
| History graph | `git.rs`, `graph.rs`, `history.rs` | `commit_list.rs`, `search.rs` |
| Diffs, hunk staging | `diff.rs` | `diff_view.rs`, `details.rs` |
| Notes | `notes.rs`, `detached.rs`, `journal.rs`, `sync.rs`, `markdown.rs` | `notes_panel.rs`, `markdown_view.rs` |
| Review mode (branches, PRs) | `github.rs`, `review.rs` | `review.rs`, `pr_view.rs`, `mention.rs` |
| Conflict editor | `conflict.rs` | `conflicts.rs` |
| Interactive rebase | `rebase.rs` | `rebase.rs` |
| Stashes | `git.rs` (`stash_*`) | `details.rs`, `sidebar.rs` |
| Insights | `insights.rs` | `insights.rs` |
| Command palette | — | `commands.rs`, `palette.rs` |
| Self-update | `update.rs` | `app.rs` |
| Per-OS code | `platform/` | `platform.rs`, `menu_bar.rs`, `cli.rs` |

## How the app runs

**One loop.** egui is immediate-mode: every frame (a click, a key, a finished job) `MarginApp::ui` in `app.rs` runs top to bottom and draws the whole window from Margin's own state. Views don't change state while drawing; they push what the user asked for as an `Action` (one big enum in `app.rs`), and `handle()` acts on them after the frame is drawn, when nothing is borrowed. Adding a feature usually starts with an `Action` variant.

**Background work.** Anything slow (git, HTTP) runs on a short-lived thread via `Jobs::spawn` (`jobs.rs`) and comes back as a `Msg` over a channel; the next frame's `poll()` hands it to `on_msg()`. Git operations go through `op()`, which runs one at a time (the "Busy" state) and refreshes afterwards. A refresh loads status, refs and the log in parallel into one `Snapshot`. Results carry the repository path, and many carry a generation counter, so an answer for a tab or commit you've moved away from is dropped.

**State.** `MarginApp` holds the app-wide state; each open repository tab is a `RepoView` (its `Snapshot`, selection, diffs, notes editor, and whichever mode is open: review, conflicts, rebase plan, insights). Preferences and layout are saved as JSON in the config folder (`config.json`, `settings.json`, `state.json`).

## Key decisions

| Decision | Why |
|---|---|
| **egui + glow, with wgpu on Windows** | OpenGL (glow) gives a small binary, fast cold start and ~0 idle CPU in reactive mode. `system_fonts` memory-maps OS fonts on demand, so Korean, Japanese and Chinese render without bundling CJK fonts. Windows also carries egui's wgpu renderer with only the Direct3D 12 backend: when OpenGL 2+ isn't available (VMs, some Remote Desktop sessions, no graphics driver, winget's validation machines), Margin starts itself again with `MARGIN_RENDERER=wgpu`, which falls back to Windows' software renderer (WARP), and shows a message box if that fails too. macOS and Linux don't include wgpu. |
| **Shell out to `git`** (not libgit2 or gitoxide) | Enterprise parity: hooks, GPG/SSH signing, credential helpers and SSO, `includeIf`, LFS, sparse/partial clone, fsmonitor. Reads use porcelain v2, `-z`, and explicit formats. `GIT_OPTIONAL_LOCKS=0` avoids fighting your terminal for locks. `GIT_TERMINAL_PROMPT=0` means git never hangs; `GIT_EDITOR=true` accepts default messages. `MARGIN_GIT` can pin a vendored git. |
| **One source for macOS, Windows, and Linux** | What differs by OS lives in two `platform` modules with the same functions everywhere: `core::platform` (config and home folders, console-free child processes for git and gh, the local time zone, cloud-sync folder locations, secrets in the Keychain / Credential Manager / Secret Service) and `app::platform` (reveal a folder, open a terminal or a link, folder pickers: native on macOS/Windows, zenity/kdialog on Linux). macOS adds the native menu bar. CI builds and tests all three. |
| **Background threads, no async runtime** | Each git or HTTP call runs on a short-lived thread and posts a `Msg`. Results carry a generation counter so stale diffs are dropped. Status, log, and refs load in parallel. |
| **Notes as files in the common git dir** | No database. Agents and humans can both edit. Notes are never committed by accident, survive checkouts, and are shared by worktrees. Writes are atomic (temp file + rename) and versioned (`if_version`). Appends retry on conflict. |
| **Markdown policy in core** | One `validate()` used by the GUI and MCP: no images, no HTML, and only http(s), mailto, relative, and `margin:` links. Size is capped at 256 KB. |
| **MCP hand-rolled** | Newline-delimited JSON-RPC in one file, no tokio. Supports protocol 2024-11-05 … 2025-11-25 and returns `structuredContent`. Started outside a repository or without git, it still answers the handshake and the tool list, and each tool call explains what's wrong (`--repo <path>`, install git); it exits 0 when its input ends. |
| **Two note kinds** | Commit notes are keyed by SHA and deliberately never move. Branch notes are keyed by branch name, so they survive rebase and amend for free, and the UI shows them on the branch's current tip (`tips` map: commit → local branches). No rewrite tracking or hooks are needed. |
| **Detached notes & renames** (`core::detached`) | Runs off the UI thread whenever refs or the note set change. Renames come from the reflog (`Branch: renamed refs/heads/a to refs/heads/b`; git moves a branch's reflog on rename, so chains are visible). A rename is followed only if it happened after the note was last written, which guards against reused names. Reachability uses `cat-file --batch-check` plus `for-each-ref --contains`, skipping commits already in the loaded log. |
| **Auto re-attach by `git patch-id --stable`** | Identifies "the same change" independent of SHA, message, and line offsets. Candidates are non-merge commits on refs that aren't ancestors of the old commit. Commits with the same author email and author timestamp are checked first (rebase, amend, and cherry-pick preserve them, so usually only 1 is checked). If none match, the 200 most recent candidates are scanned. Only a unique match moves a note; several matches become suggestions. A move onto an existing note appends to it. |
| **Re-attach journal** (`core::journal`) | `<notes>/reattach.json` records each automatic move: the moved text, the destination's previous content, and its version right after the move. Undo restores both sides exactly when the destination is untouched. If it was edited, undo keeps the edits and only restores the source. Undone `(from, to)` pairs go on a reject list that `auto_reattach` honours. The last 100 entries are kept. |
| **Command palette** | Commands are rebuilt from repo state each time the palette opens: refs, remotes, stashes, worktrees (`git worktree list --porcelain`), and git-flow config (one `git config --get-regexp`). Each command is a list of steps (pick, input, optional input, confirm) plus a closure that produces actions, so new commands are a few lines each. The fuzzy matcher favours word starts and consecutive runs. Groups are ordered by their best match. The macOS menu bar runs palette commands by name. |
| **Native git flow** (`core::gitflow`) | Plain git commands against the `gitflow.*` config keys. Merges use `--no-ff`. Each finish step is skipped if already done (`merge-base --is-ancestor`, existing tag), so a finish interrupted by a conflict resumes after you commit. The branch is deleted only after it's verified to be merged into every target. |
| **Conflicts** (`core::conflict`) | Parses the conflict markers in the working file (merge and diff3 styles, CRLF kept) into parts, assembles the chosen side, both, the ancestor or hand-written text, then stages it. Whole-file choices use `git checkout --ours/--theirs`; files one side deleted get keep/delete. "Start over" is `git checkout --conflict=diff3`. The editor opens by itself when an operation stops on conflicts, or when a repository is opened mid-conflict. |
| **Interactive rebase** (`core::rebase`) | The plan (pick, reword, edit, squash, fixup, drop, and order) becomes a todo file passed through `GIT_SEQUENCE_EDITOR`; messages are applied by `exec git commit --amend -F` lines, so no editor opens. `--autostash` keeps uncommitted work. Ranges with merges are refused. |
| **Stashes of some files** | `git stash push -- <paths>` records the whole staging area. When other files are staged, Margin builds the stash itself the way git does (index, worktree and untracked commits through temporary index files and `commit-tree`, stored with `git stash store`), so only the picked files go in. |
| **Insights** (`core::insights`) | One `git log --no-merges --use-mailmap --numstat -M` over the range (HEAD or all branches, capped at 50,000 commits), then aggregation in Rust by folder, person, file, week and weekday × hour, narrowed by a folder or a person. Lock files and generated code are left out by default. `code:` in the history search uses `git log -G` with the text escaped. |
| **Notes sync** (`core::sync`) | A two-way mirror between `.git/margin/notes` and a cloud-synced folder, with no provider APIs. `.sync-state.json` stores each file's hash at the last sync, tied to the remote folder, so each side's changes are known. On a conflict, both versions are merged into the note. Safety: a new or empty folder (or a wiped local dir) means union with no deletes; deletions move to `.trash/<time>/`; a pass deleting more than 3 notes and more than half is refused before anything changes (plan, then apply). Each tab's watcher thread runs passes when either side's stamp changes. MCP syncs at startup and after writes. |
| **Icons without an SVG crate** | GitHub and note icons are Octicons path data, rasterised by a ~150-line nonzero scanline filler (4×4 supersampling, cubic/quadratic/arc flattening) at the exact pixel size, then cached as tinted textures. resvg would add 1–2 MB. |
| **Syntax highlighting without a grammar engine** | `app::syntax` is a small hand-written tokenizer per language family (keywords, strings, comments, numbers), enough for diffs and code blocks in notes without the weight of syntect or tree-sitter. |
| **GitHub via GraphQL** | One aliased query returns the latest PR for up to 25 branches. A second query loads comments, checks and review threads only for the branch on screen. The PR head is the branch's upstream name, so a local name that differs from the remote still works. Review mode compares `base...head` like a pull request; a PR from a fork is fetched into `refs/margin/pr/<n>` without creating a branch. Reviews (with line comments) go in one `addPullRequestReview`; `@mentions` come from `mentionableUsers`. The token comes from env, `gh auth token`, or the OS secret store. |
| **ureq + rustls-platform-verifier** | Blocking HTTP (no runtime). Uses the OS trust store, so corporate TLS inspection works. Proxy settings come from the environment. |
| **Self-update** (`core::update`) | Checks the latest GitHub release, downloads the package for this install (the macOS app zip, the Windows installer, the AppImage or the Linux tar.gz) and checks its SHA-256. The swap happens after Margin quits: a helper replaces the `.app` on macOS, the installer runs silently on Windows, files are replaced on Linux. Installs owned by a package manager (`.deb`, Homebrew on Linux) are left to it. |

## Distribution

The Release workflow (`.github/workflows/release-macos.yml`) builds on macOS, Windows and Linux and publishes one GitHub release: a universal DMG and update zip, an Inno Setup installer and portable zip, an AppImage, a `.deb` and a tar.gz, each with a `.sha256`. It then updates the Homebrew tap (`wiggleji/homebrew-tap`: the macOS cask and the Linux formula) and opens a winget pull request (`wiggleji.Margin`). See [RELEASING.md](RELEASING.md).

## Known limitations

- By design, commit notes stay on the old SHA after a rebase or amend. They show up under *Detached notes* with a suggested new commit; re-attaching is manual.
- Rename detection needs reflogs (on by default in non-bare repos). With `core.logAllRefUpdates=false`, a terminal rename leaves the note detached; re-attach it from the list.
- The commit list loads 2000 commits per page ("Load more").
- Git status refreshes on focus and after operations; there's no filesystem watcher for the working tree yet (notes are watched).
- HTTPS auth needs a credential helper (osxkeychain, GCM, libsecret). Margin never prompts for passwords.
- The macOS app and the Windows installer aren't code-signed yet, so the first open shows the OS warning (Homebrew's cask clears it on macOS).

## Roadmap

1. **Signing:** Developer ID signing and notarization on macOS, code signing on Windows (the workflow already supports the Apple secrets).
2. **Git depth:** blame and file history, a filesystem watcher for the working tree.
3. **Notes:** optional team sharing through `refs/notes/margin` push/fetch, export a branch's notes into a PR description.
4. **Tasks:** API cards for Linear and Asana (today they're link-out only), Jira transitions (start ticket → "In Progress" when the branch is created), worklogs, and comments, behind the existing `TaskProvider` trait.
5. **MCP:** Streamable HTTP transport (for remote agents), resource subscriptions for notes, an opt-in "write" tool tier (stage/commit) behind a config flag.
