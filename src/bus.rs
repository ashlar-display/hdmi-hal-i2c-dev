//! [`DdcBus`] implementation over a `/dev/i2c-N` device node, with `I2C_RDWR` or, for an
//! adapter without plain I²C, SMBus calls.
//!
//! [`DdcBus`]: hdmi_hal::ddc::DdcBus

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use i2cdev::core::{I2CMessage, I2CTransfer};
use i2cdev::linux::{LinuxI2CBus, LinuxI2CDevice, LinuxI2CError, LinuxI2CMessage};

use hdmi_hal::ddc::{DdcBus, DdcCapabilities, EddcTarget, MAX_SEGMENT, SCDC, SEGMENT_POINTER};

use crate::adapters::{self, Eagain, Known, SMBUS_LEVEL};
use crate::error::{I2cDevError, I2cErrorKind, Refusal};
use crate::smbus::SmbusBus;
use crate::transport::{map_errno, to_raw_errno};

/// The longest message the kernel's `i2c-dev` takes in one `I2C_RDWR` transfer
/// (`i2c-dev.c`, 8192 bytes). A longer one is refused before the ioctl: `i2cdev` passes
/// lengths as `u16`, so a longer buffer would be cut short silently.
pub const MAX_MESSAGE_LEN: usize = 8192;

