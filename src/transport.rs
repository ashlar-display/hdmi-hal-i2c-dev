//! [`ScdcTransport`] implementation backed by a `/dev/i2c-N` device node.
//!
//! [`ScdcTransport`]: hdmi_hal::scdc::ScdcTransport

use std::path::Path;
use std::sync::Mutex;

use i2cdev::core::I2CDevice;
use i2cdev::linux::{LinuxI2CDevice, LinuxI2CError};

use hdmi_hal::scdc::{ScdcTransport, read_block_bytewise};

use crate::error::{I2cDevError, I2cErrorKind, I2cTransactionError, MessagePhase};

/// SCDC slave address, fixed by HDMI 2.1.
const SCDC_ADDRESS: u16 = 0x54;

/// The longest [`I2cDevTransport`] block read
/// ([`read_block`](ScdcTransport::read_block)), opened with
/// [`open`](I2cDevTransport::open): 32 bytes, the SMBus block maximum
/// (`I2C_SMBUS_BLOCK_MAX`). A longer read is refused, never split. A transport opened with
/// [`open_bytewise`](I2cDevTransport::open_bytewise) has no limit.
pub const MAX_BLOCK_LEN: usize = 32;

/// A narrow seam over the three SMBus operations [`I2cDevTransport`] requires.
///
/// This trait is crate-private. It exists solely to allow a lightweight mock
/// in unit tests without exposing the seam in the public API.
pub(crate) trait SmbusDevice {
    fn smbus_read_byte_data(&mut self, reg: u8) -> Result<u8, LinuxI2CError>;
    fn smbus_write_byte_data(&mut self, reg: u8, value: u8) -> Result<(), LinuxI2CError>;
    fn smbus_read_i2c_block_data(&mut self, reg: u8, len: u8) -> Result<Vec<u8>, LinuxI2CError>;
}

impl SmbusDevice for LinuxI2CDevice {
    fn smbus_read_byte_data(&mut self, reg: u8) -> Result<u8, LinuxI2CError> {
        I2CDevice::smbus_read_byte_data(self, reg)
    }

    fn smbus_write_byte_data(&mut self, reg: u8, value: u8) -> Result<(), LinuxI2CError> {
        I2CDevice::smbus_write_byte_data(self, reg, value)
    }

    fn smbus_read_i2c_block_data(&mut self, reg: u8, len: u8) -> Result<Vec<u8>, LinuxI2CError> {
        I2CDevice::smbus_read_i2c_block_data(self, reg, len)
    }
}

/// An SCDC transport backed by a `/dev/i2c-N` device node.
///
/// Implements [`ScdcTransport`] by issuing SMBus operations (`I2C_SMBUS`) to the
/// SCDC slave address (0x54) on the specified adapter: byte-data reads and writes,
/// and I²C block reads for [`read_block`](ScdcTransport::read_block). Each read is one
/// ioctl asking for a combined write-then-read — the register offset written, a repeated
/// START, the bytes read — the transaction an SCDC read is on the wire.
///
/// # Block reads
///
/// Opened with [`open`](Self::open), `read_block` is one SMBus I2C block read
/// (`I2C_SMBUS_I2C_BLOCK_DATA`): one ioctl, handed to the kernel whole. What reaches the
/// wire is the adapter driver's, not this crate's:
///
/// - Checked to put it on the wire as one transaction: the DRM DDC adapters of `i915`
///   (GMBUS) and `amdgpu` (DC), through the kernel's SMBus emulation (the offset write and
///   a repeated-START read). `amdgpu` was also checked on hardware.
/// - Known to split it: the Synopsys DesignWare HDMI controller's I²C master
///   (`dw-hdmi`, and `dw-hdmi-qp`; i.MX, Rockchip, Allwinner and Amlogic boards) reads
///   one byte per hardware operation, each with its own offset write, while advertising
///   the block read. There `read_block` returns `Ok`, but its registers were read one at a
///   time; only the CED checksum (culvert) can show a torn read.
/// - Others are unchecked.
///
/// Its limit is [`MAX_BLOCK_LEN`], 32 bytes (`I2C_SMBUS_BLOCK_MAX`): a longer buffer is
/// [`I2cDevError::BlockReadTooLong`], before any ioctl, never split. There is no fallback
/// to one register at a time: an adapter without the SMBus I2C block read
/// (`I2C_FUNC_SMBUS_READ_I2C_BLOCK`, as on some SMBus-only adapters) fails the ioctl,
/// typically with [`I2cErrorKind::Unsupported`], and the error is returned. On such an
/// adapter, open the transport with [`open_bytewise`](Self::open_bytewise) instead.
///
/// Past register 0xFF the result is the sink's on real hardware; `i2c-stub` ends the read
/// at 0xFF, which is [`I2cDevError::UnexpectedBlockLength`].
///
/// # Error recovery
///
/// A [`I2cDevError::Transaction`] error does not invalidate the transport. The
/// underlying file descriptor remains open and subsequent calls may succeed.
/// If errors persist, the sink has most likely disconnected; drop and
/// reconstruct the transport via [`I2cDevTransport::open`].
///
/// [`ScdcTransport`]: hdmi_hal::scdc::ScdcTransport
// `SmbusDevice` is intentionally `pub(crate)`: it is a test seam, not a
// public extension point. The bound on `D` is therefore more private than the
// struct itself, which the compiler warns about. The warning is expected and
// correct here — external callers cannot name `SmbusDevice` and therefore
// cannot instantiate `I2cDevTransport<D>` with any `D` other than the default
// `LinuxI2CDevice`. That is exactly the desired constraint.
#[allow(private_bounds)]
#[derive(Debug)]
pub struct I2cDevTransport<D: SmbusDevice = LinuxI2CDevice> {
    device: Mutex<D>,
    block_reads: BlockReads,
}

