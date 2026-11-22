#![cfg(test)]

use std::error::Error;
use std::io;
use std::path::PathBuf;

use crate::error::{I2cDevError, I2cErrorKind, map_errno, to_raw_errno};

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
fn source_connector_not_found_is_none() {
    let e = I2cDevError::ConnectorNotFound {
        connector: "x".to_string(),
    };
    assert!(e.source().is_none());
}

// --- DdcError ---------------------------------------------------------------

#[test]
fn errnos_classify_as_ddc_error_kinds() {
    use hdmi_hal::ddc::{DdcError, DdcErrorKind, NoAcknowledgeSource};

    let kind = |errno| {
        I2cDevError::Transfer {
            address: 0x54,
            kind: map_errno(errno),
        }
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

    let classify = |kind| {
        I2cDevError::Transfer {
            address: 0x54,
            kind,
        }
        .kind()
    };
    assert_eq!(
        classify(I2cErrorKind::DataNack),
        DdcErrorKind::NoAcknowledge(NoAcknowledgeSource::Data)
    );
    assert_eq!(
        classify(I2cErrorKind::NoDevice),
        DdcErrorKind::NoAcknowledge(NoAcknowledgeSource::Address)
    );
    let refused = I2cDevError::Refused {
        reason: crate::error::Refusal::EmptyWrite,
    };
    assert_eq!(refused.kind(), DdcErrorKind::Unsupported);
    let incomplete = I2cDevError::IncompleteTransfer { sent: 2, done: 1 };
    assert_eq!(incomplete.kind(), DdcErrorKind::Other);
    let not_found = I2cDevError::ConnectorNotFound {
        connector: "card0-HDMI-A-1".into(),
    };
    assert_eq!(not_found.kind(), DdcErrorKind::Other);
}

#[test]
fn bus_errors_and_refusals_print() {
    use crate::error::Refusal;

    let transfer = I2cDevError::Transfer {
        address: 0x54,
        kind: I2cErrorKind::AddressNack,
    };
    assert_eq!(
        transfer.to_string(),
        "I²C transfer to 0x54 failed: AddressNack"
    );
    let incomplete = I2cDevError::IncompleteTransfer { sent: 3, done: 1 };
    assert_eq!(
        incomplete.to_string(),
        "I²C transfer incomplete: 1 of 3 messages executed"
    );
    for (reason, text) in [
        (Refusal::EmptyWrite, "a write of no bytes"),
        (
            Refusal::TooLong { len: 9, max: 8 },
            "9 bytes, more than the 8 allowed",
        ),
        (
            Refusal::Unreachable { address: 0x37 },
            "address 0x37 is not reachable",
        ),
        (
            Refusal::Target { address: 0x52 },
            "the E-DDC target at 0x52 is not reachable",
        ),
        (
            Refusal::Segment { segment: 2 },
            "E-DDC segment 2 is not reachable",
        ),
        (
            Refusal::PastSegmentEnd {
                offset: 240,
                len: 32,
            },
            "32 bytes from offset 240 run past the segment",
        ),
    ] {
        assert_eq!(
            I2cDevError::Refused { reason }.to_string(),
            format!("refused: {text}")
        );
    }
    assert!(transfer.source().is_none());
}

// --- map_errno, to_raw_errno ---------------------------------------------------

#[test]
fn errnos_map_to_the_fault_codes_classes() {
    for (errno, expected) in [
        (libc::ENXIO, "AddressNack"),
        (libc::EAGAIN, "ArbitrationLost"),
        (libc::ETIMEDOUT, "ClockStretchTimeout"),
        (libc::EBUSY, "BusBusy"),
        (libc::EPROTO, "BusProtocolError"),
        (libc::ESHUTDOWN, "AdapterGone"),
        (libc::ENODEV, "AdapterGone"),
        (libc::EOPNOTSUPP, "Unsupported"),
    ] {
        assert_eq!(format!("{:?}", map_errno(errno)), expected, "errno {errno}");
    }
    // amdgpu's only errno has no class; nor has an unknown one, or none.
    for errno in [libc::EIO, 9999, 0] {
        assert!(matches!(map_errno(errno), I2cErrorKind::Unknown { errno: e } if e == errno));
    }
}

#[test]
fn an_io_error_without_an_errno_is_errno_0() {
    use i2cdev::linux::LinuxI2CError;

    assert_eq!(to_raw_errno(LinuxI2CError::Errno(libc::EIO)), libc::EIO);
    let os = io::Error::from_raw_os_error(libc::ENXIO);
    assert_eq!(to_raw_errno(LinuxI2CError::Io(os)), libc::ENXIO);
    let other = io::Error::other("no errno");
    assert_eq!(to_raw_errno(LinuxI2CError::Io(other)), 0);
}
