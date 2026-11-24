# Margin guide

Everything Margin does, in detail. For a quick start see the [README](../README.md).

## Features

**Settings (⌘,)**: sections for Appearance, Accounts, Notes sync, Fetch, Tasks, and Diffs, each setting with a short explanation. Changes save automatically.

**Text size** (Settings → Appearance), applied right away: **Code and notes** (11–20 pt; diffs, notes, and commit messages; overrides `font_size` in config.json) and **Interface** (12–15 pt; lists, sidebar, toolbar, menus), kept separate so a bigger reading size doesn't reflow the whole window.

**Fonts** (Settings → Appearance): any installed font for the **Interface** and for **Code and notes** (fixed-width fonts listed first), with a filter. Each block has its font and its size together, and Code and notes shows a preview. Characters a font lacks (Korean in a Latin code font, emoji) still come from the built-in and system fonts.

**Settings** open with ⌘, (Margin menu) or Ctrl+, on Linux and Windows, and close with Esc. The window keeps one size whichever section is shown. Checkboxes, radio buttons, and buttons have a visible edge in every theme, light ones included.

**Themes** (Settings → Appearance, applied right away): Margin (default), Material, Material Darker, Material Palenight, Material Ocean, Material Lighter, Solarized Dark, and Solarized Light; plus bright **Paper** and **One Light**, and the warm, low-glare **Gruvbox Light** and **Sepia** for long reading. Each is built from its palette's base colours; diff backgrounds, selection, and graph lanes are mixed from them.