/// How [`I2cDevTransport::read_block`](ScdcTransport::read_block) reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockReads {
    /// One SMBus I2C block read ([`I2cDevTransport::open`]).
    Smbus,
    /// One byte-data read per register ([`I2cDevTransport::open_bytewise`]).
    Bytewise,
}

impl I2cDevTransport<LinuxI2CDevice> {
    /// Open the given `/dev/i2c-N` device for use as an SCDC transport, with block reads
    /// as SMBus I2C block reads (see "Block reads").
    ///
    /// The calling process must have read/write permission on the device node
    /// (typically via membership in the `i2c` group, or by running as root).
    ///
    /// # Errors
    ///
    /// Returns [`I2cDevError::DeviceOpenFailed`] if the device cannot be opened.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, I2cDevError> {
        Self::open_with(path.as_ref(), BlockReads::Smbus)
    }

    /// Open the given `/dev/i2c-N` device with block reads one register at a time: for an
    /// adapter without the SMBus I2C block read, where [`open`](Self::open)'s block reads
    /// fail ([`I2cErrorKind::Unsupported`]).
    ///
    /// **Its block reads are not one transaction.** `read_block` is
    /// [`read_block_bytewise`]: one byte-data read, its own ioctl, per register, so a value
    /// the sink updates between two of them can be read half before and half after the
    /// update, and another caller's transaction can land in between. culvert's CED checksum
    /// catches most such tears in the error counters; the LTP requests and the RS count have
    /// no checksum. There is no length limit, a read past 0xFF wraps to 0x00, and on an
    /// error the bytes before the failing register are already in `buf`.
    ///
    /// Use [`open`](Self::open) wherever the block read works; this is the explicit
    /// choice for an adapter where it does not.
    ///
    /// # Errors
    ///
    /// Returns [`I2cDevError::DeviceOpenFailed`] if the device cannot be opened.
    pub fn open_bytewise(path: impl AsRef<Path>) -> Result<Self, I2cDevError> {
        Self::open_with(path.as_ref(), BlockReads::Bytewise)
    }

    fn open_with(path: &Path, block_reads: BlockReads) -> Result<Self, I2cDevError> {
        let device =
            LinuxI2CDevice::new(path, SCDC_ADDRESS).map_err(|e| I2cDevError::DeviceOpenFailed {
                path: path.to_path_buf(),
                source: e.into(),
            })?;
        Ok(Self {
            device: Mutex::new(device),
            block_reads,
        })
    }
}

// Same reasoning as the struct: `SmbusDevice` is `pub(crate)` by design, so
// the bound is more private than the impl block. Suppressed here for the same
// reason — this impl exists solely for tests and is not part of the public API.
#[cfg(test)]
#[allow(private_bounds)]
impl<D: SmbusDevice> I2cDevTransport<D> {
    /// Construct a transport from an already-opened device.
    ///
    /// Intended for unit tests; production code should use [`I2cDevTransport::open`].
    pub(crate) fn from_device(device: D) -> Self {
        Self {
            device: Mutex::new(device),
            block_reads: BlockReads::Smbus,
        }
    }

    /// As [`from_device`](Self::from_device), with block reads one register at a time, as
    /// [`I2cDevTransport::open_bytewise`] opens it.
    pub(crate) fn from_device_bytewise(device: D) -> Self {
        Self {
            device: Mutex::new(device),
            block_reads: BlockReads::Bytewise,
        }
    }
}

impl<D: SmbusDevice> ScdcTransport for I2cDevTransport<D> {
    type Error = I2cDevError;

