# Margin

A super-light git client (Rust + egui) for macOS. It adds **Markdown notes** on commits, branches, tasks, and the repo, **GitHub PR status**, and **task-tracker** links (Jira API; Linear, Asana and others via URL template). An **MCP server** lets AI agents read and write the same notes you do.

![screenshot](docs/screenshot.png)

![command palette](docs/screenshot-palette.png)

| | |
|---|---|
| GUI binary (release, stripped) | ~13.2 MB |
| MCP server binary | ~2.8 MB |
| Idle CPU | ~0 (repaints only on input or when a note changes) |
| Private memory, small repo | ~40 MB |

## What it does

**Settings (⌘,)**: sections for Appearance, Margin Pro, Notes sync, Fetch, Tasks, and Diffs, each setting with a short explanation. Changes save automatically.

**Themes** (Settings → Appearance, applied right away): Margin (default), Material, Material Darker, Material Palenight, Material Ocean, Material Lighter, Solarized Dark, and Solarized Light. Each is built from its palette's base colours; diff backgrounds, selection, and graph lanes are mixed from them.

**Command palette (⌘P)**: every action, fuzzy-searchable and grouped with icons. The groups are *In progress* (continue/abort), Repository, Remote, Commit, Branch, Tag, Stash, Worktree, Git Flow, Submodule, GitHub, Notes, and Go to. Commands that need input ask for it inside the palette: pick a branch, type a name, or confirm. ↑/↓ move, ↵ runs, and Esc steps back. With an empty search, the **5 most recently run commands** are listed first under *Recent* (remembered across restarts).

**Menu bar (macOS):** Margin, File (Open Folder… ⌘O, Open Recent, Save Note ⌘S, Close Tab ⌘W), Edit (undo, copy/paste, Find Commits… ⌘F), Repository (fetch, pull, push, commit, stash, branch, merge, rebase, tag, worktree, Git Flow, Refresh ⌘R), Navigate (Command Palette ⌘P, Back ⌘[, Forward ⌘], Go to…), Window (tabs ⌃Tab / ⌃⇧Tab), and Help.

| Group | Commands |
|---|---|
| Remote | fetch (all or one remote), pull, pull with rebase, push, force push (with lease), add, rename, remove, or set-URL a remote, delete a remote branch |
| Commit | commit, amend, stage/unstage all, discard all, revert, cherry-pick, reset (soft/mixed/hard) |
| Branch | new, checkout (local or remote), merge, rebase onto, rename (note follows), delete, force delete, set upstream, push to remote, link task |
| Tag | create (lightweight or annotated), checkout, delete, push one or all, delete on remote |
| Stash | stash, stash with message, pop latest, apply, pop, drop |
| Worktree | open, add for an existing branch, add with a new branch (folder defaults to `../<repo>-<branch>`), remove, force remove, prune |
| Git Flow | initialize; Feature, Bugfix, Release, Hotfix, Support: start, finish (release and hotfix are tagged), publish |
| Other | submodule update/sync; continue/skip/abort a merge, rebase, cherry-pick, or revert; open or create a PR, open the repo on GitHub; open notes; go to a branch, tag, or commit |

**Git Flow without the extension.** It's implemented with plain git and uses the standard `gitflow.*` config, so it's compatible with repos set up by `git flow init`. Finish is idempotent: after a merge conflict, resolve it, commit, and run Finish again. A hotfix goes into an open release branch if there is one, like git-flow does.

**Toolbar** (the only bar, at the top):