**Command palette (⌘P)**: every action, fuzzy-searchable and grouped with icons. The groups are *In progress* (continue/abort), Repository, Remote, Commit, Branch, Tag, Stash, Worktree, Git Flow, Submodule, GitHub, Notes, and Go to. Commands that need input ask for it inside the palette: pick a branch, type a name, or confirm. ↑/↓ move, ↵ runs, and Esc steps back (shown at the bottom of the palette). The letters you typed are highlighted in each title (the whole word when it's there), keyboard shortcuts show as key caps (Ctrl R, ⌘ ⇧ ↩) apart from plain descriptions, and commands that delete or throw work away (Delete…, Force delete…, Drop stash…, Remove…) are red. With an empty search, the **5 most recently run commands** are listed first under *Recent* (remembered across restarts).

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
- **Toolbar:** the repository and branch buttons show their icon and ▾ (they open menus; long branch names are shortened in the middle, full on hover). The command palette button and the search box sit between the left and right groups and shrink when the window is narrow ("Ctrl+P", a shorter search hint, then no palette button: Ctrl+P still opens it), so nothing overlaps.
- **repo ▾ (directory tabs)**: each opened repository or worktree is its own tab, and the name shows its position (`demo 1/3`). The dropdown lists open tabs (click to switch, × to close), then recent repositories, Open repository…, Reveal in Finder, Open in Terminal, and Close tab. ⌘P → **Change directory…** picks from open tabs and recent directories. Worktrees are in the sidebar instead (see below). Ctrl+Tab / Ctrl+Shift+Tab (or ⌘⇧] / ⌘⇧[) cycle tabs, and ⌘1…⌘8 jump to a tab (⌘9: the last). Drag a tab to reorder it. When tabs don't fit, they shrink evenly (names shortened in the middle), then the rest go into a **⋯ +N** menu, with the active tab always shown. Two open folders with the same name are labelled with their parent (`app · work`, `app · oss`). Each tab keeps its own selection, history, notes editor, and PR status, and open tabs are restored on restart. A tab only loads when you first switch to it. Results of background work for a hidden tab never touch the visible one: the tab is refreshed when you come back to it.
- **branch ▾**: switch branch, New branch…, more branch commands. A merge or rebase in progress shows as a red badge (click it to resolve its conflicts, or for continue/abort).
- **‹ ›**: back and forward through what you selected (commits, working directory), like a browser. ⌘[ / ⌘] or the mouse back/forward buttons also work. Commits that no longer exist (rebased away) are skipped.
- **[Fetch][▾]** fetches all remotes (with prune). **▾** opens a menu: Fetch now, Fetch a remote…, an auto-fetch toggle, **Fetch settings…**, and the last fetch time. The button reads *Fetch ⟳* while auto-fetch is on.
- **Auto-fetch** (Settings → Fetch): enable/disable and set the interval in minutes (1–1440; presets 5/15/30/60). It's off by default. It runs in the background without blocking your own git operations, and waits if one is running. It's quiet on success (the graph and PR status just update) and shows a message only on failure. There's no polling: the app wakes once when the next fetch is due. Settings are saved in `settings.json` next to `config.json`.
- **Status messages, errors, and Undo** appear just left of the Fetch/Pull/Push buttons. Stash and Refresh are in ⌘P; refresh also happens on window focus and ⌘R.

**Git (Sublime Merge–style)**
- Commit graph across all branches, remotes, and tags, virtualised for large histories, with a filter box.
- Working directory: stage or unstage files, **per-hunk** stage, unstage, and discard, and commit or amend (⌘↵).
- Commit details: message, parents, and a full diff (large diffs are virtualised and capped at 8 MB).
- Branches: checkout, create, delete, merge. Remotes: tracking checkout. Tags, stashes (see below).
- **Ahead/behind badges:** each branch with an upstream shows a right-aligned pill, `2↓ │ 1↑` (behind, then ahead, like Sublime Merge). A dimmed `gone` pill means the upstream was deleted on the remote. Hover for details. Counts are as of the last fetch, so auto-fetch keeps them current. Long names truncate before the task key, note, PR, and badge, so those stay visible.
- **Branch groups:** branches that share a prefix (`feature/`, `hotfix/`, `release/`, or any custom one like `projectA/`) are grouped once 2 or more share it. A prefix with a single branch stays flat (`projectA/login-page`). This applies at every level, and single-child chains are compacted (`projectB/feature/`). The top level is alphabetical; **inside a group, branches are most-recently-committed first**. Groups show a count and start open only if they contain the current or focused branch. Indentation is one shallow 12pt step per level, and the ● current-branch marker sits in the triangle column so names stay aligned. The same grouping applies under each remote and in the toolbar's branch switcher (as submenus).
- **Filter the sidebar:** the box at the top of the sidebar (it stays put while the list scrolls) narrows branches, remote branches, and tags to the names containing what you type, as flat lists with full names (headings say e.g. "BRANCHES (3 of 324)"); other sections step aside meanwhile. Esc or × clears it.
- **Branches in the history (eye):** each branch row in the sidebar has an eye at the right. By default only the **major branches** are in the graph: the current branch and its upstream, and main / master / develop / trunk / staging / production (locally and on remotes). Click the eye to show or hide any branch. Folders (`feature/`, `hotfix/`) and remotes (`origin`) have an eye too: it's open while any branch inside is shown (fainter when only some are), and a click hides them all, or shows them all when none are. The history header says e.g. "3 of 12 branches shown" with **Show all** (or *Major branches only* to go back). Right-click → *Show only this branch in history*; ⌘P has the same commands plus *Hide tags*. Saved per repository (shared by its worktrees).
- **Ahead/behind counts are buttons:** click **↓** to pull (for a branch that isn't checked out: fast-forward it from its upstream) and **↑** to push it.
- The **whole branch row** is the click target: click shows its tip, double-click checks it out.
- Branches whose upstream is **gone** (deleted on the remote) are dimmed.
- **Worktrees** are listed in the sidebar like branches, by the branch each has checked out (● marks this tab's; on the right, only the part of the folder name that isn't already in the branch or repo name). The section only shows when the repo has more than one worktree. Click a worktree to show its branch tip, double-click to open it in a tab; right-click to reveal, open a terminal, copy the path, or remove it. In the branch list, a folder icon marks branches checked out in another worktree; double-clicking one opens that worktree instead of failing to check it out.
- **Search commits** with words and filters, combined with AND: `commit:3c7d469`, `author:sungmin` (name or email), `title:"merge pull"`, `branch:develop`, `tag:26.47`, `task:SER-6114`, `note:"rollout plan"` (title of a commit note, or of a branch note on the branch's tip), `code:"fn login"` (commits that added or removed a line containing that text, on any branch, exact case: `git log -G`; it runs in the background, and the list says if more such commits are older than what's loaded or on hidden branches), `after:2026-11-01` / `before:2026-11-15` (also `2026-11`, `7d`, `2w`, `3m`, `today`, `yesterday`; dates in your machine's time zone, `after:` includes the day, `before:` excludes it). Hover the box for the list. While typing, a dropdown **autocompletes** filter names and values (authors, branches, tags, task keys, note titles, dates): ↑/↓ to pick, Tab or ↵ to insert, Esc to close.
- **↑ / ↓** move through the history; the list only scrolls when the selection reaches the top or bottom edge.
- **History** loads 2,000 commits at a time; scrolling to the bottom loads the next batch automatically.
- **Commit details:** metadata first (SHA with a **Copy ▾** menu: full SHA, short SHA, message, or short SHA and title; author name with the email and time quieter, plus how long ago; committer when different; each parent with its title; the branches containing the commit as clickable chips, "+N more" expanding in place; files changed with +/− and how many were added, deleted, or renamed, and task keys), then the commit title and full message. It scrolls together with the diff as one page; the diff toolbar then sticks to the top and shows the commit's short SHA and title, so nothing ever resizes while you scroll. While the next commit loads, the current one stays on screen (a small spinner shows in the toolbar).
- **History rows** have two lines: the subject on the left (ending in “…” when cut) with branch and tag labels on the right (at most 40% of the row, then “+N”; branches hidden with the eye, and hidden tags, get no label, but the row's tooltip lists every ref), then the author with note markers (commit note, branch note title) on the left and PR status, the task key, and time on the right. A task key is shown only on its **newest** commit (and one per row, with "+N" for more), so a ticket spanning many commits doesn't repeat. While searching, it moves to the newest matching commit.
- **Find in a diff:** with the pointer over a diff, **⌘F / Ctrl+F** (or the 🔍 in its toolbar) opens a find box: every match is highlighted, ↵ / ⇧↵ (or ↑ ↓) step through them (`3 of 12`), folded files open as needed, Esc closes. It ignores case unless you type a capital letter. Elsewhere ⌘F is the commit search.
- **Working directory:** the commit box on top, then every changed file under **Unstaged** (including untracked files) and **Staged**, with merge conflicts first (click one to open the [conflict editor](#resolving-conflicts)). Click a file header to expand its diff; each file has Stage / Unstage / Discard buttons (double-click stages or unstages), hunks have their own buttons, and each section has *Stage all* / *Discard all* / *Unstage all*.
- **Syntax highlighting** by file type, in diffs and in notes' fenced code blocks (```rust, ```ts …): keywords, strings, numbers, comments, types, function calls, and attributes, in colours taken from the theme. Built in and light (a small lexer per language family, no grammar files): Rust, JavaScript / TypeScript, Go, C / C++ / C# / Java / Kotlin / Swift / Dart / Scala / PHP, Python, Ruby, shell, SQL, Lua, CSS / SCSS, JSON, YAML, TOML / INI, HTML / XML / Vue / SVG, Dockerfile, and Makefile. Block comments and multi-line strings carry across lines.
- **Expanded by default** (Settings → Diffs): commits and staged files expanded, unstaged files collapsed. Expanding or collapsing a file by hand is kept while you work.
- **Narrow windows:** rows of buttons wrap instead of overlapping or running off the edge: the notes toolbar (the formatting buttons get their own row, and in a very narrow panel everything flows onto more lines), the note's target line, the commit box, and the history header; the diff toolbar shortens its labels ("Ignore ws").
- **Focus on the diff:** the ⛶ button on the diff toolbar, **⌘⇧↩**, Navigate → *Focus on Diff*, or ⌘P gives the diff the whole window (branches, history, and left-side notes hidden); the same again brings them back. Panels can't be dragged over each other: the commit details stop short of the left edge and the sidebar has a minimum and maximum width, so every edge stays grabbable.
- **Diffs** can be shown **inline** or **side by side** (icons above the diff; remembered). File headers mark only what's worth noticing, with an icon (named on hover): added (green ⊞), deleted (red ⊟, path struck through), renamed (blue, `old → new`), or conflict (⚠); modified files have none. The folder part of a path is dimmed so the file name stands out, and only non-zero line counts are shown. Paths in any language show as written (한글.md, not `\355\225\234…`; git's path quoting is turned off and undone where it still applies). A path too long for the row is shortened in the middle: folders become `…` (`gateway/…/DocsRouter.java`), and a long file name keeps its start and extension (`2025-12-19-…reviewed.txt`); hover the row for the full path. Wide characters (Hangul, CJK, emoji) count double when fitting. Hunks read **Lines 78–81** plus git's context line, instead of `@@ -78,3 +78,4 @@`. Code lines have room to breathe (about 1.4× the text size), collapsed files stack as a tight list (a thin divider between rows; file rows are 1.3× the code line height, so the padding sits inside each row) and a blank row follows only an expanded file's code, and while you scroll inside a file its header stays pinned under the toolbar (click it to go back to the top of the file). Multi-file diffs have a collapse / expand all button. Row buttons are sized to the row, so they never overlap the next one at any text size. **Discard** buttons (per file, per hunk, and Discard all) are red on a red tint, and Stage / Unstage are outlined, so they can't be mistaken for each other; Discard still asks first. Long lines **wrap** between words (no sideways scrolling). **Select code by dragging** over it (line numbers and headers aren't included; in side by side, one side at a time; double-click selects a line) and copy with ⌘C or right-click → Copy. Tabs are kept and wrapped lines are copied as one line. Inside changed lines, **the words that changed are highlighted**. **Ignore whitespace** hides indentation-only changes (`git diff -w`; hunk staging is off while it's on). **↑ / ↓** (or ⌥↑ / ⌥↓) glide to the previous / next change, which flashes.
- Commit menu: new branch here, detached checkout, cherry-pick, revert, reset (soft, mixed, hard).
- Fetch, pull, and push run in the background. The UI refreshes when the window regains focus.

**Notes**
- Notes are for **commits and branches**: the notes panel has a **Commit** tab (the selected commit) and a **Branch** tab, and a line under the tabs says which one you're editing and how it behaves. Tasks have no notes of their own: a task key links to its ticket (shown on the right of the Branch tab, with its title when the Jira API is set up). Older `repo` and `task:<KEY>` notes (and ones written over MCP) still open from the sidebar, ⌘P, or `note:` search.
- **PR tab:** when the branch has a pull request, a **PR #123** tab next to Branch shows its status, checks, and conversation using the whole panel. The tab you last chose (Commit, Branch, or PR) is kept across selections, repositories, and restarts whenever it exists. HTML in comments (bots like CodeRabbit use `<details>`, `<summary>`, images) is shown as plain text and Markdown, and code wraps inside the panel.
- **Two kinds of git notes:**
  - **Branch note:** bound to the branch name. It's shown on the branch's **top commit** (a chip in the graph, and the default tab when you select the tip). It follows the branch through rebase, amend, and new commits. Use it for ongoing work.
  - **Commit note:** bound to one exact SHA. After a rebase or amend it stays with the old commit. Use it for facts about that exact snapshot (benchmarks, review of that revision).
- **Branch renames are followed.** Renaming in Margin (right-click → Rename…) or in a terminal (`git branch -m`, read from the reflog, including chains) moves the branch note and task link to the new name.
- **Automatic re-attach (git patch-id).** When a commit note's commit is rewritten without changing its diff (rebase, cherry-pick, message-only amend), the note moves to the new commit automatically, and the toolbar message says so. It only moves on a unique match. If the change itself was edited (amend with new content, squash), the note stays detached for you to decide. Turn it off with `"auto_reattach": false`.
- **Undo for automatic re-attach.** Every automatic move can be reverted from the toolbar message ("Undo"), the sidebar's **Re-attached** list, or the banner on the note itself. Undo puts the note back on the old commit and restores the destination exactly, including a note that was there before a merge. If you edited the re-attached note afterwards, your edits are kept. An undone move is never applied automatically again. **Keep** confirms a move and clears it from the list.
- **Detached notes.** Commit notes whose commit is no longer on any branch, tag, or remote, and branch notes whose branch was deleted, are listed in the sidebar. Open one to see why it's detached. If the same change exists in several commits, each one is a one-click target. **Re-attach…** suggests the rewritten commit (patch-id match, else same subject), or you can delete it. Moving onto an existing note appends instead of overwriting.
- Markdown with tables, task lists, and links. **Images, raw HTML, and unsafe link schemes are rejected** on every write, from both the GUI and MCP.
- **Notes panel:** drag the grip on top to resize it, double-click the grip (or click ▾) to collapse it to its tab row, and click ⇤ / ⇥ to move it under the branches and history (one wide panel spanning both) or under the commit details (also ⌘P → *Move notes…*). These choices are remembered. The panel keeps the height you set, except for an empty note: then it shrinks to one line with a **Write note** button, and grows back (animated) when you start writing or switch to a note with text or to the PR tab. **⋯ → Delete note…** deletes the shown note after a confirmation; it's moved to `.git/margin/notes/.trash/`, so it can be recovered.
- **Layout: Editor, Split, and Preview** (Split: editor and live preview side by side), with **+ Entry** next to them to add a dated entry (`### <date> · <you>`) at the end. Notes open in Preview; **Edit** (top right), **Write note**, or double-clicking the note puts the cursor at the end of it. While editing, the same button is **Done** (or Esc / ⌘↩): it saves and goes back to Preview. A branch's ticket sits on its own line under "Note on branch …". While editing, a toolbar of icon buttons (with tooltips) formats the text: heading, **bold** (⌘B), *italic* (⌘I), code, link (⌘K) | bullet, checklist, quote. It wraps the selection, or toggles the prefix on the selected lines; in a narrow panel it gets its own row.
- **Readable:** the editor has roomy lines (1.5×) and padding, and colours the Markdown (headings, list and task markers, quotes, code blocks). Preview has the same padding and spacing between paragraphs. In a wide panel, text wraps at a comfortable reading width. The save state is always shown (● Saved / Saving… / Can't save).
- Autosave, and clickable task checkboxes. Internal links like `margin:commit:abc123` and `margin:task:PROJ-42` navigate inside the app (`margin:task:…` opens the ticket).
- Concurrency-safe: versioned writes. If an agent edits the note you're editing, you get a *Load theirs / Keep mine* choice.

**GitHub pull requests** (github.com and Enterprise Server). Display-only, so everything links out to GitHub.
- Merge readiness: *Ready to merge*, *Changes requested*, *Review required*, *Merge conflicts*, *Behind base*, *Draft*, *Merged*…
- CI status with each check (failures first, click to open the run).
- Comments: conversation comments, reviews, and unresolved review threads (`file:line`), newest first.
- Shown as a `#123` chip on the branch tip and in the sidebar, plus the **PR** tab in the notes panel. One GraphQL request covers all local branches. It reloads on window focus (at most once a minute) and after fetch, pull, or push.

**Tasks (Jira Cloud and Data Center)**
- The task key is inferred from the branch name (`feature/PROJ-42-login` → `PROJ-42`), or you can link it explicitly.
- The task card shows status, assignee, priority, and description. "My tasks" can create a branch from a ticket. The commit box offers an "Insert PROJ-42" button.
- **Commit box:** a summary line (↵ moves to the description; a counter turns amber past 50 characters and red past 72) and an optional description. ⌘↵ commits from either. The button says what it will do (**Commit 2 files**, **Amend last commit**), and when it's off, the reason is shown next to it (nothing staged, or no summary yet). Commit details show the full message, selectable and scrollable when long.

**MCP (for agents)**: 18 tools. `get_context`, `git_status`, `git_log`, `git_show`, and `list_branches` read git. `read_note`, `write_note`, `append_note`, `list_notes`, `search_notes`, and `link_task` work on notes. `get_task`, `search_tasks`, and `my_tasks` read Jira. `get_pr` gives agents the PR's readiness, failing checks, and review comments, so they can address feedback. `list_detached_notes`, `move_note`, and `undo_reattach` let agents tidy notes after a rebase. Git access is **read-only** for agents.

## View

Show or hide parts of the window: **View** in the macOS menu bar (the **View ▾** button in the toolbar on Windows and Linux), or ⌘P / Ctrl+P → *Show / Hide …*. Remembered across restarts.

- **Focus on diff** (⇧⌘↩ / Ctrl+Shift+Enter): the diff takes the whole window. While it's on, the toolbar shows **Exit focus**.
- Panels: **Sidebar** (⌥⌘1 / Ctrl+Alt+1), **History** (⌥⌘2), **Details and diff** (⌥⌘3), **Notes** (⌥⌘4). With the history hidden the details take its place, and the other way round; they're never both hidden.
- **Sidebar sections:** Branches, Pull requests, Remotes, Tags, Stashes, Worktrees, My tasks. The Pull requests list starts open only when it's short (8 or fewer) and shows 10 at a time (**Show N more**).

## Review mode

Compare two branches the way a pull request shows them, or review a GitHub pull request without leaving Margin. Review mode takes over the history and details (the sidebar stays); **‹ History** or Esc goes back.

**Open it:**
- Right-click a branch → **Compare with <current branch> (current)**, **Compare with the default branch**, or **Compare two branches…**; or **⌘P / Ctrl+P → Compare branches…** (pick the branch with the changes, then the base). Any two local branches work, pushed or not, with or without GitHub; the base and compared branch can be changed in review mode's header.
- A pull request: click it under **PULL REQUESTS** in the sidebar (open PRs, the ones waiting for your review first, marked ●), **⌘P → Review pull request…**, a branch's right-click menu → **Review pull request #n**, or **Review** on the PR card in the notes panel.

**What it shows**, like GitHub:
- **Files changed:** what the branch brings in since it split off from the base (`git diff base...head`, so later work on the base isn't mixed in). A file list on the left with +/− and **Viewed** checkboxes (a viewed file folds; it comes back if it changes again; kept per comparison in `.git/margin/review.json`); click a file to jump to it. The diff has syntax highlighting, inline / side-by-side, and the PR's open review threads under their lines, with **Reply** and **Resolve**.
- **Commits:** the branch's commits (`base..head`); click one for its own diff.
- **Conversation** (pull requests): the description, checks, reviews and comments.

**Reviewing a pull request:** hover a line and click **+** to comment (type **@** to mention someone: the pull request's author, reviewers and commenters come first, then anyone GitHub lets you mention in the repository as you type more; ↑↓ and ↵ or Tab to pick); the comment waits as **Pending** (Remove takes it out). **Review changes** (top right) writes a summary and submits it with the pending comments as **Comment**, **Approve**, or **Request changes**, on the commit you reviewed. A pull request from someone else's fork works too: its head is fetched into `refs/margin/pr/<n>` (no branch of yours is created), and its base is fetched fresh from the remote.

## Stashes

Put changes aside to get back to later.

- **From the command palette**, name it as you go: **⌘P / Ctrl+P → Stash changes…** (or **Stash staged changes…**), type the name, ↵ (or ↵ straight away for git's default name). **Stash some files…** opens the dialog below.
- **Stash…** (next to Commit in the working directory, or the stash command in the menu bar) asks for a name and what to stash: **all changes** (optionally keeping what's staged in place too), **only staged changes** (stage the hunks or files to put aside, then stash just those), or **only some files** (picked from a list). New, untracked files are included unless you untick it. A file's right-click menu in the working directory has **Stash this file…**.
- **STASHES** in the sidebar lists them by name, newest first (hover for where and when each was made). Click one to see what it holds, untracked files included, with **Pop** (apply and remove), **Apply** (keep it), **More ▾** (apply restoring what was staged, **New branch from stash…**, copy the message) and **Drop…**.
- **New branch from stash…** makes a branch at the commit the stash was made on, checks it out and applies the stash there (removing it if it applies cleanly): the way out when a stash no longer applies where you are.
- Stashing some files while other files are staged keeps those others out of the stash, and staged (plain `git stash` would record them in the stash too).

## Insights

Who changed what, where and when, from the history: **⌘P / Ctrl+P → Insights** (macOS: **Repository → Insights**). It takes the place of the history, like review mode; **‹ History** or Esc goes back.

- **Range and branches** (top right): the last **30 days** (the default), 90 days, a year, or all time; on the **current branch** (the default) or **all branches**. Merge commits aren't counted (a merged branch's work would count twice), and people are counted once each, as the repository's `.mailmap` names them.
- **Overview:** commits, people, files changed and lines; commits per week; and when work happens (weekday × hour, in your time zone). The busiest folders and the most active people are clickable.
- **Directories:** a folder tree with commits, lines, the main contributor and their share, and how many people work there. Click a folder to open it and narrow everything to it; click it again to see who works there. Folders where one person made 80% or more of the changes (8 commits or more) are marked **mostly one person**: code few others know.
- **Contributors:** commits, lines, files, active days, where each person works most, and their last commit; sort by any column. Click a person to see where they work.
- **Hotspots:** the files changed most often, and by how many people (**busy** at 4 or more): where bugs and merge conflicts tend to come from. Click one to see who changed it.
- The folder and person you chose show as chips; × takes one off. **Ignore generated files** (on by default) leaves out lock files and vendored or generated code (`package-lock.json`, `Cargo.lock`, `vendor/`, `*.min.js`, `schema.rb`, …), whose line counts would drown the rest.

## Interactive rebase

Reorder, reword, squash or drop commits before you push (or share a cleaner history). Right-click a commit in the history → **Interactive rebase from here…** (that commit and the ones after it on the current branch), or **⌘P / Ctrl+P → Interactive rebase…** (every commit since the branch split off from the default branch). The plan takes the place of the history; **‹ History** or Esc leaves without changing anything.

- The commits read like the history, newest at the top. Give each an action, from its menu or with a key on the selected row: **Pick** (P) keeps it, **Reword** (R) keeps the changes with a new message, **Edit** (E) stops after it so you can amend it or add commits, **Squash** (S) folds it into the commit below and lets you write the combined message, **Fixup** (F) folds it in keeping the message below, **Drop** (D) leaves it out.
- **Drag** a commit by its handle to move it, or **Alt+↑ / Alt+↓**. The summary shows what you'll end up with (`5 commits → 3`); commits already pushed are marked, since rewriting them needs a force push.
- **Start rebase** runs it (changes you haven't committed are put aside and back). Messages you wrote are applied without an editor. If it stops, on conflicts or at an **Edit**, the [conflict editor](#resolving-conflicts) or the red **rebase in progress** badge takes you on: resolve, then **Continue** (or **Skip commit** / **Abort**).
- Ranges with merge commits aren't offered (a rebase would flatten them).

## Resolving conflicts

When a merge, pull, rebase, cherry-pick, revert or stash pop stops on conflicts, the conflict editor opens in place of the history (like review mode; **‹ History** or Esc goes back). Open it again any time from a conflicted file in the working directory, the red **merge in progress** badge, or **⌘P / Ctrl+P → Resolve conflicts…**.

- **Files:** the conflicted files on the left, each with what happened to it (both changed, deleted on one side…); resolved ones are ticked below them.
- **Each conflict** shows the two sides next to each other, syntax highlighted: **Current** (the branch you're on) and **Incoming** (what's merged in). In a rebase they're **Rebasing onto** and **Your commit**. Keep one with **Use current** / **Use incoming**, keep **Both** (either order), show the **Common ancestor** (when the file has one) and use it, or **Edit…** to write the result yourself. A chosen conflict shows its result; **Change** brings the choice back.
- Text both sides agree on is shown dimmed around the conflicts, long stretches folded (click to show them). **Next unresolved ↓** jumps to the next one.
- **Mark resolved** writes the file and stages it, then goes to the next file. **Whole file ▾**: use one side's version for the whole file, **Start over** (git's conflict markers back, with the common ancestor), mark it resolved as it is (if you fixed it in your own editor), or open it.
- **A file one side deleted:** **Keep the file** or **Delete it**, with a preview of the version that's left. Binary files: use one side's version.
- **Finish** from the header: **Commit merge** (or **Continue** for a rebase, cherry-pick or revert, **Skip commit** in a rebase) once every file is resolved, or **Abort** to go back to how things were before.

## Updates

Margin updates itself from GitHub Releases on macOS (from 0.2 on), Windows and Linux (from 0.4 on). **Settings → Updates**, **Margin → Check for Updates…** (macOS), or ⌘P / Ctrl+P → *Check for updates…*:

- **Check for updates once a day** (on by default): one request to GitHub's releases API a day (and once right after a restart while an update is waiting). Nothing else is sent.
- **Download and install updates automatically** (on by default): a new version downloads in the background and its SHA-256 must match the release's. Then, per system:
  - **macOS:** its version must be the release's, its code signature must verify, and when your copy is Developer ID signed the new one must come from the same developer. It replaces Margin.app when you quit (the old app is kept until the swap succeeds).
  - **Windows:** the new installer runs silently over your install when you quit (same folder, same options).
  - **Linux:** the new `margin --version` must print the release's version; then the AppImage file, or the files of your tar.gz install, are replaced (a running Margin keeps its old copy).

  The toolbar shows **Restart to update** to do it right away.
- With automatic install off, the toolbar shows **Update to x.y.z**; one click downloads and installs it the same way.
- **Where it updates in place:** macOS — Margin.app in a folder you can write to (normally /Applications); Windows — installed with the installer, for your user; Linux — the AppImage (the file is replaced; keep it in a folder you own), or installed from the tar.gz in a folder you own (`install.sh` puts it in `~/.local`). Otherwise (the DMG, the portable Windows zip, an all-users install, a `.deb` that apt manages) Margin opens the release page instead.

## Open from the terminal

On Windows, the installer's *Add the margin command to PATH* option does this; **Install 'margin' command…** (⌘P / Ctrl+P) also works there, for the portable zip (it adds `margin.cmd` to your WindowsApps folder). On every system, a running Margin opens the folder as a new tab.


**Margin → Install 'margin' Command…** (or ⌘P → *Install 'margin' command*) puts a `margin` command in `/usr/local/bin` (macOS asks for your password if that folder isn't writable; on Linux it goes in `~/.local/bin`). `margin .` opens the current folder and `margin ~/src/app` opens another; when Margin is already running, the folder opens as a tab in that window. Reinstall it if you move Margin.app.

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

## Connect GitHub and Jira

**Settings → Accounts** (⌘, or Ctrl+,):

- **GitHub:** host (`github.com` or your Enterprise host) and a token. **Test** shows who it signs in as. Nothing is needed if you use the GitHub CLI: Margin reuses `gh auth token` (a token saved here wins over it and over `GH_TOKEN` / `GITHUB_TOKEN`; `MARGIN_GITHUB_TOKEN` wins over all).
- Where it shows: a commit's **Tasks** row lists each key with its status (To Do / In Progress / Done) and title, the notes panel's **Branch** tab shows the branch's ticket, and the sidebar has **My tasks**. **Repository settings** says what a repository's keys link to (✓) or why they don't (!), with a way to fix it.
- **Jira:** your site, Cloud or Data Center / Server, your email (Cloud), and an API token (Cloud: id.atlassian.com → Security → API tokens; Data Center: a personal access token). **Test** shows who it signs in as.

Saving applies right away (no restart). Tokens never go in Margin's settings files: they're kept in the system's secure store: the **macOS Keychain** (services `margin-github` / `margin-jira`), **Windows Credential Manager** (`margin:<service>/<account>`), or on Linux the **desktop keyring** (Secret Service, via `secret-tool`; GNOME Keyring, KWallet). A Linux machine without a keyring keeps them in `secrets.json` in the config folder, readable only by your user; tokens saved there earlier move to the keyring when saved again. Accounts set here are also used by Margin's MCP server, so agents don't need a second setup.

config.json still works and is overridden by what's set in the app: `jira` (`base_url`, `flavor`, `email`; token from `$MARGIN_JIRA_TOKEN` or the Keychain), `github.enterprise_hosts`, and `github.host_aliases` for SSH host aliases from `~/.ssh/config` (`git@github-work:org/repo`). TLS uses the OS trust store, so corporate CAs installed in the Keychain work. `HTTPS_PROXY` and `NO_PROXY` are honoured.

## Link other task trackers

**Per repository** (repo menu at the top left → **Repository settings…**, or ⌘P), choose where task keys come from: **Default** (the app-wide setting below), **GitHub Issues**, **Jira**, a **Ticket link**, or **Off**. With **GitHub Issues**, `#123` in commit messages (also "fixes #123") and branches like `123-fix-login`, `feature/123-x`, `issue-45`, or `gh-45` link to that repository's issues; pull request numbers ("Merge pull request #12", "(PR #12)", a squash merge's trailing "(#12)") and dates (`2026-10-30-…`) are skipped. Issue titles and open/closed state come through the GitHub connection, and *My tasks* lists the open issues assigned to you in that repository. The `task:` search filter matches either kind of key. A GitHub repository with no task source set shows one suggestion in the sidebar, *Use this repository's GitHub Issues for tasks? [Use] [Not now]*, asked once per repository; *My tasks* shows where its tasks come from ("MY TASKS · GITHUB", *Task source…*), and the notes panel offers *Set up tasks…* when a repository has none. **Repository settings** also holds the repository's notes sync switch and which branches the history shows. (The MCP server still reads tracker-style keys.)

Without a Jira API config, set **Settings → Tasks → Default ticket link**: a Jira site such as `https://acme.atlassian.net` (keys open `/browse/KEY`), or a template with `{key}` such as `https://linear.app/acme/issue/{key}`. Task keys in branch names and commit messages then open the ticket with one click (the key in commit details, and on the Branch tab of the notes panel). Look-alikes such as `UTF-8`, `SHA-256`, `ISO-8601`, and `CVE-…` aren't treated as task keys, and lowercase keys (`ops-42` in a branch name) need two or more digits and a prefix that isn't an everyday branch word, so `topic-1`, `release-2`, or `feature-123` don't count.

## Where notes live

`<repo>/.git/margin/notes/`. Notes are plain `.md` files, never committed, and shared across worktrees. They're easy to grep, back up, or sync.

See [docs/ARCHITECTURE.md](ARCHITECTURE.md) for design decisions and the roadmap. Icons: [Primer Octicons](https://github.com/primer/octicons) (MIT), see [THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md).

## Notes sync (iCloud Drive / Dropbox / Google Drive)

Sync notes between your computers through a folder your cloud provider already syncs. There's no account and no API: **Settings → Notes sync**, pick the folder (iCloud Drive, Dropbox, Google Drive, and OneDrive are detected), and tick **Sync notes for <repo>**.

- **Where notes live:** they stay in `.git/margin/notes` and are mirrored two-way with `<folder>/Margin/<repo-id>/`. The repo id comes from the remote URL (ssh and https forms match), so clones in differently named folders find each other.
- **Edits:** they sync within a second or two while Margin runs. Agent edits made through MCP are mirrored too, even when the app is closed. If both computers edit the same note, both versions are kept (merged under a marker).
- **Safety:** pointing sync at a different or empty folder is treated as a first sync, so nothing gets deleted. Deleted notes go to `.trash/` instead of disappearing. A pass that would delete most of your notes is refused. iCloud placeholders that aren't downloaded yet are never mistaken for deletions.
- **Repositories:** any number can sync.

## GitHub and task trackers

Everything in Margin is free and open source. With a GitHub account (and optionally Jira) connected in **Settings → Accounts**:

- **GitHub PR status:** readiness, CI checks, and review comments.
- **Review threads in the diff:** a PR's inline review threads appear as cards under the line they're on, in commit diffs: on the commit a thread was written on (at its original line) and, for open threads, on the PR's latest commit. Each card shows every comment, with **Reply** (a box opens in the card; ⌘↩ sends, Esc cancels), **Resolve** / **Unresolve**, and a link to GitHub. The card reloads with the new comment or state; on an error the draft stays. Resolved and outdated threads fold to one line (click to open).
- **New comments on any line:** hover a line in a commit diff (of a commit in an open PR) and click **+** next to its number. A card opens under the line; ⌘↩ posts it to GitHub as a review comment on that commit (left side for removed lines). A line must be part of the PR's diff for GitHub to accept it.
- **Emoji:** `:tada:`-style shortcodes in PR comments show as emoji, in colour from the system's emoji font (Apple Color Emoji, Segoe UI Emoji, or Noto Color Emoji), the same as in other apps. In the comment boxes, typing `:` and a few letters opens a dropdown (↑/↓, ↵ or Tab inserts, Esc closes).
- **Task trackers:** Jira ticket details, My tasks, and creating a branch from a ticket.
