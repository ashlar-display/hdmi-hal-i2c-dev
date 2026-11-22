use std::cell::RefCell;

use hdmi_hal::ddc::{DdcCapabilities, DdcErrorKind, EddcTarget, MAX_SEGMENT};
use piaf::read::{ReadError, ReadWarning};

use super::*;

/// A sink's EDID store, reached only through `read_segmented`: it refuses what its
/// capabilities rule out, as the contract asks, can fail from a block on, and records
/// each call.
struct EdidStore {
    blocks: Vec<[u8; BLOCK_LEN]>,
    max_segment: Option<u8>,
    fail_from_block: Option<(u8, DdcErrorKind)>,
    calls: RefCell<Vec<(u8, u8, usize)>>,
}

impl EdidStore {
    fn new(blocks: Vec<[u8; BLOCK_LEN]>) -> Self {
        Self {
            blocks,
            max_segment: Some(MAX_SEGMENT),
            fail_from_block: None,
            calls: RefCell::new(Vec::new()),
        }
    }
}

impl DdcBus for EdidStore {
    type Error = DdcErrorKind;

    fn capabilities(&self) -> DdcCapabilities {
        DdcCapabilities::none()
            .with_eddc_target(EddcTarget::Edid, self.max_segment.is_some())
            .with_max_segment(self.max_segment)
    }

    fn write(&self, _: u8, _: &[u8]) -> Result<(), DdcErrorKind> {
        Err(DdcErrorKind::Unsupported)
    }

    fn write_read(&self, _: u8, _: &[u8], _: &mut [u8]) -> Result<(), DdcErrorKind> {
        Err(DdcErrorKind::Unsupported)
    }

    fn read(&self, _: u8, _: &mut [u8]) -> Result<(), DdcErrorKind> {
        Err(DdcErrorKind::Unsupported)
    }

    fn read_segmented(
        &self,
        target: EddcTarget,
        segment: u8,
        offset: u8,
        buf: &mut [u8],
    ) -> Result<(), DdcErrorKind> {
        self.calls.borrow_mut().push((segment, offset, buf.len()));
        if target != EddcTarget::Edid || self.max_segment.is_none_or(|max| segment > max) {
            return Err(DdcErrorKind::Unsupported);
        }
        let block = segment * 2 + offset / 128;
        if let Some((_, kind)) = self.fail_from_block.filter(|&(from, _)| block >= from) {
            return Err(kind);
        }
        // A display without the block NACKs: the pointer's segment does not exist.
        let Some(data) = self.blocks.get(usize::from(block)) else {
            return Err(DdcErrorKind::NoAcknowledge(
                hdmi_hal::ddc::NoAcknowledgeSource::Data,
            ));
        };
        buf.copy_from_slice(&data[..buf.len()]);
        Ok(())
    }
}

fn checksum(block: &mut [u8; BLOCK_LEN]) {
    let sum = block[..127].iter().fold(0u8, |a, &b| a.wrapping_add(b));
    block[127] = 0u8.wrapping_sub(sum);
}

/// An EDID 1.4 base block declaring `extensions`, then that many empty CTA extensions.
fn edid(extensions: u8) -> Vec<[u8; BLOCK_LEN]> {
    let mut base = [0u8; BLOCK_LEN];
    base[..8].copy_from_slice(&piaf::parser::EDID_HEADER);
    base[0x12] = 1;
    base[0x13] = 4;
    base[126] = extensions;
    checksum(&mut base);
    let mut blocks = vec![base];
    for _ in 0..extensions {
        let mut ext = [0u8; BLOCK_LEN];
        ext[0] = 0x02;
        ext[1] = 0x03;
        ext[2] = 4;
        checksum(&mut ext);
        blocks.push(ext);
    }
    blocks
}

#[test]
fn reads_every_block_across_segments_and_parses() {
    let store = EdidStore::new(edid(3));
    let mut blocks = [[0u8; BLOCK_LEN]; 8];
    let plan = read_edid(&store, &mut blocks, ReadOptions::new()).unwrap();
    assert_eq!(plan.blocks_read(), 4);
    assert_eq!(plan.warnings().count(), 0);
    assert_eq!(
        *store.calls.borrow(),
        [(0, 0, 128), (0, 128, 128), (1, 0, 128), (1, 128, 128)]
    );
    let read = &blocks.as_flattened()[..usize::from(plan.blocks_read()) * BLOCK_LEN];
    assert!(piaf::parse_edid(read, &piaf::ExtensionLibrary::new()).is_ok());
}

