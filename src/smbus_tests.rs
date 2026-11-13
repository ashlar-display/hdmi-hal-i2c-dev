use std::sync::{Arc, Mutex};

use hdmi_hal::ddc::{DdcBus, DdcError, DdcErrorKind, EDID, EddcTarget, NoAcknowledgeSource, SCDC};

use super::*;
use crate::I2cDevBus;
use crate::error::{I2cDevError, Refusal};

/// One SMBus call the mock received.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Call {
    Bind(u16),
    ReadByte,
    WriteByte(u8),
    ReadByteData(u8),
    WriteByteData(u8, u8),
    ReadBlock(u8, u8),
    WriteBlock(u8, Vec<u8>),
}

/// A register file per address, recording each call; binding `missing` fails with `ENODEV`,
/// and a block read can be cut short.
struct Chip {
    calls: Arc<Mutex<Vec<Call>>>,
    regs: [[u8; 256]; 128],
    bound: u16,
    missing: Option<u16>,
    short_blocks: bool,
}

impl Smbus for Chip {
    fn set_address(&mut self, address: u16) -> Result<(), LinuxI2CError> {
        self.calls.lock().unwrap().push(Call::Bind(address));
        if self.missing == Some(address) {
            return Err(LinuxI2CError::Errno(libc::ENODEV));
        }
        self.bound = address;
        Ok(())
    }
    fn read_byte(&mut self) -> Result<u8, LinuxI2CError> {
        self.calls.lock().unwrap().push(Call::ReadByte);
        Ok(self.regs[usize::from(self.bound)][0])
    }
    fn write_byte(&mut self, value: u8) -> Result<(), LinuxI2CError> {
        self.calls.lock().unwrap().push(Call::WriteByte(value));
        Ok(())
    }
    fn read_byte_data(&mut self, register: u8) -> Result<u8, LinuxI2CError> {
        self.calls
            .lock()
            .unwrap()
            .push(Call::ReadByteData(register));
        Ok(self.regs[usize::from(self.bound)][usize::from(register)])
    }
    fn write_byte_data(&mut self, register: u8, value: u8) -> Result<(), LinuxI2CError> {
        self.calls
            .lock()
            .unwrap()
            .push(Call::WriteByteData(register, value));
        self.regs[usize::from(self.bound)][usize::from(register)] = value;
        Ok(())
    }
    fn read_i2c_block(&mut self, register: u8, len: u8) -> Result<Vec<u8>, LinuxI2CError> {
        self.calls
            .lock()
            .unwrap()
            .push(Call::ReadBlock(register, len));
        let start = usize::from(register);
        let end = (start + usize::from(len)).min(256);
        let mut block = self.regs[usize::from(self.bound)][start..end].to_vec();
        if self.short_blocks {
            block.pop();
        }
        Ok(block)
    }
    fn write_i2c_block(&mut self, register: u8, values: &[u8]) -> Result<(), LinuxI2CError> {
        self.calls
            .lock()
            .unwrap()
            .push(Call::WriteBlock(register, values.to_vec()));
        Ok(())
    }
}

fn chip() -> (Chip, Arc<Mutex<Vec<Call>>>) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut regs = [[0u8; 256]; 128];
    for (i, byte) in regs[usize::from(EDID)].iter_mut().enumerate() {
        *byte = i as u8;
    }
    regs[usize::from(SCDC)][0x01] = 0x01;
    let chip = Chip {
        calls: Arc::clone(&calls),
        regs,
        bound: u16::from(SCDC),
        missing: None,
        short_blocks: false,
    };
    (chip, calls)
}

fn bus(chip: Chip) -> I2cDevBus<SmbusBus<Chip>> {
    I2cDevBus::from_smbus(SmbusBus::new(chip, Some(SCDC)))
}

#[test]
fn each_operation_is_its_smbus_call() {
    let (chip, calls) = chip();
    let bus = bus(chip);
    bus.write(SCDC, &[0x02]).unwrap();
    bus.write(SCDC, &[0x02, 0x01]).unwrap();
    bus.write(SCDC, &[0x30, 1, 2, 3]).unwrap();
    let mut byte = [0u8; 1];
    bus.write_read(SCDC, &[0x01], &mut byte).unwrap();
    assert_eq!(byte, [0x01]);
    let mut block = [0u8; 4];
    bus.write_read(EDID, &[8], &mut block).unwrap();
    assert_eq!(block, [8, 9, 10, 11]);
    bus.read(EDID, &mut byte).unwrap();
    assert_eq!(
        *calls.lock().unwrap(),
        [
            Call::WriteByte(0x02),
            Call::WriteByteData(0x02, 0x01),
            Call::WriteBlock(0x30, vec![1, 2, 3]),
            Call::ReadByteData(0x01),
            Call::Bind(u16::from(EDID)),
            Call::ReadBlock(8, 4),
            Call::ReadByte,
        ]
    );
}

