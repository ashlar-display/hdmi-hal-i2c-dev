# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- **The README's Ashlar block links the stack's crate list** instead of naming every crate,
  so it no longer goes stale on crates.io when a crate is added.

### Internal

- **The README's Rust example compiles as a doctest** (`ReadmeDoctests` in `lib.rs`,
  compiled only for `cargo test`; it is `no_run`, since it opens a device), so it is
  checked against the API.
- **The published `.crate` holds only what the crate needs**: `include` in `Cargo.toml`
  limits it to the sources, `doc/`, the README, changelog and license; it also shipped the
  CI workflows, Dependabot configuration, coverage baseline and the integration tests.

## [0.4.0] - 2026-11-03

### Changed

- **`display-types` requirement widened to `>=0.4.3, <0.6`**, so both 0.4.3 and 0.5 are
  accepted, as in hdmi-hal. The display-types items hdmi-hal-i2c-dev uses are the same in both, so this
  is not a breaking change: an existing dependency on display-types 0.4 keeps resolving, and
  one on 0.5 now lines up with hdmi-hal-i2c-dev's.

## [0.3.0] - 2026-10-25

### Breaking changes

- **hdmi-hal moves to the release where `ScdcTransport::read_block` is required (0.6).**
  `I2cDevTransport` implements it (below); code that names hdmi-hal's types through this
  crate needs the same hdmi-hal.