```
[repo ▾] [branch ▾] [‹][›]        [Commands ⌘P] [Search commits…]        status · Undo  [Fetch][▾] [Pull ↓n] [Push ↑n]
```
- **Tab strip:** a row of tabs above the toolbar, one per open directory. The active tab blends into the toolbar, × shows on hover, middle-click closes, and **+** opens a menu: **Open folder…** and up to 10 recent directories.
- **repo ▾ (directory tabs)**: each opened repository or worktree is its own tab, and the name shows its position (`demo 1/3`). The dropdown lists open tabs (click to switch, × to close), then recent repositories, Open repository…, Reveal in Finder, Open in Terminal, and Close tab. ⌘P → **Change directory…** picks from open tabs and recent directories. Worktrees are in the sidebar instead (see below). Ctrl+Tab / Ctrl+Shift+Tab (or ⌘⇧] / ⌘⇧[) cycle tabs. Each tab keeps its own selection, history, notes editor, and PR status, and open tabs are restored on restart. A tab only loads when you first switch to it. Results of background work for a hidden tab never touch the visible one: the tab is refreshed when you come back to it.
- **branch ▾**: switch branch, New branch…, more branch commands. A merge or rebase in progress shows as a red badge (click it for continue/abort).
- **‹ ›**: back and forward through what you selected (commits, working directory), like a browser. ⌘[ / ⌘] or the mouse back/forward buttons also work. Commits that no longer exist (rebased away) are skipped.
- **[Fetch][▾]** fetches all remotes (with prune). **▾** opens a menu: Fetch now, Fetch a remote…, an auto-fetch toggle, **Fetch settings…**, and the last fetch time. The button reads *Fetch ⟳* while auto-fetch is on.
- **Auto-fetch** (Settings → Fetch): enable/disable and set the interval in minutes (1–1440; presets 5/15/30/60). It's off by default. It runs in the background without blocking your own git operations, and waits if one is running. It's quiet on success (the graph and PR status just update) and shows a message only on failure. There's no polling: the app wakes once when the next fetch is due. Settings are saved in `settings.json` next to `config.json`.
- **Status messages, errors, and Undo** appear just left of the Fetch/Pull/Push buttons. Stash and Refresh are in ⌘P; refresh also happens on window focus and ⌘R.

