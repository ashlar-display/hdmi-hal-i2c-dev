//! Error types for all fallible operations in this crate.

use i2cdev::linux::LinuxI2CError;
use std::io;
use std::path::PathBuf;

use hdmi_hal::ddc::{DdcError, DdcErrorKind, NoAcknowledgeSource};

/// The error type for all fallible operations in this crate.
#[non_exhaustive]
#[derive(Debug)]
pub enum I2cDevError {
    /// The specified DRM connector was not found in sysfs.
    ConnectorNotFound {
        /// The connector name that was looked up.
        connector: String,
    },
    /// The connector exists in sysfs but has no `ddc` symlink.
    /// Common for non-HDMI connectors and HDMI ports with no sink attached.
    ConnectorHasNoDdcAdapter {
        /// The connector name that was looked up.
        connector: String,
    },
    /// The `ddc` symlink exists but its target does not contain a recognizable
    /// adapter index. This indicates an unexpected sysfs layout.
    DdcAdapterIndexUnparseable {
        /// The full symlink target path that could not be parsed.
        symlink_target: PathBuf,
    },
    /// The `/dev/i2c-N` device could not be opened.
    DeviceOpenFailed {
        /// The device path that could not be opened.
        path: PathBuf,
        /// The underlying I/O error.
        source: io::Error,
    },
    /// An `I2C_RDWR` transfer of an [`I2cDevBus`](crate::I2cDevBus) failed: the ioctl
    /// returned an error. Which of its messages failed the kernel does not say.
    #[non_exhaustive]
    Transfer {
        /// The address of the transfer's first message.
        address: u8,
        /// The class of bus condition, from the errno.
        kind: I2cErrorKind,
    },
    /// The kernel executed fewer of a transfer's messages than were sent, as an adapter with
    /// a limit on messages per transfer does: the operation is incomplete.
    #[non_exhaustive]
    IncompleteTransfer {
        /// The messages sent.
        sent: u32,
        /// The messages the kernel executed.
        done: u32,
    },
    /// Refused before any ioctl: the `DdcBus` contract, the bus's capabilities or the kernel
    /// rule it out, and it would be refused again.
    #[non_exhaustive]
    Refused {
        /// What rules it out.
        reason: Refusal,
    },
}

/// Why an [`I2cDevBus`](crate::I2cDevBus) refused an operation before any ioctl.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// A write of no bytes: I²C's address probe, which many adapters reject.
    EmptyWrite,
    /// A message longer than the capabilities allow, or than the kernel's 8192 bytes.
    #[non_exhaustive]
    TooLong {
        /// The length asked for.
        len: usize,
        /// The most allowed.
        max: usize,
    },
    /// An address the capabilities do not reach.
    #[non_exhaustive]
    Unreachable {
        /// The address.
        address: u8,
    },
    /// An E-DDC target the capabilities do not reach.
    #[non_exhaustive]
    Target {
        /// Its address.
        address: u8,
    },
    /// An E-DDC segment beyond the capabilities' or E-DDC's highest.
    #[non_exhaustive]
    Segment {
        /// The segment.
        segment: u8,
    },
    /// An E-DDC read past the segment's 256 bytes.
    #[non_exhaustive]
    PastSegmentEnd {
        /// The offset.
        offset: u8,
        /// The length.
        len: usize,
    },
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyWrite => f.write_str("a write of no bytes"),
            Self::TooLong { len, max } => write!(f, "{len} bytes, more than the {max} allowed"),
            Self::Unreachable { address } => write!(f, "address 0x{address:02x} is not reachable"),
            Self::Target { address } => {
                write!(f, "the E-DDC target at 0x{address:02x} is not reachable")
            }
            Self::Segment { segment } => write!(f, "E-DDC segment {segment} is not reachable"),
            Self::PastSegmentEnd { offset, len } => {
                write!(f, "{len} bytes from offset {offset} run past the segment")
            }
        }
    }
}

/// The class of I²C bus condition that caused the failure.
///
/// Variants follow the conventions in `Documentation/i2c/fault-codes.rst`.
/// See the `amdgpu` note in the crate documentation for current hardware
/// limitations.
#[non_exhaustive]
#[derive(Debug)]
pub enum I2cErrorKind {
    /// The slave did not acknowledge its I²C address byte (`ENXIO`).
    AddressNack,
    /// The slave acknowledged its address but NACKed a data byte.
    DataNack,
    /// Arbitration was lost to another bus master (`EAGAIN`). Retry is appropriate.
    ArbitrationLost,
    /// The slave held SCL low beyond the adapter timeout threshold (`ETIMEDOUT`).
    ClockStretchTimeout,
    /// The bus was busy and the transaction could not be started (`EBUSY`).
    BusBusy,
    /// An unexpected START or STOP condition was observed on the bus (`EPROTO`).
    BusProtocolError,
    /// The I²C adapter was suspended or removed (`ESHUTDOWN`/`ENODEV`).
    ///
    /// `ENODEV` is not always that: `Documentation/i2c/fault-codes.rst` also gives it for a
    /// device that is not there at all, and `i2c-stub` returns it for an address with no
    /// chip, where a real adapter returns `ENXIO` ([`AddressNack`](Self::AddressNack)).
    AdapterGone,
    /// No chip at the address (`ENODEV` on a bus opened with
    /// [`I2cDevBus::open_smbus`](crate::I2cDevBus::open_smbus), as `i2c-stub` reports it).
    NoDevice,
    /// The adapter does not support the operation (`EOPNOTSUPP`).
    Unsupported,
    /// An errno that does not map to any known I²C condition.
    Unknown {
        /// The raw OS error number.
        errno: i32,
    },
}