#[test]
fn an_edid_block_is_read_in_32_byte_pieces_at_segment_0() {
    let (chip, calls) = chip();
    let bus = bus(chip);
    let mut block = [0u8; 128];
    bus.read_segmented(EddcTarget::Edid, 0, 128, &mut block)
        .unwrap();
    assert!(block.iter().enumerate().all(|(i, &b)| b == (128 + i) as u8));
    let calls = calls.lock().unwrap();
    assert_eq!(calls[0], Call::Bind(u16::from(EDID)));
    assert_eq!(
        calls[1..],
        [
            Call::ReadBlock(128, 32),
            Call::ReadBlock(160, 32),
            Call::ReadBlock(192, 32),
            Call::ReadBlock(224, 32),
        ]
    );
}

#[test]
fn what_smbus_cannot_do_is_refused_or_unsupported() {
    let (chip, calls) = chip();
    let bus = bus(chip);
    let mut two = [0u8; 2];
    let refusal = |result: Result<(), I2cDevError>| match result {
        Err(I2cDevError::Refused { reason }) => reason,
        other => panic!("{other:?}"),
    };
    assert_eq!(
        refusal(bus.write(SCDC, &[0; 34])),
        Refusal::TooLong { len: 34, max: 33 }
    );
    assert_eq!(
        refusal(bus.read(SCDC, &mut two)),
        Refusal::TooLong { len: 2, max: 1 }
    );
    assert_eq!(
        refusal(bus.write_read(SCDC, &[0, 1], &mut two)),
        Refusal::TooLong { len: 2, max: 1 }
    );
    assert_eq!(
        refusal(bus.read_segmented(EddcTarget::Edid, 1, 0, &mut two)),
        Refusal::Segment { segment: 1 }
    );
    assert!(calls.lock().unwrap().is_empty());

    // A shape the capabilities would have refused, straight at the backend.
    let (fresh, _) = self::chip();
    let mut smbus = SmbusBus::new(fresh, None);
    let mut buf = [0u8; 2];
    let shapes: [&mut [Message<'_>]; 1] = [&mut [Message::Read {
        address: SCDC,
        buf: &mut buf,
    }]];
    for shape in shapes {
        assert!(matches!(
            smbus.transfer(shape),
            Err(LinuxI2CError::Errno(libc::EOPNOTSUPP))
        ));
    }
    let long = [0u8; 34];
    assert!(matches!(
        smbus.transfer(&mut [Message::Write {
            address: SCDC,
            bytes: &long
        }]),
        Err(LinuxI2CError::Errno(libc::EOPNOTSUPP))
    ));
}

#[test]
fn a_missing_chip_is_no_acknowledge_of_the_address() {
    let (mut chip, _) = chip();
    chip.missing = Some(u16::from(EDID));
    let bus = bus(chip);
    let mut byte = [0u8; 1];
    let error = bus.read(EDID, &mut byte).unwrap_err();
    assert_eq!(
        error.kind(),
        DdcErrorKind::NoAcknowledge(NoAcknowledgeSource::Address)
    );
    // The rebind failed, so the next call binds again.
    assert!(bus.read(EDID, &mut byte).is_err());
}

#[test]
fn a_short_block_is_an_io_error_and_leaves_the_buffer() {
    let (mut chip, _) = chip();
    chip.short_blocks = true;
    let bus = bus(chip);
    let mut block = [0xEE; 8];
    let error = bus.write_read(EDID, &[0], &mut block).unwrap_err();
    assert_eq!(error.kind(), DdcErrorKind::Other);
    assert_eq!(block, [0xEE; 8]);
}

#[test]
fn opening_needs_a_device_that_takes_an_address() {
    assert!(matches!(
        I2cDevBus::open_smbus("/dev/i2c-does-not-exist"),
        Err(I2cDevError::DeviceOpenFailed { .. })
    ));
    // A regular file opens but refuses the I2C_SLAVE ioctl.
    let file = tempfile::NamedTempFile::new().unwrap();
    assert!(matches!(
        I2cDevBus::open_smbus(file.path()),
        Err(I2cDevError::DeviceOpenFailed { .. })
    ));
}
