//! Integration tests for SCDC over an [`I2cDevBus`], hdmi-hal's [`ScdcOverDdc`], against the
//! Linux `i2c-stub` kernel module.
//!
//! These tests exercise the SMBus calls of a bus opened with [`I2cDevBus::open_smbus`]
//! against a real kernel I²C adapter, through the SCDC transport the stack uses. They
//! validate register read/write semantics and error handling — things that cannot be verified
//! without a responding kernel device.
//!
//! # Setup
//!
//! **Load i2c-stub with a device at the SCDC slave address (0x54):**
//!
//! ```sh
//! sudo modprobe i2c-stub chip_addr=0x54
//! ```
//!
//! **Find the adapter number** (the `N` in `/dev/i2c-N`):
//!
//! ```sh
//! ls -lt /dev/i2c-* | head -3
//! # or: ls /sys/bus/i2c/drivers/i2c-stub/
//! ```
//!
//! **Grant permission** to the device node (or add yourself to the `i2c` group):
//!
//! ```sh
//! sudo chmod 0666 /dev/i2c-N
//! ```
//!
//! **Run the tests:**
//!
//! ```sh
//! I2C_STUB_ADAPTER=N cargo test --locked --features integration -- --nocapture
//! ```
//!
//! A test whose adapter variable is unset is skipped, not failed, and prints
//! `skipped: <variable> not set`; `--nocapture` shows it (a skipped test still reports
//! `ok`). With `I2C_STUB_REQUIRED=1` a skip fails instead: set it in CI and for the release
//! run, so that a forgotten adapter variable cannot pass for tier 2 coverage, and run each
//! pass with only its own tests (`--skip no_chip` in the first, `no_chip` in the second):
//!
//! ```sh
//! I2C_STUB_REQUIRED=1 I2C_STUB_ADAPTER=N cargo test --locked --features integration -- --skip no_chip
//! ```
//!
//! # NACK test setup
//!
//! The NACK tests need an adapter where nothing answers at 0x54. `i2c-stub` registers a
//! single adapter whatever `chip_addr` lists, and a second `modprobe` of the loaded
//! module changes nothing, so run them in a second pass with the stub reloaded at
//! another address (its adapter number, `M`, may differ from `N`):
//!
//! ```sh
//! sudo modprobe -r i2c-stub
//! sudo modprobe i2c-stub chip_addr=0x55
//! sudo chmod 0666 /dev/i2c-M
//! I2C_STUB_ADAPTER_NACK=M cargo test --locked --features integration no_chip -- --nocapture
//! ```
//!
//! (with `I2C_STUB_REQUIRED=1` too, in CI and for the release run).
//!
//! # What is and is not covered
//!
//! **Covered:**
//! - A register read: a byte-data read, the offset written and one byte read in one ioctl
//! - A register write: a byte-data write of offset and value
//! - `read_block`: consecutive registers in one I²C block read, for 9 bytes (the CED
//!   block), 2 bytes and the 32-byte maximum; `i2c-stub` supports it by default
//!   (`I2C_FUNC_SMBUS_I2C_BLOCK`)
//! - A block read longer than 32 bytes refused before any ioctl, by the bus's capabilities
//! - A block read past 0xFF: `i2c-stub` ends it at 0xFF, so a 32-byte read from 0xF0 is
//!   `EIO` and the buffer is left as it was
//! - A transport made with `ScdcOverDdc::bytewise` reading a block a register at a time
//! - Read/write round-trip correctness against the stub register map
//! - `NoAcknowledge(Address)` when no chip is at 0x54, for a register read and for a block
//!   read (`i2c-stub` returns `ENODEV` for it)
//!
//! **Not covered:**
//! - Bus errors, clock stretch timeouts, and arbitration loss — `i2c-stub`
//!   always ACKs registered addresses; these conditions require misbehaving
//!   hardware. Their errno→`I2cErrorKind` mapping is validated by the Tier 1
//!   unit tests in `src/error_tests.rs`.

#![cfg(feature = "integration")]

use std::path::PathBuf;