impl std::fmt::Display for I2cDevError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConnectorNotFound { connector } => {
                write!(f, "DRM connector not found in sysfs: {connector}")
            }
            Self::ConnectorHasNoDdcAdapter { connector } => {
                write!(
                    f,
                    "connector {connector} has no DDC adapter (no sink attached, or not an HDMI port)"
                )
            }
            Self::DdcAdapterIndexUnparseable { symlink_target } => {
                write!(
                    f,
                    "DDC symlink target does not contain a recognizable adapter index: {}",
                    symlink_target.display()
                )
            }
            Self::DeviceOpenFailed { path, .. } => {
                write!(f, "failed to open I²C device: {}", path.display())
            }
            Self::Transfer { address, kind } => {
                write!(f, "I²C transfer to 0x{address:02x} failed: {kind:?}")
            }
            Self::IncompleteTransfer { sent, done } => {
                write!(
                    f,
                    "I²C transfer incomplete: {done} of {sent} messages executed"
                )
            }
            Self::Refused { reason } => write!(f, "refused: {reason}"),
        }
    }
}

impl std::error::Error for I2cDevError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::DeviceOpenFailed { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;

/// The classification `hdmi-hal` asks of a bus error, so that a caller such as a
/// procedure polling the sink can tell a failure that may pass from one that will not.
///
/// - A transfer maps its [`I2cErrorKind`]: `ENXIO` ([`AddressNack`](I2cErrorKind::AddressNack))
///   to `NoAcknowledge(Unknown)`, since i915 returns it for a NAK on any byte; a NACKed data
///   byte to `NoAcknowledge(Data)`; lost arbitration and a busy bus to `Busy`; a clock-stretch
///   timeout to `Timeout`; an adapter suspended or removed (`ESHUTDOWN`, `ENODEV`) to
///   `Detached`; `EOPNOTSUPP` to `Unsupported`; a protocol error and an unknown errno to
///   `Other`.
///   No chip at the address on an SMBus bus ([`NoDevice`](I2cErrorKind::NoDevice)) is
///   `NoAcknowledge(Address)`.
/// - A refusal is `Unsupported`: it would be refused again.
/// - An incomplete transfer is `Other`, as are the errors of opening a bus; those come before
///   any transfer.
impl DdcError for I2cDevError {
    fn kind(&self) -> DdcErrorKind {
        match self {
            Self::Transfer { kind, .. } => ddc_kind(kind),
            Self::Refused { .. } => DdcErrorKind::Unsupported,
            _ => DdcErrorKind::Other,
        }
    }
}

/// An errno class as a `DdcErrorKind`, as for [`I2cDevError`].
fn ddc_kind(kind: &I2cErrorKind) -> DdcErrorKind {
    match kind {
        I2cErrorKind::AddressNack => DdcErrorKind::NoAcknowledge(NoAcknowledgeSource::Unknown),
        I2cErrorKind::NoDevice => DdcErrorKind::NoAcknowledge(NoAcknowledgeSource::Address),
        I2cErrorKind::DataNack => DdcErrorKind::NoAcknowledge(NoAcknowledgeSource::Data),
        I2cErrorKind::ArbitrationLost | I2cErrorKind::BusBusy => DdcErrorKind::Busy,
        I2cErrorKind::ClockStretchTimeout => DdcErrorKind::Timeout,
        I2cErrorKind::AdapterGone => DdcErrorKind::Detached,
        I2cErrorKind::Unsupported => DdcErrorKind::Unsupported,
        _ => DdcErrorKind::Other,
    }
}

/// The errno of an `i2cdev` error.
pub(crate) fn to_raw_errno(e: LinuxI2CError) -> i32 {
    match e {
        LinuxI2CError::Errno(n) => n,
        LinuxI2CError::Io(io) => io.raw_os_error().unwrap_or(0),
    }
}

/// An errno as an [`I2cErrorKind`], following `Documentation/i2c/fault-codes.rst`.
///
/// `DataNack` has no errno of its own and is not produced here; `amdgpu` returns `EIO` for
/// every failure, which is `Unknown { errno: EIO }`.
pub(crate) fn map_errno(errno: i32) -> I2cErrorKind {
    match errno {
        libc::ENXIO => I2cErrorKind::AddressNack,
        libc::EAGAIN => I2cErrorKind::ArbitrationLost,
        libc::ETIMEDOUT => I2cErrorKind::ClockStretchTimeout,
        libc::EBUSY => I2cErrorKind::BusBusy,
        libc::EPROTO => I2cErrorKind::BusProtocolError,
        libc::ESHUTDOWN | libc::ENODEV => I2cErrorKind::AdapterGone,
        libc::EOPNOTSUPP => I2cErrorKind::Unsupported,
        e => I2cErrorKind::Unknown { errno: e },
    }
}