#[test]
fn a_bus_limited_to_segment_0_reads_two_blocks_and_says_so() {
    let mut store = EdidStore::new(edid(3));
    store.max_segment = Some(0);
    let mut blocks = [[0u8; BLOCK_LEN]; 8];
    let plan = read_edid(&store, &mut blocks, ReadOptions::new()).unwrap();
    assert_eq!(plan.blocks_read(), 2);
    assert!(plan.warnings().any(|w| w
        == ReadWarning::BlocksNotRead {
            edid_blocks: 4,
            read: 2
        }));
    assert!(
        store
            .calls
            .borrow()
            .iter()
            .all(|&(segment, ..)| segment == 0)
    );
}

#[test]
fn the_callers_buffer_and_options_bound_the_read_too() {
    let store = EdidStore::new(edid(3));
    let mut blocks = [[0u8; BLOCK_LEN]; 3];
    let plan = read_edid(&store, &mut blocks, ReadOptions::new()).unwrap();
    assert_eq!(plan.blocks_read(), 3);

    let store = EdidStore::new(edid(3));
    let mut blocks = [[0u8; BLOCK_LEN]; 8];
    let plan = read_edid(&store, &mut blocks, ReadOptions::new().with_capacity(2)).unwrap();
    assert_eq!(plan.blocks_read(), 2);

    assert!(matches!(
        read_edid(&store, &mut [], ReadOptions::new()),
        Err(EdidReadError::NoRoom)
    ));
}

#[test]
fn a_bus_error_names_its_block() {
    let mut store = EdidStore::new(edid(3));
    store.fail_from_block = Some((2, DdcErrorKind::Busy));
    let mut blocks = [[0u8; BLOCK_LEN]; 8];
    assert_eq!(
        read_edid(&store, &mut blocks, ReadOptions::new()),
        Err(EdidReadError::Bus {
            block: 2,
            error: DdcErrorKind::Busy
        })
    );
    // One attempt per block: the failed block was read once.
    assert_eq!(store.calls.borrow().len(), 3);

    // A bus that cannot read segments at all fails at the base block.
    let mut store = EdidStore::new(edid(0));
    store.max_segment = None;
    assert_eq!(
        read_edid(&store, &mut blocks, ReadOptions::new()),
        Err(EdidReadError::Bus {
            block: 0,
            error: DdcErrorKind::Unsupported
        })
    );
}

#[test]
fn a_bad_base_block_is_refused_and_a_bad_extension_is_reported() {
    let mut bad_base = edid(1);
    bad_base[0][127] ^= 0xFF;
    let store = EdidStore::new(bad_base);
    let mut blocks = [[0u8; BLOCK_LEN]; 4];
    assert!(matches!(
        read_edid(&store, &mut blocks, ReadOptions::new()),
        Err(EdidReadError::Refused(ReadError::BaseBlock(_)))
    ));

    let mut bad_ext = edid(1);
    bad_ext[1][127] ^= 0xFF;
    let store = EdidStore::new(bad_ext);
    let plan = read_edid(&store, &mut blocks, ReadOptions::new()).unwrap();
    assert_eq!(plan.blocks_read(), 2);
    assert!(
        plan.warnings()
            .any(|w| w == ReadWarning::ChecksumMismatch { block: 1 })
    );
}

#[test]
fn errors_print_and_chain() {
    use std::error::Error;

    #[derive(Debug)]
    struct Bus;
    impl fmt::Display for Bus {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("bus gone")
        }
    }
    impl Error for Bus {}

    let bus = EdidReadError::Bus {
        block: 2,
        error: Bus,
    };
    assert_eq!(bus.to_string(), "reading EDID block 2 failed: bus gone");
    assert!(bus.source().is_some());
    let refused = EdidReadError::<Bus>::Refused(ReadError::Complete);
    assert!(refused.to_string().starts_with("EDID read refused: "));
    assert!(refused.source().is_some());
    let none = EdidReadError::<Bus>::NoRoom;
    assert_eq!(none.to_string(), "no room for an EDID block");
    assert!(none.source().is_none());
}

#[test]
fn a_borrowed_or_unsized_bus_works() {
    let store = EdidStore::new(edid(1));
    let dynamic: &dyn DdcBus<Error = DdcErrorKind> = &store;
    let mut blocks = [[0u8; BLOCK_LEN]; 2];
    assert_eq!(
        read_edid(dynamic, &mut blocks, ReadOptions::new())
            .unwrap()
            .blocks_read(),
        2
    );
}
