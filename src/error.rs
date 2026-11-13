//! Error types for all fallible operations in this crate.

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
    /// An I²C transaction failed.
    Transaction(I2cTransactionError),
    /// A block read asked for more bytes than one SMBus I2C block read can carry
    /// ([`MAX_BLOCK_LEN`](crate::transport::MAX_BLOCK_LEN)).
    /// Nothing was read: the read is never split into several transactions. A caller
    /// that needs more reads it in parts, knowing they are separate transactions.
    #[non_exhaustive]
    BlockReadTooLong {
        /// The first register of the block.
        register: u8,
        /// The length asked for.
        len: usize,
        /// The most one block read can carry.
        max: usize,
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
    /// The kernel returned another number of bytes than the block read asked for. The
    /// buffer is left unchanged rather than partly filled or padded.
    ///
    /// The kernel's SMBus emulation returns the length asked for, so this is rare: it
    /// comes from an adapter driver that reports another length, or from `i2c-stub`,
    /// which ends a block read at register 0xFF (a 32-byte read from 0xF0 returns 16
    /// bytes). A driver that reports more than 33 bytes does not reach this error:
    /// `i2cdev` panics on it, while the transport's lock is held, and every later call on
    /// the transport then panics too; open a new one.
    #[non_exhaustive]
    UnexpectedBlockLength {
        /// The first register of the block.
        register: u8,
        /// The length asked for.
        requested: usize,
        /// The length returned.
        returned: usize,
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

/// The phase and kind of an I²C transaction failure.
#[non_exhaustive]
#[derive(Debug)]
pub struct I2cTransactionError {
    /// Which message in the compound transaction failed, as far as this crate can tell:
    /// always [`MessagePhase::Write`] here (see [`MessagePhase`]).
    pub phase: MessagePhase,
    /// The class of bus condition that caused the failure.
    pub kind: I2cErrorKind,
}

/// Which message in the compound SCDC transaction failed.
///
/// [`I2cDevTransport`](crate::I2cDevTransport) always reports `Write`, for reads and block
/// reads too: each operation is one `I2C_SMBUS` ioctl, and its error does not say which
/// message of the transaction failed. Treat the phase as unknown here; `register` is the
/// register the operation started at. `Read` is for a backend that can tell, such as one
/// issuing `I2C_RDWR` messages and isolating the failing one.
#[non_exhaustive]
#[derive(Debug)]
pub enum MessagePhase {
    /// The write-phase message failed (register address byte, or write data).
    Write {
        /// The SCDC register address being accessed when the failure occurred.
        register: u8,
    },
    /// The read-phase message failed (data retrieval).
    Read {
        /// The SCDC register address being accessed when the failure occurred.
        register: u8,
    },
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
    /// The adapter does not support the operation (`EOPNOTSUPP`). For a block read, the
    /// adapter most likely lacks the SMBus I2C block read (`I2C_FUNC_SMBUS_READ_I2C_BLOCK`),
    /// as an SMBus-only adapter may; open the transport with
    /// [`I2cDevTransport::open_bytewise`](crate::I2cDevTransport::open_bytewise) to read a
    /// register at a time instead.
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
            Self::Transaction(e) => write!(f, "I²C transaction failed: {e}"),
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
            Self::BlockReadTooLong { register, len, max } => write!(
                f,
                "block read of {len} bytes from register 0x{register:02x} exceeds the {max}-byte limit"
            ),
            Self::UnexpectedBlockLength {
                register,
                requested,
                returned,
            } => write!(
                f,
                "block read from register 0x{register:02x} returned {returned} bytes, not {requested}"
            ),
        }
    }
}

impl std::error::Error for I2cDevError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::DeviceOpenFailed { source, .. } => Some(source),
            Self::Transaction(e) => Some(e),
            _ => None,
        }
    }
}

impl std::fmt::Display for I2cTransactionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let phase = match &self.phase {
            MessagePhase::Write { register } => format!("write phase (register 0x{register:02x})"),
            MessagePhase::Read { register } => format!("read phase (register 0x{register:02x})"),
        };
        write!(f, "{phase}: {:?}", self.kind)
    }
}

impl std::error::Error for I2cTransactionError {}

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;

/// The classification `hdmi-hal` asks of a transport error, so that a caller such as a
/// procedure polling the sink can tell a failure that may pass from one that will not.
///
/// - A transaction maps its [`I2cErrorKind`]: `ENXIO` ([`AddressNack`](I2cErrorKind::AddressNack))
///   to `NoAcknowledge(Unknown)`, since i915 returns it for a NAK on any byte; a NACKed data
///   byte to `NoAcknowledge(Data)`; lost arbitration and a busy bus to `Busy`; a clock-stretch
///   timeout to `Timeout`; an adapter suspended or removed (`ESHUTDOWN`, `ENODEV`) to
///   `Detached`; `EOPNOTSUPP` to `Unsupported`; a protocol error and an unknown errno to
///   `Other`.
/// - A block read longer than one transaction can carry is `Unsupported`: it would be refused
///   again. A block of another length than asked is `Other`.
/// - The errors of opening a transport are `Other`; they come before any transaction.
impl DdcError for I2cDevError {
    fn kind(&self) -> DdcErrorKind {
        match self {
            Self::Transaction(e) => e.kind(),
            Self::Transfer { kind, .. } => ddc_kind(kind),
            Self::BlockReadTooLong { .. } | Self::Refused { .. } => DdcErrorKind::Unsupported,
            _ => DdcErrorKind::Other,
        }
    }
}

/// The kind of the failed transaction, as for [`I2cDevError`].
impl DdcError for I2cTransactionError {
    fn kind(&self) -> DdcErrorKind {
        ddc_kind(&self.kind)
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
