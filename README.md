# Margin

A super-light git client (Rust + egui) for macOS. It adds **Markdown notes** on commits, branches, tasks, and the repo, **GitHub PR status**, and **task-tracker** links (Jira API; Linear, Asana and others via URL template). An **MCP server** lets AI agents read and write the same notes you do.

![screenshot](docs/screenshot.png)

| | |
|---|---|
| GUI binary (release, stripped) | ~12.9 MB |
| MCP server binary | ~2.8 MB |
| Idle CPU | ~0 (repaints only on input or when a note changes) |
| Private memory, small repo | ~40 MB |

## What it does

**Git (Sublime Merge–style)**
- Commit graph across all branches, remotes, and tags, virtualised for large histories, with a filter box.
- Working directory: stage or unstage files, **per-hunk** stage, unstage, and discard, and commit or amend (⌘↵).
- Commit details: message, parents, and a full diff (large diffs are virtualised and capped at 8 MB).
- Branches: checkout, create, delete, merge. Remotes: tracking checkout. Tags, stashes (apply, pop, drop).
- Commit menu: new branch here, detached checkout, cherry-pick, revert, reset (soft, mixed, hard).
- Fetch, pull, and push run in the background. The UI refreshes when the window regains focus.

**Notes**
- Notes attach to `repo`, `branch:<name>`, `commit:<sha>`, and `task:<KEY>`.
- **Two kinds of git notes:**
  - **Branch note:** bound to the branch name. It's shown on the branch's **top commit** (a chip in the graph, and the default tab when you select the tip). It follows the branch through rebase, amend, and new commits. Use it for ongoing work.
  - **Commit note:** bound to one exact SHA. After a rebase or amend it stays with the old commit. Use it for facts about that exact snapshot (benchmarks, review of that revision).
- **Branch renames are followed.** Renaming in Margin (right-click → Rename…) or in a terminal (`git branch -m`, read from the reflog, including chains) moves the branch note and task link to the new name.
- **Automatic re-attach (git patch-id).** When a commit note's commit is rewritten without changing its diff (rebase, cherry-pick, message-only amend), the note moves to the new commit automatically, and the status bar says so. It only moves on a unique match. If the change itself was edited (amend with new content, squash), the note stays detached for you to decide. Turn it off with `"auto_reattach": false`.
- **Undo for automatic re-attach.** Every automatic move can be reverted from the status bar ("Undo"), the sidebar's **Re-attached** list, or the banner on the note itself. Undo puts the note back on the old commit and restores the destination exactly, including a note that was there before a merge. If you edited the re-attached note afterwards, your edits are kept. An undone move is never applied automatically again. **Keep** confirms a move and clears it from the list.
- **Detached notes.** Commit notes whose commit is no longer on any branch, tag, or remote, and branch notes whose branch was deleted, are listed in the sidebar. Open one to see why it's detached. If the same change exists in several commits, each one is a one-click target. **Re-attach…** suggests the rewritten commit (patch-id match, else same subject), or you can delete it. Moving onto an existing note appends instead of overwriting.
- Markdown with tables, task lists, and links. **Images, raw HTML, and unsafe link schemes are rejected** on every write, from both the GUI and MCP.
- Edit and preview modes, autosave, and clickable task checkboxes. Internal links like `margin:commit:abc123` and `margin:task:PROJ-42` navigate inside the app.
- Concurrency-safe: versioned writes. If an agent edits the note you're editing, you get a *Load theirs / Keep mine* choice.

**GitHub pull requests** (github.com and Enterprise Server). Display-only, so everything links out to GitHub.
- Merge readiness: *Ready to merge*, *Changes requested*, *Review required*, *Merge conflicts*, *Behind base*, *Draft*, *Merged*…
- CI status with each check (failures first, click to open the run).
- Comments: conversation comments, reviews, and unresolved review threads (`file:line`), newest first.
- Shown as a `#123` chip on the branch tip and in the sidebar, plus a PR card on the branch tab. One GraphQL request covers all local branches. It reloads on window focus (at most once a minute) and after fetch, pull, or push.

**Tasks (Jira Cloud and Data Center)**
- The task key is inferred from the branch name (`feature/PROJ-42-login` → `PROJ-42`), or you can link it explicitly.
- The task card shows status, assignee, priority, and description. "My tasks" can create a branch from a ticket. The commit box offers an "Insert PROJ-42" button.

**MCP (for agents)**: 18 tools. `get_context`, `git_status`, `git_log`, `git_show`, and `list_branches` read git. `read_note`, `write_note`, `append_note`, `list_notes`, `search_notes`, and `link_task` work on notes. `get_task`, `search_tasks`, and `my_tasks` read Jira. `get_pr` gives agents the PR's readiness, failing checks, and review comments, so they can address feedback. `list_detached_notes`, `move_note`, and `undo_reattach` let agents tidy notes after a rebase. Git access is **read-only** for agents.

## Build

Requires Rust 1.88+ and `git` on `PATH`.

```bash
cargo build --release
```

```bash
./target/release/margin /path/to/repo
```

## Connect an agent

```bash
claude mcp add margin -- /path/to/margin/target/release/margin-mcp --repo /path/to/repo
```

Any MCP client works the same way (stdio transport): command `margin-mcp`, args `["--repo", "<path>"]`.
Agent notes are signed with the client's name, or with `MARGIN_AGENT_NAME` if set.

## Configure Jira

Copy `docs/config.example.json` to `~/Library/Application Support/Margin/config.json`.

- **Jira Cloud:** set `"flavor": "cloud"` and your `email`. The token is an Atlassian API token.
- **Jira Data Center/Server:** set `"flavor": "data_center"`. The token is a Personal Access Token.

The token is never stored in the config file. It's read from `$MARGIN_JIRA_TOKEN`, or from the macOS Keychain:

```bash
security add-generic-password -s margin-jira -a you@your-company.com -w
```

TLS uses the OS trust store, so corporate CAs installed in the Keychain work. `HTTPS_PROXY` and `NO_PROXY` are honoured.

## Configure GitHub

Nothing is needed if you use the GitHub CLI: Margin reuses `gh auth token`. Otherwise set `GITHUB_TOKEN`, or add the token to the Keychain:

```bash
security add-generic-password -s margin-github -a github.com -w
```

For GitHub Enterprise Server, list your host in `github.enterprise_hosts`. If you use SSH host aliases from `~/.ssh/config` (`git@github-work:org/repo`), map them in `github.host_aliases`.

## Link other task trackers

Without a Jira API config, set `task_url_template` (for example `https://linear.app/acme/issue/{key}`). Task keys found in branch names then open in that tracker with one click.

## Where notes live

`<repo>/.git/margin/notes/`. Notes are plain `.md` files, never committed, and shared across worktrees. They're easy to grep, back up, or sync.

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for design decisions and the roadmap. Icons: [Primer Octicons](https://github.com/primer/octicons) (MIT), see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
