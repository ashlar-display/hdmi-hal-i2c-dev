#![cfg(test)]

use std::error::Error;
use std::io;
use std::path::PathBuf;

use crate::error::{I2cDevError, I2cErrorKind, I2cTransactionError, MessagePhase};

// --- I2cDevError::Display ----------------------------------------------------

#[test]
fn display_connector_not_found() {
    let e = I2cDevError::ConnectorNotFound {
        connector: "card0-HDMI-A-1".to_string(),
    };
    assert_eq!(
        e.to_string(),
        "DRM connector not found in sysfs: card0-HDMI-A-1"
    );
}

#[test]
fn display_connector_has_no_ddc_adapter() {
    let e = I2cDevError::ConnectorHasNoDdcAdapter {
        connector: "card0-HDMI-A-1".to_string(),
    };
    assert_eq!(
        e.to_string(),
        "connector card0-HDMI-A-1 has no DDC adapter (no sink attached, or not an HDMI port)"
    );
}

#[test]
fn display_ddc_adapter_index_unparseable() {
    let e = I2cDevError::DdcAdapterIndexUnparseable {
        symlink_target: PathBuf::from("../../devices/usb-6"),
    };
    assert_eq!(
        e.to_string(),
        "DDC symlink target does not contain a recognizable adapter index: ../../devices/usb-6"
    );
}

#[test]
fn display_device_open_failed() {
    let e = I2cDevError::DeviceOpenFailed {
        path: PathBuf::from("/dev/i2c-99"),
        source: io::Error::from(io::ErrorKind::PermissionDenied),
    };
    assert_eq!(e.to_string(), "failed to open I²C device: /dev/i2c-99");
}

#[test]
fn display_transaction() {
    let e = I2cDevError::Transaction(I2cTransactionError {
        phase: MessagePhase::Write { register: 0x20 },
        kind: I2cErrorKind::AddressNack,
    });
    assert!(e.to_string().contains("I²C transaction failed"), "got: {e}");
}

// --- I2cDevError::Error::source ----------------------------------------------

#[test]
fn source_device_open_failed_is_some() {
    let e = I2cDevError::DeviceOpenFailed {
        path: PathBuf::from("/dev/i2c-99"),
        source: io::Error::from(io::ErrorKind::PermissionDenied),
    };
    assert!(e.source().is_some());
}

#[test]
fn source_transaction_is_some() {
    let e = I2cDevError::Transaction(I2cTransactionError {
        phase: MessagePhase::Write { register: 0x00 },
        kind: I2cErrorKind::BusBusy,
    });
    assert!(e.source().is_some());
}

#[test]
fn source_connector_not_found_is_none() {
    let e = I2cDevError::ConnectorNotFound {
        connector: "x".to_string(),
    };
    assert!(e.source().is_none());
}

#[test]
fn display_block_read_too_long() {
    let e = I2cDevError::BlockReadTooLong {
        register: 0x00,
        len: 256,
        max: 32,
    };
    assert_eq!(
        e.to_string(),
        "block read of 256 bytes from register 0x00 exceeds the 32-byte limit"
    );
}

#[test]
fn display_unexpected_block_length() {
    let e = I2cDevError::UnexpectedBlockLength {
        register: 0x50,
        requested: 9,
        returned: 8,
    };
    assert_eq!(
        e.to_string(),
        "block read from register 0x50 returned 8 bytes, not 9"
    );
}

#[test]
fn source_block_read_errors_are_none() {
    let too_long = I2cDevError::BlockReadTooLong {
        register: 0,
        len: 33,
        max: 32,
    };
    let unexpected = I2cDevError::UnexpectedBlockLength {
        register: 0,
        requested: 2,
        returned: 1,
    };
    assert!(too_long.source().is_none());
    assert!(unexpected.source().is_none());
}

// --- I2cTransactionError::Display -------------------------------------------

#[test]
fn transaction_error_display_write_phase() {
    let e = I2cTransactionError {
        phase: MessagePhase::Write { register: 0x0f },
        kind: I2cErrorKind::AddressNack,
    };
    assert_eq!(e.to_string(), "write phase (register 0x0f): AddressNack");
}

#[test]
fn transaction_error_display_read_phase() {
    let e = I2cTransactionError {
        phase: MessagePhase::Read { register: 0xa0 },
        kind: I2cErrorKind::Unknown { errno: 5 },
    };
    assert_eq!(
        e.to_string(),
        "read phase (register 0xa0): Unknown { errno: 5 }"
    );
}

// --- I2cTransactionError::Error::source -------------------------------------

#[test]
fn transaction_error_source_is_none() {
    let e = I2cTransactionError {
        phase: MessagePhase::Write { register: 0x00 },
        kind: I2cErrorKind::AddressNack,
    };
    assert!(e.source().is_none());
}

// --- DdcError ---------------------------------------------------------------

#[test]
fn errnos_classify_as_ddc_error_kinds() {
    use hdmi_hal::ddc::{DdcError, DdcErrorKind, NoAcknowledgeSource};

    let kind = |errno| {
        I2cDevError::Transaction(I2cTransactionError {
            phase: MessagePhase::Write { register: 0x21 },
            kind: crate::transport::map_errno(errno),
        })
        .kind()
    };
    for (errno, expected) in [
        (
            libc::ENXIO,
            DdcErrorKind::NoAcknowledge(NoAcknowledgeSource::Unknown),
        ),
        (libc::EAGAIN, DdcErrorKind::Busy),
        (libc::EBUSY, DdcErrorKind::Busy),
        (libc::ETIMEDOUT, DdcErrorKind::Timeout),
        (libc::ESHUTDOWN, DdcErrorKind::Detached),
        (libc::ENODEV, DdcErrorKind::Detached),
        (libc::EOPNOTSUPP, DdcErrorKind::Unsupported),
        (libc::EPROTO, DdcErrorKind::Other),
        (libc::EIO, DdcErrorKind::Other),
    ] {
        assert_eq!(kind(errno), expected, "errno {errno}");
    }

    let data_nack = I2cTransactionError {
        phase: MessagePhase::Read { register: 0x21 },
        kind: I2cErrorKind::DataNack,
    };
    assert_eq!(
        data_nack.kind(),
        DdcErrorKind::NoAcknowledge(NoAcknowledgeSource::Data)
    );
    let too_long = I2cDevError::BlockReadTooLong {
        register: 0x40,
        len: 64,
        max: 32,
    };
    assert_eq!(too_long.kind(), DdcErrorKind::Unsupported);
    let short = I2cDevError::UnexpectedBlockLength {
        register: 0xF0,
        requested: 32,
        returned: 16,
    };
    assert_eq!(short.kind(), DdcErrorKind::Other);
    let not_found = I2cDevError::ConnectorNotFound {
        connector: "card0-HDMI-A-1".into(),
    };
    assert_eq!(not_found.kind(), DdcErrorKind::Other);
}
