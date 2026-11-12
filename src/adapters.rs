//! What a Linux DDC adapter does with a transfer, by the name it registers under
//! (`/sys/class/i2c-dev/i2c-N/name`).
//!
//! The kernel's functionality bits do not describe what a transfer gets: almost every DRM
//! DDC adapter advertises plain I²C, and several reshape a transfer and report success, or
//! limit it and fail with `EINVAL`. The adapter's name does identify it, so
//! [`I2cDevBus::open`](crate::I2cDevBus::open) takes its capabilities from this table, built
//! from the drivers' transfer functions (Linux `e9954f3fbddd`; the i915, gma500 and amdgpu
//! adapter names as registered at `e774d5f1b`). An adapter not in it gets
//! [`SMBUS_LEVEL`]: the kernel's claim is no guarantee.
//!
//! | Adapter name | Driver | Capabilities |
//! |---|---|---|
//! | `i915 gmbus …` | i915 | general: combined transfers across addresses, STOP after the last |
//! | `gma500 gmbus …` | gma500 | general, the same design |
//! | `mediatek-hdmi-ddc` | MediaTek v1 | general: START and STOP driven by the driver |
//! | `msm hdmi i2c` | msm | general: at most 4 messages per transfer, more than any operation here uses |
//! | `DesignWare HDMI` | dw-hdmi | one address per transfer: `write` of at most 2 bytes (offset and one data byte), `write_read` with a one-byte write, no `read`, 0x37 not reachable, reads split byte by byte on the wire; E-DDC segment 0 only |
//! | `DesignWare HDMI QP` | dw-hdmi-qp | as dw-hdmi, and no DisplayID target |
//! | `Inno HDMI`, `RK3066 HDMI` | inno-hdmi, rk3066 | an EDID FIFO: only `read_segmented` of EDID |
//! | `oaktrail_hdmi_i2c` | gma500 (Oaktrail) | only `read_segmented` of EDID, segment 0 (its writes do nothing) |
//! | `mediatek-hdmi-ddc-v2` | MediaTek v2 | 0x50 and 0x54 only, no `read`; E-DDC segment 0 only |
//! | `sun4i_hdmi_i2c adapter` | sun4i | messages of at most 1023 bytes; E-DDC segment 0 only |
//! | `AMDGPU DM i2c …` | amdgpu (DC) | transfers that fit a 144-byte hardware buffer, address bytes included: `write` 143, `write_read` 1 + 128, `read` 143, E-DDC in 128-byte pieces |
//!
//! amdgpu's buffer is 538 bytes from DCE 10.0; the generation is not visible from the name,
//! so the table takes the smaller one.

use hdmi_hal::ddc::{DDC_CI, DdcCapabilities, EDID, EddcTarget, MAX_SEGMENT, SCDC};

/// What an adapter not in the table gets: what the SMBus calls of the past could do, which
/// every adapter takes. `write` of 33 bytes (offset and 32), `write_read` of 1 + 32, `read`
/// of 1, E-DDC segment 0 only in 32-byte pieces.
pub const SMBUS_LEVEL: DdcCapabilities = DdcCapabilities::general()
    .with_write_max(33)
    .with_write_read_max(1, 32)
    .with_read_max(1)
    .with_max_segment(Some(0))
    .with_segmented_piece(Some(32));

/// What an `EAGAIN` from the adapter means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Eagain {
    /// Linux's convention: arbitration lost, retry.
    Busy,
    /// The adapter's timeout: dw-hdmi's, Inno's, and i915's after a GMBUS timeout.
    Timeout,
}

/// What the crate knows about an adapter in the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Known {
    pub(crate) capabilities: DdcCapabilities,
    pub(crate) eagain: Eagain,
}

/// The capabilities of the adapter registered as `name`, from the table; `None` for an
/// adapter not in it.
pub fn capabilities(name: &str) -> Option<DdcCapabilities> {
    known(name).map(|known| known.capabilities)
}

/// The table's entry for `name`.
pub(crate) fn known(name: &str) -> Option<Known> {
    let general = DdcCapabilities::general();
    let dw_hdmi = general
        .with_write_max(2)
        .with_write_read_max(1, u16::MAX)
        .with_read_max(0)
        .with_address_reachable(DDC_CI, false)
        .with_splits_reads(true)
        .with_max_segment(Some(0));
    let edid_fifo = DdcCapabilities::none()
        .with_eddc_target(EddcTarget::Edid, true)
        .with_max_segment(Some(MAX_SEGMENT));
    let (capabilities, eagain) = match name {
        _ if name.starts_with("i915 gmbus ") => (general, Eagain::Timeout),
        _ if name.starts_with("gma500 gmbus ") => (general, Eagain::Busy),
        _ if name.starts_with("AMDGPU DM i2c ") => (
            general
                .with_write_max(143)
                .with_write_read_max(1, 128)
                .with_read_max(143)
                .with_segmented_piece(Some(128)),
            Eagain::Busy,
        ),
        "mediatek-hdmi-ddc" | "msm hdmi i2c" => (general, Eagain::Busy),
        "DesignWare HDMI" => (dw_hdmi, Eagain::Timeout),
        "DesignWare HDMI QP" => (
            dw_hdmi.with_eddc_target(EddcTarget::DisplayId, false),
            Eagain::Timeout,
        ),
        "Inno HDMI" | "RK3066 HDMI" => (edid_fifo, Eagain::Timeout),
        "oaktrail_hdmi_i2c" => (edid_fifo.with_max_segment(Some(0)), Eagain::Busy),
        "mediatek-hdmi-ddc-v2" => (
            general
                .with_reachable_addresses((1 << EDID) | (1 << SCDC))
                .with_read_max(0)
                .with_max_segment(Some(0)),
            Eagain::Busy,
        ),
        "sun4i_hdmi_i2c adapter" => (
            general
                .with_write_max(1023)
                .with_write_read_max(1023, 1023)
                .with_read_max(1023)
                .with_max_segment(Some(0)),
            Eagain::Busy,
        ),
        _ => return None,
    };
    Some(Known {
        capabilities,
        eagain,
    })
}

#[cfg(test)]
#[path = "adapters_tests.rs"]
mod tests;
