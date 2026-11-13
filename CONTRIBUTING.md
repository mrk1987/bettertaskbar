# Contributing to BetterTaskbar

Thanks for taking the time to contribute. This covers setup, the checks to run
and what a pull request should contain.

## Getting set up

1. Install [Rust](https://rustup.rs) (stable, 1.95 or newer). The toolchain file
   at the repository root adds `rustfmt` and `clippy`.
2. Clone the repository. The crate is in `widgets/`:

   ```sh
   cd widgets
   cargo run --release
   ```

3. Run the unit tests with `cargo test`.

You need Windows 10 or 11 to run the widgets. The test suite includes Win32 menu/window checks and process mutex tests, so
run it on Windows. Parsing and geometry checks use local fixtures.

## Before opening a pull request

Run the same checks as CI, from `widgets/`:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace -- --test-threads=1
cargo build --release
```

If you change how the widgets look or move, include a screenshot of the taskbar.
Read the notes on capturing it first: the widgets are drawn by a different part
of Windows than most apps, and some capture methods cannot see them. See
[docs/architecture.md](docs/architecture.md#testing).

## Code style

- Prefer small, focused modules. One widget per file in `src/widgets/`.
- Clients (`caldav.rs`, `imap.rs`, `ical.rs`) take plain inputs and return plain
  results, so they can be tested without a window or a server.
- `unsafe` is for Win32 calls only (`shell.rs`, `occupancy.rs`, `tray.rs`).
  Comment why a call is needed when it is not obvious.
- Never swallow an error silently. Return it, or log it with `log::line`.
- Write tests for behavior that can regress: parsing, layout math, state changes.
- Avoid new dependencies unless they remove much more code than they add.

## Adding a widget

1. Implement the `Widget` trait in `src/widgets/my_widget.rs` and register it in
   `src/widgets/mod.rs`.
2. Add a `Kind` variant in `src/app.rs` (title, state accessors, `Kind::ALL`) and
   its default tray offset.
3. Add its placement state to `Config` in `src/config.rs`.
4. Add it to the `--widget=` list and the hub's start list in `src/main.rs`, and
   to the tray menu in `src/tray.rs`.
5. Add a `docs/my_widget.md`, link it from the README, and add at least one
   unit test.

## Commit messages

Use [Conventional Commits](https://www.conventionalcommits.org/):

```
feat(calendar): show the end time of the running event
fix(mail): decode Q-encoded subjects that start with an encoded byte
docs: describe the dock model
```

Keep the subject under 72 characters and explain the *why* in the body when it
is not obvious.

## Pull requests

- Keep changes focused; split unrelated work into separate PRs.
- Describe the user-visible change and how you verified it.
- Link the issue being addressed.
- Make sure CI is green before requesting review.

## Reporting security issues

Please do not open a public issue for security problems. See
[SECURITY.md](SECURITY.md).
