# Margin

A super-light git client (Rust + egui) for macOS. It adds **Markdown notes** on commits, branches, tasks, and the repo, plus **Jira** integration. An **MCP server** lets AI agents read and write the same notes you do.

![screenshot](docs/screenshot.png)

| | |
|---|---|
| GUI binary (release, stripped) | ~12.6 MB |
| MCP server binary | ~2.7 MB |
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
- Markdown with tables, task lists, and links. **Images, raw HTML, and unsafe link schemes are rejected** on every write, from both the GUI and MCP.
- Edit and preview modes, autosave, and clickable task checkboxes. Internal links like `margin:commit:abc123` and `margin:task:PROJ-42` navigate inside the app.
- Concurrency-safe: versioned writes. If an agent edits the note you're editing, you get a *Load theirs / Keep mine* choice.

**Tasks (Jira Cloud and Data Center)**
- The task key is inferred from the branch name (`feature/PROJ-42-login` → `PROJ-42`), or you can link it explicitly.
- The task card shows status, assignee, priority, and description. "My tasks" can create a branch from a ticket. The commit box offers an "Insert PROJ-42" button.

**MCP (for agents)**: 14 tools. `get_context`, `git_status`, `git_log`, `git_show`, and `list_branches` read git. `read_note`, `write_note`, `append_note`, `list_notes`, `search_notes`, and `link_task` work on notes. `get_task`, `search_tasks`, and `my_tasks` read Jira. Git access is **read-only** for agents.

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

## Where notes live

`<repo>/.git/margin/notes/`. Notes are plain `.md` files, never committed, and shared across worktrees. They're easy to grep, back up, or sync.

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for design decisions and the roadmap.