/// One message of an `I2C_RDWR` transfer, with its own address.
#[derive(Debug)]
pub(crate) enum Message<'a> {
    /// Write `bytes` to `address`.
    Write { address: u8, bytes: &'a [u8] },
    /// Read `buf.len()` bytes from `address`.
    Read { address: u8, buf: &'a mut [u8] },
}

/// A narrow seam over the one bus operation [`I2cDevBus`] makes: an `I2C_RDWR` transfer of
/// messages, each with its own address, returning how many the kernel executed.
///
/// This trait is crate-private. It exists to allow a mock that records addresses in unit
/// tests, without exposing the seam in the public API.
pub(crate) trait I2cBus {
    fn transfer(&mut self, messages: &mut [Message<'_>]) -> Result<u32, LinuxI2CError>;
}

/// Over `i2cdev`'s `LinuxI2CBus`, not `LinuxI2CDevice`: the device's `transfer` overwrites
/// every message's address with its own and marks same-direction neighbours `NO_START`, so an
/// E-DDC read across 0x30 and 0x50 would become one merged write to a single address.
impl I2cBus for LinuxI2CBus {
    fn transfer(&mut self, messages: &mut [Message<'_>]) -> Result<u32, LinuxI2CError> {
        let mut linux: Vec<LinuxI2CMessage<'_>> = messages
            .iter_mut()
            .map(|message| match message {
                Message::Write { address, bytes } => {
                    LinuxI2CMessage::write(bytes).with_address(u16::from(*address))
                }
                Message::Read { address, buf } => {
                    LinuxI2CMessage::read(buf).with_address(u16::from(*address))
                }
            })
            .collect();
        I2CTransfer::transfer(self, &mut linux)
    }
}

/// The bus behind an [`I2cDevBus`] opened from a device node: `I2C_RDWR` transfers
/// ([`open`](I2cDevBus::open), [`open_with`](I2cDevBus::open_with)) or SMBus calls
/// ([`open_smbus`](I2cDevBus::open_smbus)). Opaque: callers name `I2cDevBus` alone.
#[derive(Debug)]
pub struct Backend(Inner);

#[derive(Debug)]
enum Inner {
    Rdwr(LinuxI2CBus),
    Smbus(SmbusBus<LinuxI2CDevice>),
}

impl I2cBus for Backend {
    fn transfer(&mut self, messages: &mut [Message<'_>]) -> Result<u32, LinuxI2CError> {
        match &mut self.0 {
            Inner::Rdwr(bus) => I2cBus::transfer(bus, messages),
            Inner::Smbus(bus) => I2cBus::transfer(bus, messages),
        }
    }
}

/// A DDC bus over a `/dev/i2c-N` device node: [`DdcBus`] with `I2C_RDWR` transfers, an
/// address per message, so an E-DDC read writes the segment pointer (0x30) and reads the
/// target in one transfer.
///
/// Its [`capabilities`](DdcBus::capabilities) come from the adapter's name
/// ([`open`](Self::open), through the [`adapters`] table) or from the
/// caller ([`open_with`](Self::open_with)): what an adapter really does with a transfer is
/// not something the kernel reports. An operation the capabilities rule out is refused before
/// any ioctl ([`I2cDevError::Refused`], kind `Unsupported`), as the `DdcBus` contract asks.
///
/// Every operation is one `I2C_RDWR` ioctl, one transaction: the kernel holds the adapter
/// for it. A `read_segmented` the capabilities split into pieces
/// ([`DdcCapabilities::segmented_piece`]) is one transfer per piece. The crate adds no
/// retries; the kernel's I²C core retries `EAGAIN` up to the adapter's `retries`.
///
/// Opened with [`open_smbus`](Self::open_smbus), every operation is one SMBus call
/// instead, for an adapter without plain I²C such as `i2c-stub`.
///
/// [`DdcBus`]: hdmi_hal::ddc::DdcBus
// `I2cBus` is `pub(crate)`, a test seam: external callers can only use the default.
#[allow(private_bounds)]
#[derive(Debug)]
pub struct I2cDevBus<B: I2cBus = Backend> {
    bus: Mutex<B>,
    /// Opened with `open_smbus`: `ENODEV` is no chip at the address.
    smbus: bool,
    capabilities: DdcCapabilities,
    /// The adapter's sysfs name, when `open` read it.
    adapter: Option<String>,
    /// The table's entry, when the adapter is in it: how its errnos read.
    known: Option<Known>,
}

impl I2cDevBus<Backend> {
    /// Open the given `/dev/i2c-N` device as a DDC bus, with the capabilities the
    /// [`adapters`] table gives for the adapter's name
    /// (`/sys/class/i2c-dev/i2c-N/name`), or [`SMBUS_LEVEL`] for an adapter not in it or a
    /// name that cannot be read.
    ///
    /// For an adapter in the table, its errnos read as its driver uses them: `EAGAIN` is a
    /// timeout where the driver returns it for one (dw-hdmi, Inno, i915 after a GMBUS
    /// timeout), and `EINVAL`, with which these drivers refuse what they cannot do, is
    /// `Unsupported`.
    ///
    /// # Errors
    ///
    /// [`I2cDevError::DeviceOpenFailed`] if the device cannot be opened.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, I2cDevError> {
        Self::open_in(Path::new("/sys/class/i2c-dev"), path.as_ref())
    }

    /// [`open`](Self::open), with the adapter's name read under `sysfs_root` instead of
    /// `/sys/class/i2c-dev`. Used in unit tests.
    pub(crate) fn open_in(sysfs_root: &Path, path: &Path) -> Result<Self, I2cDevError> {
        let adapter = adapter_name(sysfs_root, path);
        let known = adapter.as_deref().and_then(adapters::known);
        let capabilities = known.map_or(SMBUS_LEVEL, |known| known.capabilities);
        let mut bus = Self::open_with(path, capabilities)?;
        bus.adapter = adapter;
        bus.known = known;
        Ok(bus)
    }

    /// Open the given `/dev/i2c-N` device as a DDC bus with `capabilities`, the caller's
    /// assertion of what the adapter does with a transfer.
    ///
    /// The calling process must have read/write permission on the device node.
    ///
    /// # Errors
    ///
    /// [`I2cDevError::DeviceOpenFailed`] if the device cannot be opened.
    pub fn open_with(
        path: impl AsRef<Path>,
        capabilities: DdcCapabilities,
    ) -> Result<Self, I2cDevError> {
        let path = path.as_ref();
        let bus = LinuxI2CBus::new(path).map_err(|e| I2cDevError::DeviceOpenFailed {
            path: path.to_path_buf(),
            source: e.into(),
        })?;
        Ok(Self::from_bus(Backend(Inner::Rdwr(bus)), capabilities))
    }

    /// Open the given `/dev/i2c-N` device as a DDC bus over SMBus calls, for an adapter
    /// without plain I²C (`I2C_RDWR`), such as `i2c-stub`. Its capabilities are
    /// [`SMBUS_LEVEL`]: `write` of up to 33 bytes, `write_read` of one byte and up to 32,
    /// `read` of one byte, E-DDC segment 0 in 32-byte pieces, each an SMBus call: a write of
    /// one byte, a byte-data write, an I²C block write; a byte-data read or an I²C block read;
    /// a byte read.
    ///
    /// SMBus addresses a device per file descriptor, so the bus binds each address it uses
    /// as it changes (`I2C_SLAVE`): an address a kernel driver holds fails with `Busy`. An
    /// `ENODEV` is no chip at the address, as `i2c-stub` reports it:
    /// `NoAcknowledge(Address)`.
    ///
    /// # Errors
    ///
    /// [`I2cDevError::DeviceOpenFailed`] if the device cannot be opened or bound to the
    /// SCDC address it starts at.
    pub fn open_smbus(path: impl AsRef<Path>) -> Result<Self, I2cDevError> {
        let path = path.as_ref();
        let device = LinuxI2CDevice::new(path, u16::from(SCDC)).map_err(|e| {
            I2cDevError::DeviceOpenFailed {
                path: path.to_path_buf(),
                source: e.into(),
            }
        })?;
        let mut bus = Self::from_bus(
            Backend(Inner::Smbus(SmbusBus::new(device, Some(SCDC)))),
            SMBUS_LEVEL,
        );
        bus.smbus = true;
        bus.adapter = adapter_name(Path::new("/sys/class/i2c-dev"), path);
        Ok(bus)
    }
}

#[allow(private_bounds)]
impl<B: I2cBus> I2cDevBus<B> {
    pub(crate) fn from_bus(bus: B, capabilities: DdcCapabilities) -> Self {
        Self {
            bus: Mutex::new(bus),
            smbus: false,
            capabilities,
            adapter: None,
            known: None,
        }
    }

    /// A bus over SMBus calls through `bus`, as `open_smbus` opens one. Used in unit tests.
    #[cfg(test)]
    pub(crate) fn from_smbus(bus: B) -> Self {
        let mut bus = Self::from_bus(bus, SMBUS_LEVEL);
        bus.smbus = true;
        bus
    }

    /// The adapter's sysfs name, as [`open`](I2cDevBus::open) read it; `None` when it could
    /// not be read, or the bus was opened with [`open_with`](I2cDevBus::open_with).
    pub fn adapter(&self) -> Option<&str> {
        self.adapter.as_deref()
    }

    /// An errno as a class, as this adapter's driver uses it.
    fn errno_kind(&self, errno: i32) -> I2cErrorKind {
        match (errno, self.known) {
            (
                libc::EAGAIN,
                Some(Known {
                    eagain: Eagain::Timeout,
                    ..
                }),
            ) => I2cErrorKind::ClockStretchTimeout,
            (libc::EINVAL, Some(_)) => I2cErrorKind::Unsupported,
            (libc::ENODEV, _) if self.smbus => I2cErrorKind::NoDevice,
            _ => map_errno(errno),
        }
    }

    /// The bus, recovered if a previous holder panicked: the file descriptor is still good,
    /// and a transfer that was cut short is the kernel's to have finished or not.
    fn bus(&self) -> MutexGuard<'_, B> {
        self.bus
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// One `I2C_RDWR` transfer of `messages`, which must all run.
    fn transfer(&self, messages: &mut [Message<'_>]) -> Result<(), I2cDevError> {
        for message in messages.iter() {
            let len = match message {
                Message::Write { bytes, .. } => bytes.len(),
                Message::Read { buf, .. } => buf.len(),
            };
            if len > MAX_MESSAGE_LEN {
                return Err(refused(Refusal::TooLong {
                    len,
                    max: MAX_MESSAGE_LEN,
                }));
            }
        }
        let address = match &messages[0] {
            Message::Write { address, .. } | Message::Read { address, .. } => *address,
        };
        let sent = u32::try_from(messages.len()).unwrap_or(u32::MAX);
        let done = self
            .bus()
            .transfer(messages)
            .map_err(|e| I2cDevError::Transfer {
                address,
                kind: self.errno_kind(to_raw_errno(e)),
            })?;
        if done < sent {
            return Err(I2cDevError::IncompleteTransfer { sent, done });
        }
        Ok(())
    }

    fn check_address(&self, address: u8) -> Result<(), I2cDevError> {
        if self.capabilities.reaches(address) {
            Ok(())
        } else {
            Err(refused(Refusal::Unreachable { address }))
        }
    }
}

/// The sysfs name of the adapter behind `path` (`…/i2c-N`), from `sysfs_root/i2c-N/name`.
fn adapter_name(sysfs_root: &Path, path: &Path) -> Option<String> {
    let file = path.file_name()?.to_str()?;
    let index = file.strip_prefix("i2c-")?;
    if index.is_empty() || !index.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let name: PathBuf = sysfs_root.join(file).join("name");
    let name = std::fs::read_to_string(name).ok()?;
    Some(name.trim_end_matches('\n').to_string())
}

fn refused(reason: Refusal) -> I2cDevError {
    I2cDevError::Refused { reason }
}

fn check_len(len: usize, max: u16) -> Result<(), I2cDevError> {
    if len > usize::from(max) {
        Err(refused(Refusal::TooLong {
            len,
            max: usize::from(max),
        }))
    } else {
        Ok(())
    }
}

#[allow(private_bounds)]
impl<B: I2cBus> DdcBus for I2cDevBus<B> {
    type Error = I2cDevError;

    fn capabilities(&self) -> DdcCapabilities {
        self.capabilities
    }

    fn write(&self, addr: u8, bytes: &[u8]) -> Result<(), I2cDevError> {
        if bytes.is_empty() {
            return Err(refused(Refusal::EmptyWrite));
        }
        self.check_address(addr)?;
        check_len(bytes.len(), self.capabilities.write_max())?;
        self.transfer(&mut [Message::Write {
            address: addr,
            bytes,
        }])
    }

    fn write_read(&self, addr: u8, bytes: &[u8], buf: &mut [u8]) -> Result<(), I2cDevError> {
        if bytes.is_empty() {
            return self.read(addr, buf);
        }
        if buf.is_empty() {
            return Ok(());
        }
        self.check_address(addr)?;
        let (write_max, read_max) = self.capabilities.write_read_max();
        check_len(bytes.len(), write_max)?;
        check_len(buf.len(), read_max)?;
        self.transfer(&mut [
            Message::Write {
                address: addr,
                bytes,
            },
            Message::Read { address: addr, buf },
        ])
    }

    fn read(&self, addr: u8, buf: &mut [u8]) -> Result<(), I2cDevError> {
        if buf.is_empty() {
            return Ok(());
        }
        self.check_address(addr)?;
        check_len(buf.len(), self.capabilities.read_max())?;
        self.transfer(&mut [Message::Read { address: addr, buf }])
    }

    fn read_segmented(
        &self,
        target: EddcTarget,
        segment: u8,
        offset: u8,
        buf: &mut [u8],
    ) -> Result<(), I2cDevError> {
        if buf.is_empty() {
            return Ok(());
        }
        let address = target.address();
        if !self.capabilities.reads_target(target) {
            return Err(refused(Refusal::Target { address }));
        }
        if segment > MAX_SEGMENT
            || self
                .capabilities
                .max_segment()
                .is_none_or(|max| segment > max)
        {
            return Err(refused(Refusal::Segment { segment }));
        }
        if usize::from(offset) + buf.len() > 256 {
            return Err(refused(Refusal::PastSegmentEnd {
                offset,
                len: buf.len(),
            }));
        }
        let piece = self
            .capabilities
            .segmented_piece()
            .map_or(buf.len(), |piece| usize::from(piece).max(1));
        let pointer = [segment];
        for (i, chunk) in buf.chunks_mut(piece).enumerate() {
            // `offset + buf.len()` is at most 256, so every piece's offset fits a byte.
            let at = [offset.wrapping_add((i * piece) as u8)];
            let mut messages = [
                Message::Write {
                    address: SEGMENT_POINTER,
                    bytes: &pointer,
                },
                Message::Write {
                    address,
                    bytes: &at,
                },
                Message::Read {
                    address,
                    buf: chunk,
                },
            ];
            // Segment 0 needs no pointer write (E-DDC 8.1.2): a display without one answers.
            let skip = usize::from(segment == 0);
            self.transfer(&mut messages[skip..])?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "bus_tests.rs"]
mod tests;
