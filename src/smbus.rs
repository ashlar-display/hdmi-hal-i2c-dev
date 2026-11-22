//! The SMBus backend of [`I2cDevBus`](crate::I2cDevBus): its transfers as the SMBus calls of
//! an adapter without plain I²C, such as `i2c-stub`.

use i2cdev::core::I2CDevice;
use i2cdev::linux::{LinuxI2CDevice, LinuxI2CError};

use crate::bus::{I2cBus, Message};

/// The longest SMBus I²C block (`I2C_SMBUS_BLOCK_MAX`).
const BLOCK_MAX: usize = 32;

/// A narrow seam over the SMBus calls the backend makes, on a device bound to one address
/// at a time. Crate-private: it exists for a mock register file in unit tests.
pub(crate) trait Smbus {
    fn set_address(&mut self, address: u16) -> Result<(), LinuxI2CError>;
    fn read_byte(&mut self) -> Result<u8, LinuxI2CError>;
    fn write_byte(&mut self, value: u8) -> Result<(), LinuxI2CError>;
    fn read_byte_data(&mut self, register: u8) -> Result<u8, LinuxI2CError>;
    fn write_byte_data(&mut self, register: u8, value: u8) -> Result<(), LinuxI2CError>;
    fn read_i2c_block(&mut self, register: u8, len: u8) -> Result<Vec<u8>, LinuxI2CError>;
    fn write_i2c_block(&mut self, register: u8, values: &[u8]) -> Result<(), LinuxI2CError>;
}

impl Smbus for LinuxI2CDevice {
    fn set_address(&mut self, address: u16) -> Result<(), LinuxI2CError> {
        self.set_slave_address(address)
    }

    fn read_byte(&mut self) -> Result<u8, LinuxI2CError> {
        I2CDevice::smbus_read_byte(self)
    }

    fn write_byte(&mut self, value: u8) -> Result<(), LinuxI2CError> {
        I2CDevice::smbus_write_byte(self, value)
    }

    fn read_byte_data(&mut self, register: u8) -> Result<u8, LinuxI2CError> {
        I2CDevice::smbus_read_byte_data(self, register)
    }

    fn write_byte_data(&mut self, register: u8, value: u8) -> Result<(), LinuxI2CError> {
        I2CDevice::smbus_write_byte_data(self, register, value)
    }

    fn read_i2c_block(&mut self, register: u8, len: u8) -> Result<Vec<u8>, LinuxI2CError> {
        I2CDevice::smbus_read_i2c_block_data(self, register, len)
    }

    fn write_i2c_block(&mut self, register: u8, values: &[u8]) -> Result<(), LinuxI2CError> {
        I2CDevice::smbus_write_i2c_block_data(self, register, values)
    }
}

/// Transfers as SMBus calls, on a device re-bound to each address as it changes (SMBus
/// addresses a device per file descriptor).
///
/// | Transfer | SMBus call |
/// |---|---|
/// | write of 1 byte | write byte |
/// | write of 2 bytes | write byte data (offset, value) |
/// | write of 3–33 bytes | I²C block write (offset, up to 32 values) |
/// | write of 1 byte, read of 1 | read byte data |
/// | write of 1 byte, read of 2–32 | I²C block read |
/// | read of 1 byte | read byte |
///
/// Any other shape is `EOPNOTSUPP`; [`SMBUS_LEVEL`](crate::adapters::SMBUS_LEVEL) rules
/// them all out before they reach here. A block read that returns another length than asked
/// is `EIO`: the buffer is left as it was.
#[derive(Debug)]
pub(crate) struct SmbusBus<S> {
    device: S,
    address: Option<u8>,
}

impl<S: Smbus> SmbusBus<S> {
    /// Over `device`, bound to `address` (or to none yet).
    pub(crate) fn new(device: S, address: Option<u8>) -> Self {
        Self { device, address }
    }

    fn bind(&mut self, address: u8) -> Result<&mut S, LinuxI2CError> {
        if self.address != Some(address) {
            self.device.set_address(u16::from(address))?;
            self.address = Some(address);
        }
        Ok(&mut self.device)
    }
}

fn unsupported() -> LinuxI2CError {
    LinuxI2CError::Errno(libc::EOPNOTSUPP)
}

impl<S: Smbus> I2cBus for SmbusBus<S> {
    fn transfer(&mut self, messages: &mut [Message<'_>]) -> Result<u32, LinuxI2CError> {
        match messages {
            [Message::Write { address, bytes }] => {
                let device = self.bind(*address)?;
                match **bytes {
                    [value] => device.write_byte(value)?,
                    [register, value] => device.write_byte_data(register, value)?,
                    [register, ref values @ ..] if values.len() <= BLOCK_MAX => {
                        device.write_i2c_block(register, values)?;
                    }
                    _ => return Err(unsupported()),
                }
            }
            [
                Message::Write { address, bytes },
                Message::Read { address: to, buf },
            ] if address == to && bytes.len() == 1 && (1..=BLOCK_MAX).contains(&buf.len()) => {
                let register = bytes[0];
                let device = self.bind(*address)?;
                if buf.len() == 1 {
                    buf[0] = device.read_byte_data(register)?;
                } else {
                    // At most 32 here, so it fits the `u8` the call takes.
                    let block = device.read_i2c_block(register, buf.len() as u8)?;
                    if block.len() != buf.len() {
                        return Err(LinuxI2CError::Errno(libc::EIO));
                    }
                    buf.copy_from_slice(&block);
                }
            }
            [Message::Read { address, buf }] if buf.len() == 1 => {
                buf[0] = self.bind(*address)?.read_byte()?;
            }
            _ => return Err(unsupported()),
        }
        Ok(u32::try_from(messages.len()).unwrap_or(u32::MAX))
    }
}

#[cfg(test)]
#[path = "smbus_tests.rs"]
mod tests;