- **Block reads need the SMBus I2C block read.** `read_block` was one `read` per register
  (hdmi-hal's old default); it is now one SMBus I2C block read, with no fallback. On an
  adapter without `I2C_FUNC_SMBUS_READ_I2C_BLOCK` (some SMBus-only adapters), block reads,
  and so culvert's `read_ltp_requests` and link training's LTS:3, now fail where byte reads
  worked: the error is `I2cErrorKind::Unsupported`, at the first block read rather than at
  open. Open such an adapter with `I2cDevTransport::open_bytewise` to keep reading a
  register at a time.
- **`EOPNOTSUPP` is `I2cErrorKind::Unsupported`**, no longer `Unknown { errno: 95 }`.

### Added

- **`I2cDevTransport::read_block`: one SMBus I2C block read** (`I2C_SMBUS_I2C_BLOCK_DATA`):
  one ioctl, the register offset written, a repeated START, the bytes read. One ioctl is
  one transaction on the wire where the adapter driver makes it one: the DRM DDC adapters
  of `i915` (GMBUS) and `amdgpu` (DC) do, through the kernel's SMBus emulation; the
  Synopsys DesignWare HDMI controller's I²C master (`dw-hdmi`, `dw-hdmi-qp`) reads one byte
  per hardware operation. Up to 32 bytes, **`transport::MAX_BLOCK_LEN`**; a longer buffer
  is **`I2cDevError::BlockReadTooLong`**, before any ioctl, never split. An empty buffer is
  `Ok` without an ioctl; a result of another length is
  **`I2cDevError::UnexpectedBlockLength`**, never padded (on `i2c-stub`, a read past 0xFF).
  Both errors are `#[non_exhaustive]` per variant.
- **`I2cDevTransport::open_bytewise`**: a transport whose `read_block` is one byte-data
  read per register (`read_block_bytewise`), for an adapter without the SMBus I2C block
  read. Not one transaction; documented as such.
- **`I2cErrorKind::Unsupported`** (`EOPNOTSUPP`).
- **Tier 2 tests for block reads** against `i2c-stub`: 9, 2 and 32 bytes, a 33-byte read
  refused, a read from 0xF0 ending at 0xFF, a bytewise block read, and a block read to an
  unregistered address.
- **`I2C_STUB_REQUIRED=1`** makes a tier 2 test whose adapter variable is unset fail
  instead of skipping, for CI and the release run.

### Changed

- **Documented what the error details can and cannot say.** `MessagePhase` is always
  `Write` from this crate, for reads too: one ioctl does not say which message failed.
  `AdapterGone` notes that `ENODEV` can also mean no device at all (as `i2c-stub` reports
  a missing chip). `UnexpectedBlockLength` says when it occurs, and that a driver reporting
  more than 33 bytes panics inside `i2cdev` instead, poisoning the transport.

- **Part of Ashlar** — the README now opens with a block naming the
  [Ashlar](https://ashlar-display.org) stack and linking the other crates in it. Keywords
  are aligned across the stack, with `ashlar` and `hdmi` on every crate; `ashlar` is added,
  `hal` is dropped. Categories gain `os::linux-apis`. The crate's `homepage` is now the
  Ashlar website.
- **Moved to the `ashlar-display` organization** — the repository is now
  `ashlar-display/hdmi-hal-i2c-dev` (the old URLs redirect), and `repository` points
  there. Releases are attested and published from the new repository; 0.2.0, attested
  before the move, verifies with `--owner DracoWhitefire --signer-repo
  DracoWhitefire/hdmi-hal-i2c-dev` (see "Verifying releases" in the README). The code of
  conduct is now the organization's, shared by every Ashlar repository; the repository's
  own copy is removed.

### Fixed

- **The NACK test instructions could not work**: they loaded "a second `i2c-stub`
  instance", but `i2c-stub` registers one adapter, and a second `modprobe` of the loaded
  module changes nothing, so the NACK tests were skipped. `doc/setup.md` and the test
  file now run them in a second pass with the stub reloaded at another address, and say
  that the stub reports a missing chip as `ENODEV` (`AdapterGone`), not `ENXIO`
  (`AddressNack`).
- **Skipped Tier 2 tests were silent**: a test whose adapter variable was unset reported
  `ok` like a test that ran. It now prints `skipped: <variable> not set` (seen with
  `--nocapture`), and fails instead with `I2C_STUB_REQUIRED=1`.

## [0.2.0] - 2026-10-11

### Breaking changes

- **`LtpPattern` (from `hdmi-hal`, carried by `PhyCall::SendLtp`) is now a
  `#[non_exhaustive]` enum** with the HDMI 2.1 pattern values (1 = all ones, 2 = all zeros,
  3 = Nyquist clock, 4 = DDE compliance, 5–8 = LFSR 0–3); `LtpPattern::new(u8)` is gone.
  Match on the variants (`LtpPattern::Lfsr0` etc.).
- **`PhyCall::SendLtp` carries the full per-lane pattern set** (`LanePatterns`), following
  `hdmi-hal`'s new `HdmiPhy::send_ltp(patterns: LanePatterns)`. `None` on a lane means no
  training pattern; `None` on every lane stops the training patterns.
- **`hdmi-hal` updated to 0.5**: `StubPhy` implements hdmi-hal 0.5's `HdmiPhy` and
  `I2cDevTransport` its `ScdcTransport`, the versions the reworked plumbob and culvert
  use; a crate on hdmi-hal 0.4 (such as plumbob 0.1 or culvert 0.1) cannot use them.
  0.1.0 depends on hdmi-hal 0.4.1, which was yanked, so a new project can no longer
  resolve it: use this release.
- **`MessagePhase` and `I2cTransactionError` are `#[non_exhaustive]`**, like the other
  error types: a burst `read_block` may need another phase or more detail. Match
  `MessagePhase` with a wildcard arm; `I2cTransactionError`'s fields are still readable,
  but it can no longer be built outside this crate.

### Added

- `PhyCall` derives `Clone`, `Copy`, `PartialEq` and `Eq`, so tests can record the calls
  and compare them as a whole.
- `StubPhy` implements `hdmi-hal`'s new required `HdmiPhy::set_frl_output`, forwarding it
  as the new `PhyCall::SetFrlOutput(FrlOutput)` event. `PhyCall` is `#[non_exhaustive]`,
  so matches with a wildcard arm are unaffected.

### Internal

- **Publishing uses crates.io trusted publishing** — `publish.yml` obtains a short-lived
  token through OIDC (`rust-lang/crates-io-auth-action`, revoked at the end of the job, and
  short-lived regardless). The repository never had a `CARGO_REGISTRY_TOKEN` secret, so the
  previous workflow could not have uploaded a release (0.1.0 was published by hand). The
  upload job runs in the `release` environment, which only `v*` tags can deploy to and
  which requires a maintainer's approval. Quality gates moved to a separate `verify` job,
  so no dependency build scripts run and no cache is restored where publishing credentials
  exist. The attested `.crate` is kept as a separate copy; after upload the job fails if
  crates.io's checksum differs from it, and that same copy is attached to the GitHub
  release. Re-running a partially failed publish reuses an existing GitHub release.
  `publish.yml` refuses tags that do not match the `Cargo.toml` version.
- **Release summary before approval** — `publish.yml` gains a `summary` job that shows the
  crate, version and commit, changes since the previous published release (ignoring
  pre-release or malformed tags, and flagging a baseline that is not on crates.io), changes
  to workflow, toolchain and build configuration, manifest changes, the changelog section,
  and the `.crate` sha256 with a command to reproduce it, on the run page while the publish
  job waits for approval. The publish job refuses to attest a package whose sha256 differs
  from the reviewed one.
- **Release prep and tagging workflows** — releases no longer need a manual promotion.
  `release-prep` picks the version bump with `cargo semver-checks` (or validates a chosen
  one), stamps the changelog and opens a `Release vX.Y.Z` PR into `develop`. `release-tag`
  then finds that release commit (merged or squash-merged), promotes `develop` to `main`
  through a `release-promote/v*` PR, tags only when `main` holds exactly the promoted tree,
  and dispatches `publish.yml` for the tag (which `workflow_dispatch` now allows; dispatches
  against a non-tag ref are skipped). Both fail when started from a branch other than
  `develop`.
- **No CI run on promotion PRs** — CI no longer runs on pull requests into `main`. The only
  ones are `release-tag`'s `release-promote/*` PRs, whose content already passed CI on
  `develop`; opened by `github-actions[bot]`, their runs would be held for approval and
  listed as `action_required` or failed.
- **Supply-chain hardening for CI** — every third-party action is pinned to a commit SHA
  of a release tag, with `dependabot.yml` keeping the pins (and `Cargo.lock`) current. The
  Rust toolchain itself still follows `stable`. `cargo-semver-checks` is installed as a
  checksum-verified prebuilt binary (the version listed by the pinned `install-action`).
  The security audit also runs weekly.
- **README documents release provenance** — a badge shows whether crates.io requires trusted
  publishing for this crate, and a new "Verifying releases" section states that SLSA
  provenance, GitHub releases and trusted publishing start at 0.2.0, that 0.1.0 was uploaded
  by hand with an API token, and how to verify a release and look up its publishing details.

## [0.1.0] - 2026-05-27

### Added

- `I2cDevTransport` — implements `ScdcTransport` over a `/dev/i2c-N` device node via
  SMBus byte-data ioctls (`I2C_SMBUS`) to the SCDC slave address (0x54).
- `StubPhy<F>` — implements `HdmiPhy` with a caller-supplied callback; forwards each PHY
  call as a `PhyCall` event and returns `Ok(())`. `type Error = Infallible`.
- `PhyCall` — structured event type describing a single `HdmiPhy` method invocation;
  `#[non_exhaustive]`.
- `connector_ddc_adapter` — resolves a DRM connector name (e.g. `card0-HDMI-A-1`) to its
  DDC adapter device path (`/dev/i2c-N`) via sysfs.
- `I2cDevError` — unified error type with variants for connector lookup, device open, and
  I²C transaction failures; `#[non_exhaustive]`.
- `I2cTransactionError`, `MessagePhase`, `I2cErrorKind` — structured transaction error
  types following `Documentation/i2c/fault-codes.rst`. `I2cErrorKind` is `#[non_exhaustive]`.
- Tier 1 unit tests: synthetic sysfs tree tests for `connector_ddc_adapter`; errno-mapping
  unit tests for `I2cErrorKind`.
- Tier 2 integration tests (gated behind `--features integration`): `I2cDevTransport`
  read/write correctness and NACK detection against the Linux `i2c-stub` kernel module.