use hdmi_hal::ddc::{DdcError, DdcErrorKind, NoAcknowledgeSource};
use hdmi_hal::scdc::{ScdcOverDdc, ScdcTransport};
use hdmi_hal_i2c_dev::{I2cDevBus, I2cDevError, I2cErrorKind, Refusal};

/// Return the adapter path for `var`, or `None` if the env var is unset.
///
/// Tests that call this and get `None` return early — they are skipped rather
/// than failed so that the suite can be run with partial hardware available. A skip
/// prints a notice (visible with `--nocapture`). With `I2C_STUB_REQUIRED=1`, an unset
/// variable fails the test instead, so a run that should have tier 2 coverage cannot pass
/// without it.
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

fn open_bus(var: &str) -> Option<I2cDevBus> {
    let path = adapter_path(var)?;
    Some(
        I2cDevBus::open_smbus(&path)
            .unwrap_or_else(|e| panic!("failed to open {}: {e}", path.display())),
    )
}

/// SCDC over `bus`, `read_block` in one transfer.
fn scdc(bus: &I2cDevBus) -> ScdcOverDdc<&I2cDevBus> {
    ScdcOverDdc::new(bus).expect("an SMBus bus serves SCDC")
}

// --- Read/write correctness --------------------------------------------------

#[test]
fn read_unwritten_register_returns_zero() {
    // i2c-stub initialises all registers to 0x00. Reading register 0x01
    // (an SCDC register unlikely to be touched by other tests) should return 0.
    let Some(bus) = open_bus("I2C_STUB_ADAPTER") else {
        return;
    };
    let val = scdc(&bus).read(0x01).expect("SCDC read failed");
    assert_eq!(val, 0x00, "stub registers should initialise to 0x00");
}

#[test]
fn write_returns_ok() {
    let Some(bus) = open_bus("I2C_STUB_ADAPTER") else {
        return;
    };
    scdc(&bus).write(0x20, 0x01).expect("SCDC write failed");
}

#[test]
fn read_write_round_trip() {
    // Write a distinctive value to a register and read it back.
    let Some(bus) = open_bus("I2C_STUB_ADAPTER") else {
        return;
    };
    let mut transport = scdc(&bus);
    transport.write(0x30, 0xAB).expect("write failed");
    let val = transport.read(0x30).expect("read failed");
    assert_eq!(val, 0xAB, "read-back should match written value");
}

#[test]
fn multiple_registers_are_independent() {
    let Some(bus) = open_bus("I2C_STUB_ADAPTER") else {
        return;
    };
    let mut transport = scdc(&bus);
    transport.write(0x40, 0x11).expect("write reg 0x40 failed");
    transport.write(0x41, 0x22).expect("write reg 0x41 failed");
    assert_eq!(transport.read(0x40).expect("read reg 0x40 failed"), 0x11);
    assert_eq!(transport.read(0x41).expect("read reg 0x41 failed"), 0x22);
}

// --- Block reads (`read_block`) ---------------------------------------------
//
// Each test uses registers no other test writes, as the tests share one stub.

#[test]
fn block_read_returns_consecutive_registers() {
    // The CED block's nine registers, 0x50–0x58, in one ioctl.
    let Some(bus) = open_bus("I2C_STUB_ADAPTER") else {
        return;
    };
    let mut transport = scdc(&bus);
    let values = [0x34, 0x81, 0xFF, 0x00, 0xFF, 0xFF, 0x4E, 0x01, 0x80];
    for (reg, value) in (0x50..).zip(values) {
        transport.write(reg, value).expect("write failed");
    }
    let mut buf = [0u8; 9];
    transport
        .read_block(0x50, &mut buf)
        .expect("block read failed");
    assert_eq!(buf, values);
}

#[test]
fn block_read_of_two_registers() {
    let Some(bus) = open_bus("I2C_STUB_ADAPTER") else {
        return;
    };
    let mut transport = scdc(&bus);
    transport.write(0x60, 0x65).expect("write failed");
    transport.write(0x61, 0xF1).expect("write failed");
    let mut buf = [0u8; 2];
    transport
        .read_block(0x60, &mut buf)
        .expect("block read failed");
    assert_eq!(buf, [0x65, 0xF1]);
}

