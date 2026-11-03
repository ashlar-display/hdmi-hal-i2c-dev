# Roadmap

Deferred work that is out of scope for the initial release but should be revisited
as the stack matures.

## `DdcBus` implementation (with `hdmi-hal`'s breaking release)

`hdmi-hal` gains `ddc::DdcBus`, one trait for the whole DDC bus (EDID with the E-DDC segment
pointer, DisplayID, SCDC, HDCP, DDC/CI), with errors that carry a kind and a capabilities
report that never says more than the hardware can do. This crate implements it, replacing
its direct `ScdcTransport` impl: SCDC then goes through `hdmi-hal`'s `ScdcOverDdc` adapter.

- **`open()` over `I2C_RDWR`:** combined transfers across addresses (an E-DDC read writes
  the segment to 0x30 and reads 0x50 with no STOP between) and transfers beyond 32 bytes
  (HDCP reads of up to 635 bytes), through `i2cdev`'s `LinuxI2CBus`, which keeps each
  message's address (`LinuxI2CDevice::transfer` rewrites them to the device's own). A result
  with fewer messages than sent is an error; a message over the kernel's 8192 bytes is refused
  before the ioctl.
- **`open_smbus()`:** today's SMBus calls, for adapters without plain I²C and for the
  `i2c-stub` test target, with capabilities that say so (writes of 33 bytes, block reads of
  32, E-DDC segment 0 only).
- **An adapter table.** Linux DDC adapters advertise plain I²C, yet several reshape a transfer
  and report success: dw-hdmi and dw-hdmi-qp send a multi-byte write as single-byte register
  writes and use one address for the whole transfer; Inno and RK3066 return EDID bytes for a
  read at any address; MediaTek's second DDC driver writes an offset before every read;
  Oaktrail drops writes. Others refuse some shapes with `EINVAL` (sun4i, amdgpu's display
  core, msm). `open()` reads the adapter's name from sysfs and takes its capabilities from a
  documented table of these; an adapter outside the table gets SMBus-level capabilities, and
  `open_with(caps)` overrides.
- **Error mapping** onto `hdmi-hal`'s kinds: `ENXIO` to an unacknowledged transfer of unknown
  position (i915 returns it for a NAK on any byte), `EAGAIN` to a timeout or busy bus by
  adapter, `ENODEV` and `ESHUTDOWN` to a detached bus, `EOPNOTSUPP` to unsupported.
- **An EDID read** over `piaf`'s planned read plan, for dump tools.
- **No panic on a poisoned mutex.**
- The `I2C_RDWR` path has no CI target (`i2c-stub` offers SMBus only): unit tests use a mock
  that records addresses, and the release lists the adapters it ran on.

## `connector_ddc_adapter` → `linux-drm` crate

`connector_ddc_adapter` resolves a DRM connector name to a DDC adapter device path via
sysfs. This is a general operation with no dependency on SCDC, I²C transactions, or
address 0x54. Any tool that needs DDC bus access — for EDID reads, CEC, or anything
else — requires the same lookup.

The function lives in this crate because there is currently no second consumer. When a
second crate in this stack needs DDC adapter resolution, `connector_ddc_adapter` should
move to a dedicated `linux-drm` (or similarly named) utility crate, and this crate
should take a dependency on it.

**Trigger:** a second crate needs `connector_ddc_adapter`, or `piaf` needs to locate a
DDC bus without going through SCDC.

## `from_file` constructor for `I2cDevTransport`

The architecture calls for `I2cDevTransport::from_file(File) -> Self` to support
privilege-separation patterns where a privileged parent opens `/dev/i2c-N` and passes
the file descriptor to an unprivileged child.

This is not implemented because `LinuxI2CBus` (from `i2cdev`) only exposes a
path-based constructor; its internal file field is private and there is no
`FromRawFd` implementation. Implementing `from_file` correctly requires either:

1. Upstream `i2cdev` support — a `LinuxI2CBus::from_raw_fd` or `from_file` constructor.
2. A targeted `#[allow(unsafe_code)]` exception in this crate to call `I2C_RDWR`
   directly on the raw fd.

**Trigger:** a use case requires privilege separation and neither workaround is
acceptable.

## Multi-card sysfs validation

`connector_ddc_adapter` assumes the sysfs path structure is
`/sys/class/drm/<connector>/ddc`. Systems with multiple DRM cards are expected to
follow the same structure (e.g. `card1-HDMI-A-1`). This has not been tested on a
multi-GPU system.

**Trigger:** a multi-GPU system is available for testing, or a bug report surfaces.

## Richer errno coverage for Tier 2 tests

The Tier 2 integration tests cannot currently exercise `AddressNack` on `amdgpu`
because the driver collapses all errors to `EIO`. Testing the named `I2cErrorKind`
variants end-to-end requires a compliant non-`amdgpu` I²C adapter. The Tier 1 unit
tests provide coverage of the mapping logic.

**Trigger:** a compliant adapter is available in a CI environment.