**Git (Sublime Merge–style)**
- Commit graph across all branches, remotes, and tags, virtualised for large histories, with a filter box.
- Working directory: stage or unstage files, **per-hunk** stage, unstage, and discard, and commit or amend (⌘↵).
- Commit details: message, parents, and a full diff (large diffs are virtualised and capped at 8 MB).
- Branches: checkout, create, delete, merge. Remotes: tracking checkout. Tags, stashes (apply, pop, drop).
- **Ahead/behind badges:** each branch with an upstream shows a right-aligned pill, `2↓ │ 1↑` (behind, then ahead, like Sublime Merge). A dimmed `gone` pill means the upstream was deleted on the remote. Hover for details. Counts are as of the last fetch, so auto-fetch keeps them current. Long names truncate before the task key, note, PR, and badge, so those stay visible.
- **Branch groups:** branches that share a prefix (`feature/`, `hotfix/`, `release/`, or any custom one like `projectA/`) are grouped once 2 or more share it. A prefix with a single branch stays flat (`projectA/login-page`). This applies at every level, and single-child chains are compacted (`projectB/feature/`). The top level is alphabetical; **inside a group, branches are most-recently-committed first**. Groups show a count and start open only if they contain the current or focused branch. Indentation is one shallow 12pt step per level, and the ● current-branch marker sits in the triangle column so names stay aligned. The same grouping applies under each remote and in the toolbar's branch switcher (as submenus).
- **Branches in the history (eye):** each branch row in the sidebar has an eye at the right. By default only the **major branches** are in the graph: the current branch and its upstream, and main / master / develop / trunk / staging / production (locally and on remotes). Click the eye to show or hide any branch. The history header says e.g. "3 of 12 branches shown" with **Show all** (or *Major branches only* to go back). Right-click → *Show only this branch in history*; ⌘P has the same commands plus *Hide tags*. Saved per repository (shared by its worktrees).
- **Ahead/behind counts are buttons:** click **↓** to pull (for a branch that isn't checked out: fast-forward it from its upstream) and **↑** to push it.
- The **whole branch row** is the click target: click shows its tip, double-click checks it out.
- Branches whose upstream is **gone** (deleted on the remote) are dimmed.
- **Worktrees** are listed in the sidebar like branches, by the branch each has checked out (● marks this tab's; on the right, only the part of the folder name that isn't already in the branch or repo name). The section only shows when the repo has more than one worktree. Click a worktree to show its branch tip, double-click to open it in a tab; right-click to reveal, open a terminal, copy the path, or remove it. In the branch list, a folder icon marks branches checked out in another worktree; double-clicking one opens that worktree instead of failing to check it out.
- **Search commits** with words and filters, combined with AND: `commit:3c7d469`, `author:sungmin` (name or email), `title:"merge pull"`, `branch:develop`, `tag:26.47`, `task:SER-6114`, `note:"rollout plan"` (title of a commit note, or of a branch note on the branch's tip), `after:2026-11-01` / `before:2026-11-15` (also `2026-11`, `7d`, `2w`, `3m`, `today`, `yesterday`; dates in your machine's time zone, `after:` includes the day, `before:` excludes it). Hover the box for the list. While typing, a dropdown **autocompletes** filter names and values (authors, branches, tags, task keys, note titles, dates): ↑/↓ to pick, Tab or ↵ to insert, Esc to close.
- **↑ / ↓** move through the history; the list only scrolls when the selection reaches the top or bottom edge.
- **History** loads 2,000 commits at a time; scrolling to the bottom loads the next batch automatically.
- **Commit details:** metadata first (SHA, author and committer with times in your machine's time zone, parents, the branches containing the commit, files changed with +/−, and task keys), then the commit title and full message. It scrolls together with the diff as one page; the diff toolbar then sticks to the top and shows the commit's short SHA and title, so nothing ever resizes while you scroll. While the next commit loads, the current one stays on screen (a small spinner shows in the toolbar).
- **History rows** have two lines: the subject on the left with branch and tag labels on the right, then the author with note markers (commit note, branch note title) on the left and PR status, the task key, and time on the right. A task key is shown only on its **newest** commit (and one per row, with "+N" for more), so a ticket spanning many commits doesn't repeat. While searching, it moves to the newest matching commit.
- **Working directory:** the commit box on top, then every changed file under **Unstaged** (including untracked files) and **Staged**, with merge conflicts first. Click a file header to expand its diff; each file has Stage / Unstage / Discard buttons (double-click stages or unstages), hunks have their own buttons, and each section has *Stage all* / *Discard all* / *Unstage all*.
- **Expanded by default** (Settings → Diffs): commits and staged files expanded, unstaged files collapsed. Expanding or collapsing a file by hand is kept while you work.
- **Focus on the diff:** the ⛶ button on the diff toolbar, **⌘⇧↩**, Navigate → *Focus on Diff*, or ⌘P gives the diff the whole window (branches, history, and left-side notes hidden); the same again brings them back. Panels can't be dragged over each other: the commit details stop short of the left edge and the sidebar has a minimum and maximum width, so every edge stays grabbable.
- **Diffs** can be shown **inline** or **side by side** (icons above the diff; remembered). Each file header shows its status (A / M / D / R), and multi-file diffs have *Collapse all* / *Expand all*. Long lines **wrap** between words (no sideways scrolling). **Select code by dragging** over it (line numbers and headers aren't included; in side by side, one side at a time; double-click selects a line) and copy with ⌘C or right-click → Copy. Tabs are kept and wrapped lines are copied as one line. Inside changed lines, **the words that changed are highlighted**. **Ignore whitespace** hides indentation-only changes (`git diff -w`; hunk staging is off while it's on). **↑ / ↓** (or ⌥↑ / ⌥↓) glide to the previous / next change, which flashes.
- Commit menu: new branch here, detached checkout, cherry-pick, revert, reset (soft, mixed, hard).
- Fetch, pull, and push run in the background. The UI refreshes when the window regains focus.

**Notes**
- Notes attach to `repo`, `branch:<name>`, `commit:<sha>`, and `task:<KEY>`.
- **Two kinds of git notes:**
  - **Branch note:** bound to the branch name. It's shown on the branch's **top commit** (a chip in the graph, and the default tab when you select the tip). It follows the branch through rebase, amend, and new commits. Use it for ongoing work.
  - **Commit note:** bound to one exact SHA. After a rebase or amend it stays with the old commit. Use it for facts about that exact snapshot (benchmarks, review of that revision).
- **Branch renames are followed.** Renaming in Margin (right-click → Rename…) or in a terminal (`git branch -m`, read from the reflog, including chains) moves the branch note and task link to the new name.
- **Automatic re-attach (git patch-id).** When a commit note's commit is rewritten without changing its diff (rebase, cherry-pick, message-only amend), the note moves to the new commit automatically, and the toolbar message says so. It only moves on a unique match. If the change itself was edited (amend with new content, squash), the note stays detached for you to decide. Turn it off with `"auto_reattach": false`.
- **Undo for automatic re-attach.** Every automatic move can be reverted from the toolbar message ("Undo"), the sidebar's **Re-attached** list, or the banner on the note itself. Undo puts the note back on the old commit and restores the destination exactly, including a note that was there before a merge. If you edited the re-attached note afterwards, your edits are kept. An undone move is never applied automatically again. **Keep** confirms a move and clears it from the list.
- **Detached notes.** Commit notes whose commit is no longer on any branch, tag, or remote, and branch notes whose branch was deleted, are listed in the sidebar. Open one to see why it's detached. If the same change exists in several commits, each one is a one-click target. **Re-attach…** suggests the rewritten commit (patch-id match, else same subject), or you can delete it. Moving onto an existing note appends instead of overwriting.
- Markdown with tables, task lists, and links. **Images, raw HTML, and unsafe link schemes are rejected** on every write, from both the GUI and MCP.
- **Notes panel:** drag the grip on top to resize it, double-click the grip (or click ▾) to collapse it to its tab row, and click ⇤ / ⇥ to move it under the branches and history (one wide panel spanning both) or under the commit details (also ⌘P → *Move notes…*). These choices are remembered. The panel keeps the height you set (switching notes never resizes it); an empty note shows a one-line **Write note** button. The **trash** button deletes the shown note after a confirmation; it's moved to `.git/margin/notes/.trash/`, so it can be recovered.
- **Preview, Edit, and Split** (editor and live preview side by side). Notes open in Preview. Edit and Split have a formatting toolbar: heading, **bold** (⌘B), *italic* (⌘I), code, link (⌘K), bullet, checklist, and quote. It wraps the selection, or toggles the prefix on the selected lines.
- Autosave, and clickable task checkboxes. Internal links like `margin:commit:abc123` and `margin:task:PROJ-42` navigate inside the app.
- Concurrency-safe: versioned writes. If an agent edits the note you're editing, you get a *Load theirs / Keep mine* choice.

**GitHub pull requests** (github.com and Enterprise Server). Display-only, so everything links out to GitHub.
- Merge readiness: *Ready to merge*, *Changes requested*, *Review required*, *Merge conflicts*, *Behind base*, *Draft*, *Merged*…
- CI status with each check (failures first, click to open the run).
- Comments: conversation comments, reviews, and unresolved review threads (`file:line`), newest first.
- Shown as a `#123` chip on the branch tip and in the sidebar, plus a PR card on the branch tab. One GraphQL request covers all local branches. It reloads on window focus (at most once a minute) and after fetch, pull, or push.

**Tasks (Jira Cloud and Data Center)**
- The task key is inferred from the branch name (`feature/PROJ-42-login` → `PROJ-42`), or you can link it explicitly.
- The task card shows status, assignee, priority, and description. "My tasks" can create a branch from a ticket. The commit box offers an "Insert PROJ-42" button.
- **Commit box:** a summary line (↵ moves to the description; a counter turns amber past 50 characters and red past 72) and an optional description. ⌘↵ commits from either. Commit details show the full message, selectable and scrollable when long.

**MCP (for agents)**: 18 tools. `get_context`, `git_status`, `git_log`, `git_show`, and `list_branches` read git. `read_note`, `write_note`, `append_note`, `list_notes`, `search_notes`, and `link_task` work on notes. `get_task`, `search_tasks`, and `my_tasks` read Jira. `get_pr` gives agents the PR's readiness, failing checks, and review comments, so they can address feedback. `list_detached_notes`, `move_note`, and `undo_reattach` let agents tidy notes after a rebase. Git access is **read-only** for agents.

## Install / release

Margin ships as a signed, notarized universal `.dmg`. See [docs/RELEASING.md](docs/RELEASING.md) to build one (`scripts/package-macos.sh`) or release through GitHub Actions on a version tag.

## Build

Requires Rust 1.88+ and `git` on `PATH`.

```bash
cargo build --release
```

```bash
./target/release/margin /path/to/repo
```

## Connect an agent

Add Margin as a stdio MCP server in your agent's MCP configuration:

```json
{
  "mcpServers": {
    "margin": {
      "command": "/path/to/margin/target/release/margin-mcp",
      "args": ["--repo", "/path/to/repo"]
    }
  }
}
```
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

Without a Jira API config, set **Settings → Tasks → Ticket link**: a Jira site such as `https://acme.atlassian.net` (keys open `/browse/KEY`), or a template with `{key}` such as `https://linear.app/acme/issue/{key}`. Task keys in branch names and commit messages then open the ticket with one click (↗ next to the key in commit details, and in the Task tab). Look-alikes such as `UTF-8`, `SHA-256`, `ISO-8601`, and `CVE-…` aren't treated as task keys, and lowercase keys (`ops-42` in a branch name) need two or more digits and a prefix that isn't an everyday branch word, so `topic-1`, `release-2`, or `feature-123` don't count.

## Where notes live

`<repo>/.git/margin/notes/`. Notes are plain `.md` files, never committed, and shared across worktrees. They're easy to grep, back up, or sync.

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for design decisions and the roadmap. Icons: [Primer Octicons](https://github.com/primer/octicons) (MIT), see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

## Notes sync (iCloud Drive / Dropbox / Google Drive)

Sync notes between your computers through a folder your cloud provider already syncs. There's no account and no API: **Settings → Notes sync**, pick the folder (iCloud Drive, Dropbox, Google Drive, and OneDrive are detected), and tick **Sync notes for <repo>**.

- **Where notes live:** they stay in `.git/margin/notes` and are mirrored two-way with `<folder>/Margin/<repo-id>/`. The repo id comes from the remote URL (ssh and https forms match), so clones in differently named folders find each other.
- **Edits:** they sync within a second or two while Margin runs. Agent edits made through MCP are mirrored too, even when the app is closed. If both computers edit the same note, both versions are kept (merged under a marker).
- **Safety:** pointing sync at a different or empty folder is treated as a first sync, so nothing gets deleted. Deleted notes go to `.trash/` instead of disappearing. A pass that would delete most of your notes is refused. iCloud placeholders that aren't downloaded yet are never mistaken for deletions.
- **Limit:** **Free syncs 1 repository; Margin Pro syncs unlimited.** Repositories over the limit pause, keep working locally, and catch up when Pro is active.

## Margin Pro

The git client, notes, agents (MCP), tabs, and Git Flow are free. **Margin Pro** ($29 one-time, **$19 launch price**, including a year of updates) unlocks:

- **GitHub PR status:** readiness, CI checks, and review comments.
- **Task trackers:** Jira ticket details, My tasks, and creating a branch from a ticket.
- **Notes sync for more than one repository.**

New installs get a **14-day trial**. After that, Pro features show a lock hint (Enter licence key / Buy Margin Pro), and the MCP tools `get_pr`, `get_task`, `search_tasks`, and `my_tasks` are refused. Activate in **Settings → Margin Pro** or with ⌘P → *Enter Margin Pro licence key…*.

Keys are verified **offline** (Ed25519 signature, no licence server, works on locked-down networks). A key covers every version built up to its *updates until* date.

**Issuing keys** (maintainer):

```bash
cargo run -p margin-license -- keygen ~/secure/margin
```

```bash
MARGIN_LICENSE_PUBKEY=<hex from keygen> cargo build --release
```

```bash
cargo run -p margin-license -- issue ~/secure/margin/margin-license.private --name "Kim Dev" --email kim@example.com --years 1
```

Keep `margin-license.private` secret and backed up (`*.private` is git-ignored). Builds without `MARGIN_LICENSE_PUBKEY` accept development keys only (`issue --dev`), and release builds warn about it. Set `PURCHASE_URL` in `crates/margin-app/src/pro.rs` to your store's checkout page. Store and pricing plan: [docs/SELLING.md](docs/SELLING.md).

## License

MIT — see [LICENSE](LICENSE). Third-party notices: [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
