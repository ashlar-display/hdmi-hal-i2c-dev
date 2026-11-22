//! hdmi-hal-i2c-dev's `read_edid`, piaf's read plan run block by block, over the bench's EDID
//! behind the E-DDC segment pointer.

use hdmi_hal_i2c_dev::{EdidReadError, read_edid};
use maquette::{
    Bench, BusOp, DdcCapabilities, DdcError, DdcErrorKind, Effect, Event, Fault, Filter,
    NoAcknowledgeSource, OpKind, SinkConfig,
};
use piaf::read::{ReadError, ReadOptions, ReadWarning};

/// An EDID of `extensions` extension blocks: a version 1.4 base block, then CTA blocks, each
/// marked with its number at byte 4, every block with a valid checksum.
fn edid(extensions: u8) -> Vec<u8> {
    let mut bytes = vec![0u8; 128 * (usize::from(extensions) + 1)];
    bytes[..8].copy_from_slice(&[0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00]);
    bytes[18] = 1;
    bytes[19] = 4;
    bytes[126] = extensions;
    for (n, block) in bytes.chunks_mut(128).enumerate() {
        if n > 0 {
            block[0] = 0x02;
            block[1] = 3;
            block[4] = n as u8;
        }
        fix_checksum(block);
    }
    bytes
}

fn fix_checksum(block: &mut [u8]) {
    let sum = block[..127]
        .iter()
        .fold(0u8, |sum, &byte| sum.wrapping_add(byte));
    block[127] = 0u8.wrapping_sub(sum);
}

/// Reads the bench's EDID into room for 8 blocks; the blocks read, flattened.
fn read(
    bench: &Bench<'_>,
) -> Result<(Vec<u8>, Vec<ReadWarning>), EdidReadError<maquette::BusError>> {
    let mut blocks = [[0u8; 128]; 8];
    let plan = read_edid(&bench.ddc(), &mut blocks, ReadOptions::new())?;
    let read = blocks[..usize::from(plan.blocks_read())]
        .as_flattened()
        .to_vec();
    Ok((read, plan.warnings().collect()))
}

/// The segments `read_segmented` was asked for, in order.
fn segments(bench: &Bench<'_>) -> Vec<u8> {
    bench
        .transcript()
        .filter_map(|record| match record.event {
            Event::Bus(bus) => match bus.op {
                BusOp::ReadSegmented { segment, .. } => Some(segment),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

#[test]
fn a_one_block_edid_reads_whole() {
    let store = edid(0);
    let bench = Bench::new(SinkConfig::new(), &store).unwrap();
    let (read, warnings) = read(&bench).unwrap();
    assert_eq!(read, store);
    assert!(warnings.is_empty());
}

#[test]
fn blocks_past_segment_0_are_read_through_the_pointer() {
    let store = edid(3);
    let bench = Bench::new(SinkConfig::new(), &store).unwrap();
    let (read, warnings) = read(&bench).unwrap();
    assert_eq!(read, store);
    assert!(warnings.is_empty());
    assert_eq!(segments(&bench), [0, 0, 1, 1]);
}

#[test]
fn a_bus_that_splits_reads_still_reads_every_block() {
    let store = edid(3);
    let caps = DdcCapabilities::general().with_segmented_piece(Some(32));
    let bench = Bench::builder(&store)
        .ddc_capabilities(caps)
        .build()
        .unwrap();
    let (read, _) = read(&bench).unwrap();
    assert_eq!(read, store);
    assert_eq!(segments(&bench).len(), 4 * 4, "four pieces a block");
}

#[test]
fn a_bus_that_reaches_segment_0_only_reads_two_blocks_and_says_so() {
    let store = edid(3);
    let caps = DdcCapabilities::general().with_max_segment(Some(0));
    let bench = Bench::builder(&store)
        .ddc_capabilities(caps)
        .build()
        .unwrap();
    let (read, warnings) = read(&bench).unwrap();
    assert_eq!(read, store[..256]);
    assert!(
        warnings
            .iter()
            .any(|warning| matches!(warning, ReadWarning::BlocksNotRead { .. })),
        "{warnings:?}"
    );
}

#[test]
fn a_sink_without_a_pointer_serves_segment_0_only() {
    let config = SinkConfig::new().without_segment_pointer();
    let store = edid(1);
    let bench = Bench::new(config, &store).unwrap();
    assert_eq!(read(&bench).unwrap().0, store);

    let store = edid(3);
    let bench = Bench::new(config, &store).unwrap();
    let Err(EdidReadError::Bus { block, error, .. }) = read(&bench) else {
        panic!("block 2 is out of reach");
    };
    assert_eq!(block, 2);
    assert_eq!(
        error.kind(),
        DdcErrorKind::NoAcknowledge(NoAcknowledgeSource::Address)
    );
}

#[test]
fn a_failed_block_read_names_the_block() {
    let store = edid(1);
    let bench = Bench::new(SinkConfig::new(), &store).unwrap();
    let nack = DdcErrorKind::NoAcknowledge(NoAcknowledgeSource::Data);
    bench
        .schedule(
            Fault::new(Effect::Fail(nack))
                .on(Filter::ANY.with_op(OpKind::ReadSegmented).with_first(128)),
        )
        .unwrap();
    let Err(EdidReadError::Bus { block, error, .. }) = read(&bench) else {
        panic!("block 1 failed");
    };
    assert_eq!((block, error.kind()), (1, nack));
}

#[test]
fn a_bad_base_block_is_refused_and_a_bad_extension_is_reported() {
    let mut store = edid(1);
    store[127] ^= 0xFF;
    let bench = Bench::new(SinkConfig::new(), &store).unwrap();
    assert!(matches!(
        read(&bench),
        Err(EdidReadError::Refused(ReadError::BaseBlock(_)))
    ));

    let mut store = edid(1);
    store[255] ^= 0xFF;
    let bench = Bench::new(SinkConfig::new(), &store).unwrap();
    let (read, warnings) = read(&bench).unwrap();
    assert_eq!(read, store);
    assert_eq!(warnings, [ReadWarning::ChecksumMismatch { block: 1 }]);
}

#[test]
fn a_sink_without_an_edid_does_not_answer() {
    let bench = Bench::new(SinkConfig::new(), &[]).unwrap();
    let Err(EdidReadError::Bus { block, error, .. }) = read(&bench) else {
        panic!("nothing answers 0x50");
    };
    assert_eq!(block, 0);
    assert_eq!(
        error.kind(),
        DdcErrorKind::NoAcknowledge(NoAcknowledgeSource::Address)
    );
}
