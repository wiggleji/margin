# Contributing to Margin

Thanks for helping! Bug reports, ideas, and pull requests are all welcome.

- **Bugs and ideas:** [open an issue](https://github.com/wiggleji/margin/issues/new/choose). For a bug, say what you did, what you expected, and what happened, with your macOS and Margin versions (Margin → About Margin).
- **Questions:** issues are fine too.
- **Bigger changes:** open an issue first so we can agree on the approach before you spend time on it.

## Build and run

You need `git` and [rustup](https://rustup.rs). The Rust version is pinned in `rust-toolchain.toml`; rustup installs it on the first `cargo` command.

```bash
cargo run -p margin-app -- /path/to/a/repo
```

```bash
cargo test --workspace
```

On Linux, egui needs a few system libraries first:

```bash
sudo apt-get install -y libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev libxkbcommon-dev libssl-dev
```

To try a demo repository with branches, merges, tags and notes: `scripts/demo-repo.sh ~/margin-demo`, then open `~/margin-demo/lumen`.

How the code is laid out: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md). What every feature does: [docs/GUIDE.md](docs/GUIDE.md).

## Branches and pull requests

`main` is always releasable. Every change goes through a short-lived branch and a pull request:

1. Branch from `main`, named `type/topic`:
   - `feat/…` a new feature (`feat/syntax-highlighting`)
   - `fix/…` a bug fix (`fix/branch-chip-overlap`)
   - `docs/…` documentation only
   - `refactor/…`, `perf/…`, `test/…`, `chore/…` (build, CI, dependencies)
2. Keep the change focused: one topic per pull request.
3. Before you push:
   - `cargo test --workspace` passes
   - `cargo build --workspace --locked` works (commit `Cargo.lock` changes)
   - new code adds no `cargo clippy` warnings
   - user-facing changes are described in `docs/GUIDE.md` (and the README if it's a headline feature)
4. Open the pull request against `main` and fill in the template. For UI changes, add a screenshot.

CI builds and tests on Linux and macOS. Pull requests are squash-merged, so the PR title becomes the commit on `main`: write it as a short summary of the change ("Syntax highlighting in diffs", "Fix wrapped branch chips overlapping").

## Style

- Follow the code around you. Comments say *why*, in plain sentences.
- Keep Margin light: think twice before adding a dependency, and prefer a small focused crate (or a few lines of code) over a large one.
- Interface text is short and plain: say what happens, no jargon.

## Releases

Maintainers release from `main` with *Actions → Release → Run workflow*; see [docs/RELEASING.md](docs/RELEASING.md). Versions follow `MAJOR.MINOR.PATCH`.

## License

By contributing you agree that your contributions are licensed under the [MIT License](LICENSE).
