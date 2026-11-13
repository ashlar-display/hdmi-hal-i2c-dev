//! Integration tests for [`I2cDevBus::open_smbus`] against the Linux `i2c-stub` kernel module.
//!
//! The same setup as `tests/transport.rs`: `i2c-stub` loaded with a chip at the SCDC address
//! (`sudo modprobe i2c-stub chip_addr=0x54`), its adapter number in `I2C_STUB_ADAPTER`, and for
//! the second pass the stub reloaded at 0x55 with its adapter in `I2C_STUB_ADAPTER_NACK`. A
//! test whose variable is unset is skipped, unless `I2C_STUB_REQUIRED=1`.
//!
//! ```sh
//! I2C_STUB_ADAPTER=N cargo test --locked --features integration --test smbus_bus -- --skip no_chip
//! I2C_STUB_ADAPTER_NACK=M cargo test --locked --features integration --test smbus_bus no_chip
//! ```
//!
//! **Covered:** the SMBus calls `I2cDevBus` makes in SMBus mode (byte and byte-data writes,
//! an I²C block write, byte-data and I²C block reads), binding another address and back, the
//! adapter's name read from sysfs, and `ENODEV` for an address without a chip, which is
//! `NoAcknowledge(Address)`.

#![cfg(feature = "integration")]

use std::path::PathBuf;

use hdmi_hal::ddc::{DdcBus, DdcError, DdcErrorKind, EDID, NoAcknowledgeSource, SCDC};
use hdmi_hal_i2c_dev::I2cDevBus;

fn adapter_path(var: &str) -> Option<PathBuf> {
    let Ok(n) = std::env::var(var) else {
        if std::env::var("I2C_STUB_REQUIRED").is_ok_and(|v| v == "1") {
            panic!("{var} not set, and I2C_STUB_REQUIRED=1: this test must run");
        }
        eprintln!("skipped: {var} not set");
        return None;
    };
    Some(PathBuf::from(format!("/dev/i2c-{n}")))
}

fn open(var: &str) -> Option<I2cDevBus> {
    let path = adapter_path(var)?;
    Some(
        I2cDevBus::open_smbus(&path)
            .unwrap_or_else(|e| panic!("failed to open {}: {e}", path.display())),
    )
}

#[test]
fn writes_and_reads_round_trip_through_smbus_calls() {
    let Some(bus) = open("I2C_STUB_ADAPTER") else {
        return;
    };
    assert_eq!(bus.adapter(), Some("SMBus stub driver"));
    // A byte-data write, read back with a byte-data read.
    bus.write(SCDC, &[0x02, 0x01]).unwrap();
    let mut byte = [0u8; 1];
    bus.write_read(SCDC, &[0x02], &mut byte).unwrap();
    assert_eq!(byte, [0x01]);
    // An I²C block write, read back with an I²C block read.
    bus.write(SCDC, &[0x40, 0xA1, 0xA2, 0xA3]).unwrap();
    let mut block = [0u8; 3];
    bus.write_read(SCDC, &[0x40], &mut block).unwrap();
    assert_eq!(block, [0xA1, 0xA2, 0xA3]);
    // A byte write sets the stub's pointer; a byte read reads at it.
    bus.write(SCDC, &[0x40]).unwrap();
    bus.read(SCDC, &mut byte).unwrap();
    assert_eq!(byte, [0xA1]);
}

#[test]
fn an_address_without_a_chip_is_no_acknowledge_and_the_bus_binds_back() {
    let Some(bus) = open("I2C_STUB_ADAPTER") else {
        return;
    };
    let mut block = [0u8; 32];
    let error = bus.write_read(EDID, &[0], &mut block).unwrap_err();
    assert_eq!(
        error.kind(),
        DdcErrorKind::NoAcknowledge(NoAcknowledgeSource::Address)
    );
    bus.write(SCDC, &[0x02, 0x01]).unwrap();
}

#[test]
fn no_chip_at_the_scdc_address() {
    let Some(bus) = open("I2C_STUB_ADAPTER_NACK") else {
        return;
    };
    let mut byte = [0u8; 1];
    let error = bus.write_read(SCDC, &[0x01], &mut byte).unwrap_err();
    assert_eq!(
        error.kind(),
        DdcErrorKind::NoAcknowledge(NoAcknowledgeSource::Address)
    );
}