    /// Read one byte from the given SCDC register.
    ///
    /// Issues an SMBus byte-data read (`I2C_SMBUS`): a combined write of the
    /// register address followed by a read of one byte, in a single ioctl
    /// call. The kernel holds the bus for the duration, making the read atomic.
    fn read(&self, reg: u8) -> Result<u8, I2cDevError> {
        let mut device = self
            .device
            .lock()
            .expect("I2cDevTransport device mutex was poisoned");
        device.smbus_read_byte_data(reg).map_err(|e| {
            I2cDevError::Transaction(I2cTransactionError {
                phase: MessagePhase::Write { register: reg },
                kind: map_errno(to_raw_errno(e)),
            })
        })
    }

    /// Write one byte to the given SCDC register.
    ///
    /// Issues an SMBus byte-data write (`I2C_SMBUS`): a single write of the
    /// register address byte followed by the value byte.
    fn write(&mut self, reg: u8, value: u8) -> Result<(), I2cDevError> {
        let mut device = self
            .device
            .lock()
            .expect("I2cDevTransport device mutex was poisoned");
        device.smbus_write_byte_data(reg, value).map_err(|e| {
            I2cDevError::Transaction(I2cTransactionError {
                phase: MessagePhase::Write { register: reg },
                kind: map_errno(to_raw_errno(e)),
            })
        })
    }

    /// Read `buf.len()` consecutive registers, starting at `reg`.
    ///
    /// Opened with [`open`](I2cDevTransport::open): one SMBus I2C block read (`I2C_SMBUS`,
    /// `I2C_SMBUS_I2C_BLOCK_DATA`), one ioctl asking for the register offset written, a
    /// repeated START, then `buf.len()` bytes read. The kernel holds the adapter for the
    /// duration; whether the adapter driver puts it on the wire as one transaction is the
    /// driver's (see the type's "Block reads").
    ///
    /// Opened with [`open_bytewise`](I2cDevTransport::open_bytewise): one byte-data read
    /// per register ([`read_block_bytewise`]), not one transaction; the rules below are
    /// [`open`](I2cDevTransport::open)'s.
    ///
    /// - An empty `buf` is `Ok` without an ioctl.
    /// - A `buf` longer than [`MAX_BLOCK_LEN`] is
    ///   [`I2cDevError::BlockReadTooLong`], without an ioctl: the read is never split.
    /// - A result of another length than requested is
    ///   [`I2cDevError::UnexpectedBlockLength`], never padded.
    /// - A failed ioctl is [`I2cDevError::Transaction`], mapped as for `read`; on any
    ///   error, `buf` is unchanged.
    fn read_block(&self, reg: u8, buf: &mut [u8]) -> Result<(), I2cDevError> {
        if self.block_reads == BlockReads::Bytewise {
            return read_block_bytewise(self, reg, buf);
        }
        if buf.is_empty() {
            return Ok(());
        }
        if buf.len() > MAX_BLOCK_LEN {
            return Err(I2cDevError::BlockReadTooLong {
                register: reg,
                len: buf.len(),
                max: MAX_BLOCK_LEN,
            });
        }
        let mut device = self
            .device
            .lock()
            .expect("I2cDevTransport device mutex was poisoned");
        // `buf.len()` is at most 32 here, so it fits the `u8` the ioctl takes.
        let bytes = device
            .smbus_read_i2c_block_data(reg, buf.len() as u8)
            .map_err(|e| {
                I2cDevError::Transaction(I2cTransactionError {
                    phase: MessagePhase::Write { register: reg },
                    kind: map_errno(to_raw_errno(e)),
                })
            })?;
        if bytes.len() != buf.len() {
            return Err(I2cDevError::UnexpectedBlockLength {
                register: reg,
                requested: buf.len(),
                returned: bytes.len(),
            });
        }
        buf.copy_from_slice(&bytes);
        Ok(())
    }
}

/// Extract a raw errno integer from a [`LinuxI2CError`].
fn to_raw_errno(e: LinuxI2CError) -> i32 {
    match e {
        LinuxI2CError::Errno(n) => n,
        LinuxI2CError::Io(io) => io.raw_os_error().unwrap_or(0),
    }
}

/// Map a raw errno to an [`I2cErrorKind`] variant, following the conventions
/// in `Documentation/i2c/fault-codes.rst`.
///
/// `DataNack` has no standard errno mapping and is not produced by this
/// function; it falls through to `Unknown`.
///
/// On `amdgpu`, all failures produce `EIO` regardless of the actual bus
/// condition, so `Unknown { errno: EIO }` is the only variant reachable on
/// that hardware. See the crate documentation for details.
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

#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;
