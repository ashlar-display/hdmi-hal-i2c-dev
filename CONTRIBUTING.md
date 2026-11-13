# Contributing to hdmi-hal-i2c-dev

Thanks for your interest in contributing. This document covers the basics.

## Getting started

See [`doc/setup.md`](doc/setup.md) for build, check, and test instructions, including how
to load `i2c-stub` for the Tier 2 integration tests.

## Issues and pull requests

**Open an issue first** if you're unsure whether something is a bug or if you want to
discuss a change before implementing it. For small, self-contained fixes a PR on its own
is fine.

- Bug reports: include the connector name, kernel version, driver in use, and the exact
  error returned.
- Feature requests: a brief description of what you need and why is enough to start a
  conversation.
- PRs: keep them focused. One logical change per PR makes review faster and keeps
  history readable.

## Coding standards

- Run `cargo fmt` and `cargo clippy --all-targets -- -D warnings` before pushing
  ([doc/setup.md](doc/setup.md) lists the commands CI runs).
- Public items need rustdoc comments, and intra-doc links must resolve
  (`cargo rustdoc --all-features -- -D warnings` must pass).
- `#![forbid(unsafe_code)]` is enforced; no unsafe code in this crate.
- A claim about HDMI behaviour (a rule, a default, a threshold, a register encoding) cites
  at least two independent public implementations (repository, commit, file and line).
  Where implementations differ, the behaviour is a setting with the majority as default.
- This crate is `std`-only and Linux-only. There is no `no_std` or async path.

## Tests

- **Tier 1** (no hardware): `cargo test --locked`. These must pass in CI on any Linux host.
- **Tier 2** (requires `i2c-stub`): `cargo test --locked --features integration`. These are
  not run in standard CI. See [`doc/testing.md`](doc/testing.md) and
  [`doc/setup.md`](doc/setup.md) for setup instructions. Run them before any release.

## Commit and PR expectations

- Write commit messages in the imperative mood ("Add support for …", not "Added …").
- Keep commits logically atomic. A PR that touches three unrelated things should be
  three commits (or three PRs).
- CI must be green before a PR can merge: fmt, clippy (the Tier 2 tests included), docs,
  the Tier 1 tests, the same tests on the MSRV (Rust 1.85), and coverage must not drop more
  than 0.1 percentage points below the baseline (stored in `.coverage-baseline`).

## Publishing and upstream dependencies

This crate depends on `hdmi-hal`. Changes to the `ScdcTransport` or `HdmiPhy` trait
surfaces in `hdmi-hal` are breaking changes for this crate. The workflow is:

1. Merge, tag, and publish the `hdmi-hal` change.
2. Update the version constraint in this crate's `Cargo.toml`.
3. Update `CHANGELOG.md` and open a PR.

## Review process

PRs are reviewed on a best-effort basis. Expect feedback within a few days; if you
haven't heard back in a week feel free to ping the thread. Approval from the maintainer
is required to merge.

## Code of Conduct

This project follows the [Contributor Covenant 3.0](https://github.com/ashlar-display/.github/blob/HEAD/CODE_OF_CONDUCT.md),
the code of conduct of every [Ashlar](https://ashlar-display.org) repository. Please read it
before participating.
