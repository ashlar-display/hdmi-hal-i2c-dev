# Roadmap

Deferred work that is out of scope for the initial release but should be revisited
as the stack matures.

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

## `from_file` constructor for `I2cDevBus`

The architecture calls for `I2cDevBus::from_file(File) -> Self` to support
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