#[test]
fn block_read_of_the_maximum_length() {
    let Some(bus) = open_bus("I2C_STUB_ADAPTER") else {
        return;
    };
    let mut transport = scdc(&bus);
    for reg in 0xA0..0xC0u8 {
        transport.write(reg, reg ^ 0x5A).expect("write failed");
    }
    let mut buf = [0u8; 32];
    transport
        .read_block(0xA0, &mut buf)
        .expect("block read failed");
    let expected: Vec<u8> = (0xA0..0xC0u8).map(|reg| reg ^ 0x5A).collect();
    assert_eq!(buf.as_slice(), expected.as_slice());
}

#[test]
fn block_read_longer_than_the_maximum_is_refused() {
    let Some(bus) = open_bus("I2C_STUB_ADAPTER") else {
        return;
    };
    let mut buf = [0u8; 33];
    let err = scdc(&bus)
        .read_block(0x00, &mut buf)
        .expect_err("a 33-byte block read should be refused");
    assert!(
        matches!(
            err,
            I2cDevError::Refused {
                reason: Refusal::TooLong {
                    len: 33,
                    max: 32,
                    ..
                },
                ..
            }
        ),
        "got: {err:?}"
    );
}

#[test]
fn block_read_past_0xff_ends_at_0xff_on_i2c_stub() {
    // `i2c-stub` returns at most `256 - command` bytes for an I2C block read, so a 32-byte
    // read from 0xF0 returns 16: the one end-to-end way to a block of another length.
    let Some(bus) = open_bus("I2C_STUB_ADAPTER") else {
        return;
    };
    let mut buf = [0xAAu8; 32];
    let err = scdc(&bus)
        .read_block(0xF0, &mut buf)
        .expect_err("a block read past 0xFF should come back short on i2c-stub");
    assert!(
        matches!(
            err,
            I2cDevError::Transfer {
                address: 0x54,
                kind: I2cErrorKind::Unknown { errno: libc::EIO },
                ..
            }
        ),
        "got: {err:?}"
    );
    assert_eq!(buf, [0xAAu8; 32], "the buffer should be left unchanged");
}

#[test]
fn bytewise_block_read_returns_consecutive_registers() {
    // `bytewise`: one byte-data read per register, for adapters without the block read.
    let Some(bus) = open_bus("I2C_STUB_ADAPTER") else {
        return;
    };
    let mut transport = ScdcOverDdc::bytewise(&bus).expect("an SMBus bus serves SCDC");
    let values = [0x12, 0x34, 0x56, 0x78];
    for (reg, value) in (0xD0..).zip(values) {
        transport.write(reg, value).expect("write failed");
    }
    let mut buf = [0u8; 4];
    transport
        .read_block(0xD0, &mut buf)
        .expect("bytewise block read failed");
    assert_eq!(buf, values);
}

// --- NACK (no chip at the address) -------------------------------------------

#[test]
fn read_fails_with_no_chip() {
    // Uses a stub bus where 0x54 is not registered, so any transfer to the SCDC address
    // fails: ENXIO (a NACK) on compliant adapters, EIO on amdgpu, and ENODEV on i2c-stub,
    // which reports a missing chip that way; a bus opened for SMBus takes it as such.
    let Some(bus) = open_bus("I2C_STUB_ADAPTER_NACK") else {
        return;
    };
    let err = scdc(&bus)
        .read(0x00)
        .expect_err("expected error for unregistered address, got Ok");
    assert_eq!(
        err.kind(),
        DdcErrorKind::NoAcknowledge(NoAcknowledgeSource::Address),
        "got: {err:?}"
    );
}

#[test]
fn block_read_fails_with_no_chip() {
    let Some(bus) = open_bus("I2C_STUB_ADAPTER_NACK") else {
        return;
    };
    let mut buf = [0u8; 9];
    let err = scdc(&bus)
        .read_block(0x50, &mut buf)
        .expect_err("expected error for unregistered address, got Ok");
    assert_eq!(
        err.kind(),
        DdcErrorKind::NoAcknowledge(NoAcknowledgeSource::Address),
        "got: {err:?}"
    );
}
