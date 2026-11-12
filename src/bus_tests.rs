use std::sync::{Arc, Mutex as StdMutex};

use hdmi_hal::ddc::{DISPLAYID, DdcError, DdcErrorKind, EDID, NoAcknowledgeSource, SCDC};

use super::*;
use crate::error::I2cErrorKind;

/// The transfers a [`Mock`] saw, shared with the test.
type Log = Arc<StdMutex<Vec<Vec<Seen>>>>;

/// One recorded message: written bytes, or the length of a read, with its address.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Seen {
    W(u8, Vec<u8>),
    R(u8, usize),
}

/// A bus that records each transfer's messages, answers reads with `fill`, and can fail
/// with an errno or execute fewer messages than sent.
#[derive(Default)]
struct Mock {
    transfers: Log,
    fill: u8,
    errno: Option<i32>,
    done: Option<u32>,
}

impl I2cBus for Mock {
    fn transfer(&mut self, messages: &mut [Message<'_>]) -> Result<u32, LinuxI2CError> {
        let seen = messages
            .iter_mut()
            .map(|m| match m {
                Message::Write { address, bytes } => Seen::W(*address, bytes.to_vec()),
                Message::Read { address, buf } => {
                    buf.fill(self.fill);
                    Seen::R(*address, buf.len())
                }
            })
            .collect();
        self.transfers.lock().unwrap().push(seen);
        if let Some(errno) = self.errno {
            return Err(LinuxI2CError::Errno(errno));
        }
        Ok(self.done.unwrap_or(u32::try_from(messages.len()).unwrap()))
    }
}

fn bus(capabilities: DdcCapabilities) -> (I2cDevBus<Mock>, Log) {
    let mock = Mock {
        fill: 0xA5,
        ..Mock::default()
    };
    let transfers = Arc::clone(&mock.transfers);
    (I2cDevBus::from_bus(mock, capabilities), transfers)
}

fn refusal(result: Result<(), I2cDevError>) -> Refusal {
    match result {
        Err(I2cDevError::Refused { reason }) => reason,
        other => panic!("not refused: {other:?}"),
    }
}

#[test]
fn each_operation_is_one_transfer_with_its_addresses() {
    let (bus, seen) = bus(DdcCapabilities::general());
    bus.write(SCDC, &[0x20, 0x03]).unwrap();
    let mut buf = [0u8; 2];
    bus.write_read(SCDC, &[0x40], &mut buf).unwrap();
    assert_eq!(buf, [0xA5; 2]);
    bus.read(0x37, &mut buf).unwrap();
    assert_eq!(
        *seen.lock().unwrap(),
        [
            vec![Seen::W(SCDC, vec![0x20, 0x03])],
            vec![Seen::W(SCDC, vec![0x40]), Seen::R(SCDC, 2)],
            vec![Seen::R(0x37, 2)],
        ]
    );
}

#[test]
fn an_eddc_read_writes_the_pointer_only_past_segment_0() {
    let (bus, seen) = bus(DdcCapabilities::general());
    let mut block = [0u8; 128];
    bus.read_segmented(EddcTarget::Edid, 0, 128, &mut block)
        .unwrap();
    bus.read_segmented(EddcTarget::DisplayId, 1, 0, &mut block)
        .unwrap();
    assert_eq!(
        *seen.lock().unwrap(),
        [
            vec![Seen::W(EDID, vec![128]), Seen::R(EDID, 128)],
            vec![
                Seen::W(SEGMENT_POINTER, vec![1]),
                Seen::W(DISPLAYID, vec![0]),
                Seen::R(DISPLAYID, 128),
            ],
        ]
    );
}

#[test]
fn a_split_eddc_read_is_one_transfer_per_piece() {
    let (bus, seen) = bus(DdcCapabilities::general().with_segmented_piece(Some(48)));
    let mut block = [0u8; 128];
    bus.read_segmented(EddcTarget::Edid, 2, 128, &mut block)
        .unwrap();
    let transfers = seen.lock().unwrap();
    assert_eq!(transfers.len(), 3);
    assert_eq!(
        transfers[2],
        [
            Seen::W(SEGMENT_POINTER, vec![2]),
            Seen::W(EDID, vec![128 + 96]),
            Seen::R(EDID, 32),
        ]
    );
}

#[test]
fn empty_buffers_do_nothing_and_an_empty_write_is_refused() {
    let (bus, seen) = bus(DdcCapabilities::general());
    bus.read(EDID, &mut []).unwrap();
    bus.write_read(EDID, &[0], &mut []).unwrap();
    bus.read_segmented(EddcTarget::Edid, 0, 0, &mut []).unwrap();
    assert!(seen.lock().unwrap().is_empty());
    assert_eq!(refusal(bus.write(EDID, &[])), Refusal::EmptyWrite);
    // A write_read with no bytes is a read.
    let mut buf = [0u8; 1];
    bus.write_read(EDID, &[], &mut buf).unwrap();
    assert_eq!(seen.lock().unwrap()[0], [Seen::R(EDID, 1)]);
}

#[test]
fn what_the_capabilities_rule_out_is_refused_before_any_transfer() {
    let caps = DdcCapabilities::none()
        .with_write_max(2)
        .with_write_read_max(1, 16)
        .with_read_max(0)
        .with_address_reachable(SCDC, true)
        .with_eddc_target(EddcTarget::Edid, true)
        .with_max_segment(Some(0));
    let (bus, seen) = bus(caps);
    let mut buf = [0u8; 32];
    assert_eq!(
        refusal(bus.write(0x37, &[1])),
        Refusal::Unreachable { address: 0x37 }
    );
    assert_eq!(
        refusal(bus.write(SCDC, &[1, 2, 3])),
        Refusal::TooLong { len: 3, max: 2 }
    );
    assert_eq!(
        refusal(bus.write_read(SCDC, &[1], &mut buf)),
        Refusal::TooLong { len: 32, max: 16 }
    );
    assert_eq!(
        refusal(bus.read(SCDC, &mut buf[..1])),
        Refusal::TooLong { len: 1, max: 0 }
    );
    assert_eq!(
        refusal(bus.read_segmented(EddcTarget::DisplayId, 0, 0, &mut buf)),
        Refusal::Target { address: DISPLAYID }
    );
    assert_eq!(
        refusal(bus.read_segmented(EddcTarget::Edid, 1, 0, &mut buf)),
        Refusal::Segment { segment: 1 }
    );
    assert_eq!(
        refusal(bus.read_segmented(EddcTarget::Edid, 0, 240, &mut buf)),
        Refusal::PastSegmentEnd {
            offset: 240,
            len: 32
        }
    );
    assert!(seen.lock().unwrap().is_empty());
    // Refusals are Unsupported to a caller.
    assert_eq!(
        bus.write(SCDC, &[1, 2, 3]).unwrap_err().kind(),
        DdcErrorKind::Unsupported
    );
}

#[test]
fn eddc_reaches_no_segment_beyond_its_own_and_messages_stay_under_the_kernels_limit() {
    let (bus, seen) = bus(DdcCapabilities::general());
    let mut block = [0u8; 1];
    assert_eq!(
        refusal(bus.read_segmented(EddcTarget::Edid, MAX_SEGMENT + 1, 0, &mut block)),
        Refusal::Segment {
            segment: MAX_SEGMENT + 1
        }
    );
    let big = vec![0u8; MAX_MESSAGE_LEN + 1];
    assert_eq!(
        refusal(bus.write(SCDC, &big)),
        Refusal::TooLong {
            len: MAX_MESSAGE_LEN + 1,
            max: MAX_MESSAGE_LEN
        }
    );
    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn failures_keep_the_address_and_the_errno_class() {
    let (mut bus, _) = bus(DdcCapabilities::general());
    bus.bus.get_mut().unwrap().errno = Some(libc::ENXIO);
    let error = bus.write(SCDC, &[0x20, 0]).unwrap_err();
    assert!(matches!(
        error,
        I2cDevError::Transfer {
            address: SCDC,
            kind: I2cErrorKind::AddressNack
        }
    ));
    assert_eq!(
        error.kind(),
        DdcErrorKind::NoAcknowledge(NoAcknowledgeSource::Unknown)
    );
    bus.bus.get_mut().unwrap().errno = Some(libc::ENODEV);
    let mut block = [0u8; 8];
    let error = bus
        .read_segmented(EddcTarget::Edid, 1, 0, &mut block)
        .unwrap_err();
    // The first message's address: the segment pointer.
    assert!(matches!(
        error,
        I2cDevError::Transfer {
            address: SEGMENT_POINTER,
            ..
        }
    ));
    assert_eq!(error.kind(), DdcErrorKind::Detached);
}

#[test]
fn a_transfer_the_kernel_cut_short_is_an_error() {
    let (mut bus, _) = bus(DdcCapabilities::general());
    bus.bus.get_mut().unwrap().done = Some(1);
    let mut block = [0u8; 8];
    let error = bus
        .read_segmented(EddcTarget::Edid, 1, 0, &mut block)
        .unwrap_err();
    assert!(matches!(
        error,
        I2cDevError::IncompleteTransfer { sent: 3, done: 1 }
    ));
    assert_eq!(error.kind(), DdcErrorKind::Other);
}

#[test]
fn a_poisoned_lock_is_recovered() {
    let (bus, seen) = bus(DdcCapabilities::general());
    let bus = Arc::new(bus);
    let poisoner = Arc::clone(&bus);
    let _ = std::thread::spawn(move || {
        let _guard = poisoner.bus.lock().unwrap();
        panic!("poison the lock");
    })
    .join();
    assert!(bus.bus.is_poisoned());
    bus.write(SCDC, &[0x20, 0]).unwrap();
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn the_capabilities_are_the_callers() {
    let caps = DdcCapabilities::none().with_eddc_target(EddcTarget::Edid, true);
    let (bus, _) = bus(caps);
    assert_eq!(bus.capabilities(), caps);
}

#[test]
fn opening_a_path_that_is_not_there_fails() {
    let error =
        I2cDevBus::open_with("/dev/i2c-does-not-exist", DdcCapabilities::general()).unwrap_err();
    assert!(matches!(error, I2cDevError::DeviceOpenFailed { .. }));
}

#[test]
fn a_file_that_is_no_i2c_device_fails_its_transfer_in_the_ioctl() {
    // Opening checks nothing but the file; the ioctl then fails with ENOTTY, through the
    // real `LinuxI2CBus` path, both message directions included.
    let file = tempfile::NamedTempFile::new().unwrap();
    let bus = I2cDevBus::open_with(file.path(), DdcCapabilities::general()).unwrap();
    let mut buf = [0u8; 2];
    let error = bus.write_read(SCDC, &[0x40], &mut buf).unwrap_err();
    assert!(matches!(
        error,
        I2cDevError::Transfer {
            address: SCDC,
            kind: I2cErrorKind::Unknown {
                errno: libc::ENOTTY
            }
        }
    ));
    assert_eq!(error.kind(), DdcErrorKind::Other);
}
