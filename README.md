# hdmi-hal-i2c-dev

[![CI](https://github.com/ashlar-display/hdmi-hal-i2c-dev/actions/workflows/ci.yml/badge.svg)](https://github.com/ashlar-display/hdmi-hal-i2c-dev/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/hdmi-hal-i2c-dev.svg)](https://crates.io/crates/hdmi-hal-i2c-dev)
[![docs.rs](https://docs.rs/hdmi-hal-i2c-dev/badge.svg)](https://docs.rs/hdmi-hal-i2c-dev)
[![License: MPL-2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](LICENSE)
[![Rust 1.85+](https://img.shields.io/badge/rustc-1.85+-orange.svg)](https://blog.rust-lang.org/2025/02/20/Rust-1.85.0.html)
[![SLSA Level 2](https://slsa.dev/images/gh-badge-level2.svg)](https://slsa.dev)
[![trusted publishing only](https://img.shields.io/badge/dynamic/json?url=https%3A%2F%2Fcrates.io%2Fapi%2Fv1%2Fcrates%2Fhdmi-hal-i2c-dev&query=%24.crate.trustpub_only&label=trusted%20publishing%20only)](#verifying-releases)

> **Part of [Ashlar](https://ashlar-display.org)** — a display connection stack in Rust,
> HDMI 2.1 first: from EDID parsing through mode negotiation to FRL link training.
>
> [display-types](https://github.com/ashlar-display/display-types) ·
> [piaf](https://github.com/ashlar-display/piaf) ·
> [concordance](https://github.com/ashlar-display/concordance) ·
> [cartouche](https://github.com/ashlar-display/cartouche) ·
> [culvert](https://github.com/ashlar-display/culvert) ·
> [plumbob](https://github.com/ashlar-display/plumbob) ·
> [hdmi-hal](https://github.com/ashlar-display/hdmi-hal) ·
> **hdmi-hal-i2c-dev**

Linux userspace [`hdmi-hal`] backend backed by `/dev/i2c-N` via the `i2c-dev` kernel interface.

`hdmi-hal-i2c-dev` is the **development, validation, and diagnostic backend** for the HDMI stack.
It implements [`ScdcTransport`] over a real `/dev/i2c-N` device node and provides [`StubPhy`], a
callback-based [`HdmiPhy`] stub that lets you run the full FRL link training state machine from
userspace without a PHY kernel driver. Use it to validate protocol crates ([`culvert`], [`plumbob`])
against a real HDMI sink before and alongside kernel integration.

This crate is **not** the production path. The production implementation is a kernel module
operating through the DRM driver's `i2c_adapter`. See [`doc/architecture.md`] for full context.

## Usage

```sh
cargo add hdmi-hal-i2c-dev
```

```rust,no_run
use hdmi_hal_i2c_dev::{connector_ddc_adapter, I2cDevTransport, StubPhy};

// Resolve the DDC adapter for a named DRM connector and open the transport.
let path = connector_ddc_adapter("card0-HDMI-A-1")?;
let transport = I2cDevTransport::open(path)?;

// StubPhy forwards each PHY call to a callback and returns Ok(()).
// Pass a closure for logging, test assertions, or recording.
let phy = StubPhy::new(|call| eprintln!("{call:?}"));

// Pass transport and phy to culvert / plumbob as you would any other backend.
# Ok::<(), hdmi_hal_i2c_dev::I2cDevError>(())
```

## Block reads

`I2cDevTransport::read_block` is one SMBus I2C block read (one ioctl), up to 32 bytes
(`transport::MAX_BLOCK_LEN`; a longer read is refused, not split). The adapter must support
the SMBus I2C block read: the DRM DDC adapters of `i915` and `amdgpu` do, and on one without
it a block read fails with `I2cErrorKind::Unsupported`; for such an adapter,
`I2cDevTransport::open_bytewise` reads a block one register at a time instead, which is not
one transaction. One ioctl is not always one transaction on the wire: that is the adapter
driver's, and the Synopsys DesignWare HDMI controller's (`dw-hdmi`) reads byte by byte. See
[`doc/architecture.md`] for the adapters checked.

## Privileges

The calling process needs read/write permission on the `/dev/i2c-N` device node — typically via
membership in the `i2c` group, or by running as root.

## `amdgpu` limitation

The `amdgpu` DDC adapter collapses all I²C errors to `EIO`. On that hardware,
`I2cTransactionError::kind` is always `Unknown { errno: EIO }` regardless of the actual bus
condition. The named `I2cErrorKind` variants (`AddressNack`, `ClockStretchTimeout`, etc.) are
correct per `Documentation/i2c/fault-codes.rst` and are reachable on compliant adapters.
See [`doc/architecture.md`] for details.

## Documentation

- [`doc/architecture.md`] — design rationale, type descriptions, and test strategy
- [`doc/setup.md`] — build and test instructions, including `i2c-stub` setup for integration tests
- [`doc/testing.md`] — what each test tier covers and why

## Verifying releases

Releases are built on GitHub Actions and attested with
[SLSA Build Level 2](https://slsa.dev) provenance (from 0.2.0). To verify a release
`.crate` against its signed provenance, install the
[GitHub CLI](https://cli.github.com/) and run:

```sh
gh attestation verify hdmi-hal-i2c-dev-X.Y.Z.crate --repo ashlar-display/hdmi-hal-i2c-dev
```

The release from before the repository moved to the `ashlar-display` organization (0.2.0)
was attested under its previous owner, and verifies against it:

```sh
gh attestation verify hdmi-hal-i2c-dev-0.2.0.crate --owner DracoWhitefire \
  --signer-repo DracoWhitefire/hdmi-hal-i2c-dev
```

The attested `.crate` is attached to each
[GitHub release](https://github.com/ashlar-display/hdmi-hal-i2c-dev/releases).

Releases from 0.2.0 are uploaded through crates.io
[trusted publishing](https://rust-lang.github.io/rfcs/3691-trusted-publishing-cratesio.html): the publish
workflow obtains a short-lived token via OpenID Connect instead of using a stored
API token. crates.io can also reject uploads made with an API token for this crate;
the badge above shows whether that setting is on. Each version's publishing details,
including the workflow run and commit, are available from the crates.io API:

```sh
curl -A "your-tool (contact)" https://crates.io/api/v1/crates/hdmi-hal-i2c-dev/X.Y.Z
# see .version.trustpub_data
```

0.1.0 was uploaded by hand with an API token; it carries no provenance and has no
GitHub release or tag.

[`hdmi-hal`]: https://crates.io/crates/hdmi-hal
[`ScdcTransport`]: https://docs.rs/hdmi-hal/latest/hdmi_hal/scdc/trait.ScdcTransport.html
[`HdmiPhy`]: https://docs.rs/hdmi-hal/latest/hdmi_hal/phy/trait.HdmiPhy.html
[`StubPhy`]: https://docs.rs/hdmi-hal-i2c-dev/latest/hdmi_hal_i2c_dev/phy/struct.StubPhy.html
[`culvert`]: https://crates.io/crates/culvert
[`plumbob`]: https://crates.io/crates/plumbob
[`doc/architecture.md`]: doc/architecture.md
[`doc/setup.md`]: doc/setup.md
[`doc/testing.md`]: doc/testing.md
