//! Reading a sink's EDID over any [`DdcBus`]: piaf's read plan, run block by block.
//!
//! [`read_edid`] is a thin driver: piaf's [`EdidReadPlan`] says which block to read next and
//! checks what was read, and the bus reads it with an E-DDC
//! [`read_segmented`](DdcBus::read_segmented). It works over any [`DdcBus`], not only this
//! crate's, and reads into the caller's buffer, so it allocates nothing.
//!
//! It makes one attempt per block. Whether to read a block again after a bus error, or a
//! base block that fails its checks, is the caller's: the reference implementations differ
//! (Linux reads a block up to four times, Amlogic once), and a sink with a corrupt EDID
//! returns a bad base block every time, so only the caller can bound the retries.

use core::fmt;

use hdmi_hal::ddc::{DdcBus, EddcTarget};
use piaf::read::{EdidReadPlan, ReadError, ReadOptions};

/// The length of one EDID block.
pub const BLOCK_LEN: usize = 128;

/// Why [`read_edid`] stopped.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub enum EdidReadError<E> {
    /// The bus failed reading block `block`: what is in `blocks` from it on is not the
    /// sink's.
    #[non_exhaustive]
    Bus {
        /// The block being read: 0 for the base block, `n` for extension `n`.
        block: u8,
        /// The bus's error; its [`DdcError::kind`](hdmi_hal::ddc::DdcError::kind) says
        /// whether reading again may help.
        error: E,
    },
    /// The plan refused a block: the base block failed the checks `parse_edid` makes, so
    /// its block count cannot be trusted.
    Refused(ReadError),
    /// The caller's buffer holds no block.
    NoRoom,
}

impl<E: fmt::Display> fmt::Display for EdidReadError<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bus { block, error } => write!(f, "reading EDID block {block} failed: {error}"),
            Self::Refused(error) => write!(f, "EDID read refused: {error}"),
            Self::NoRoom => f.write_str("no room for an EDID block"),
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for EdidReadError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Bus { error, .. } => Some(error),
            Self::Refused(error) => Some(error),
            Self::NoRoom => None,
        }
    }
}

/// Reads the sink's EDID over `bus` into `blocks`, as piaf's read plan directs, and returns
/// the finished plan: [`blocks_read`](EdidReadPlan::blocks_read) blocks of `blocks` hold the
/// EDID, and [`warnings`](EdidReadPlan::warnings) says what went wrong without stopping the
/// read. Hand `blocks.as_flattened()`, up to the blocks read, to `piaf::parse_edid`.
///
/// Block `n` is read with `read_segmented` at segment `n / 2`, offset `0` or `128`, one
/// block per call. The plan reads no more blocks than `options` allow, `blocks` holds, or
/// the bus reaches ([`DdcCapabilities::max_segment`](hdmi_hal::ddc::DdcCapabilities::max_segment):
/// two blocks per segment). An EDID with more blocks than that is reported by the plan as
/// [`BlocksNotRead`](piaf::read::ReadWarning::BlocksNotRead), not as an error.
///
/// # Errors
///
/// [`EdidReadError::Bus`] for a failed read, with the block; [`EdidReadError::Refused`] for
/// a base block that fails its checks; [`EdidReadError::NoRoom`] when `blocks` is empty. One
/// attempt each: see the [module documentation](self).
pub fn read_edid<B: DdcBus + ?Sized>(
    bus: &B,
    blocks: &mut [[u8; BLOCK_LEN]],
    options: ReadOptions,
) -> Result<EdidReadPlan, EdidReadError<B::Error>> {
    if blocks.is_empty() {
        return Err(EdidReadError::NoRoom);
    }
    let reach = bus
        .capabilities()
        .max_segment()
        .map_or(0, |max| (u16::from(max) + 1) * 2);
    let held = u16::try_from(blocks.len()).unwrap_or(u16::MAX);
    let capacity = options.capacity.min(held).min(reach.max(1));
    let mut plan = EdidReadPlan::new(options.with_capacity(capacity));
    while let Some(at) = plan.next() {
        let block = &mut blocks[usize::from(at.block())];
        bus.read_segmented(EddcTarget::Edid, at.segment(), at.offset(), block)
            .map_err(|error| EdidReadError::Bus {
                block: at.block(),
                error,
            })?;
        plan.push(block).map_err(EdidReadError::Refused)?;
    }
    Ok(plan)
}

#[cfg(test)]
#[path = "edid_tests.rs"]
mod tests;
