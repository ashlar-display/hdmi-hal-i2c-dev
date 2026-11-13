# Architecture

## Role

`hdmi-hal-i2c-dev` implements the `hdmi-hal` traits for Linux userspace, backed by
`/dev/i2c-N` via the `i2c-dev` kernel interface. It serves two distinct audiences:

**Protocol validation.** It is the hardware validation backend for the protocol crates —
the bridge from pure Rust trait implementations to a real HDMI sink on real hardware,
without requiring a kernel module. `culvert` and `plumbob` can be exercised against a real
sink by supplying `hdmi-hal`'s `ScdcOverDdc` over an `I2cDevBus` in place of any other
`ScdcTransport`.

**Diagnostic tooling.** It is the hardware access layer for any tool that needs to inspect
or manipulate SCDC state directly — register dumps, scrambling state inspection, CED
counter monitoring, link status queries. A tool author who has no interest in concordance
or plumbob can depend on this crate alone and use `ScdcOverDdc` over an `I2cDevBus` as a
raw conduit to the SCDC register map, and read the EDID over the same bus.

`DdcBus` is implemented by `I2cDevBus`, which opens the DDC adapter device for an HDMI
connector and issues I²C transfers to the addresses the adapter reaches; its error
classifies itself as a `DdcError`. SCDC (0x54) is `hdmi-hal`'s `ScdcOverDdc` over that bus. `FrlPhy` and `TmdsPhy` are implemented by `StubPhy`, a generic stub that accepts all PHY calls, forwards each as a
`PhyCall` event to a caller-supplied callback, and returns `Ok(())`. PHY programming from
Linux userspace has no standard kernel interface; the stub is sufficient to run the full
FRL training state machine and validate SCDC register logic against a real sink, and lets
callers observe or record the PHY call sequence without requiring hardware.

This crate is explicitly not the production path. The production implementation is a kernel
module operating through the DRM driver's `i2c_adapter`. This crate is the development,
validation, and diagnostic backend, intended for use before and alongside kernel integration.

---

## Scope

`hdmi-hal-i2c-dev` covers:

- `I2cDevBus` — implements `DdcBus` over a `/dev/i2c-N` device file with `I2C_RDWR`,
- `StubPhy<F>` — implements `FrlPhy` and `TmdsPhy` with a caller-supplied callback;
  forwards each PHY call as a `PhyCall` event and returns `Ok(())`,
- `PhyCall` — structured event type describing a single PHY method invocation,
- `connector_ddc_adapter` — resolves a DRM connector name (e.g. `card0-HDMI-A-1`) to its
  DDC adapter device path (`/dev/i2c-N`) via sysfs,
- `edid::read_edid` — reads a sink's EDID over any `DdcBus`, running piaf's I/O-free read
  plan,
- `I2cDevError` — the unified error type for I²C device operations.

The following are out of scope:

- **PHY programming** — there is no standard Linux userspace interface for HDMI PHY
  register access. `StubPhy` stubs all methods. A real PHY backend, if one is ever needed
  in userspace, is a separate crate.
- **CEC** — unrelated to this crate's scope.
- **Multi-connector enumeration** — `connector_ddc_adapter` resolves a single named
  connector. Enumerating all connectors on a system is the caller's responsibility.
- **Hotplug detection** — this crate opens a device, uses it, and closes it. Monitoring
  for connector state changes is out of scope.

`connector_ddc_adapter` is included here for pragmatic reasons, not because it is
correctly scoped. The function resolves a DRM connector name to a device path via sysfs —
an operation with no dependency on SCDC, I²C transactions, or address 0x54. Any tool that
needs DDC bus access for any purpose (EDID reads, CEC, or anything else) requires the same
lookup. Placing it here forces those callers to depend on an SCDC implementation crate to
obtain a general utility function.

It lives here because there is currently no second consumer. When a second crate in this
stack needs DDC adapter resolution — or when `piaf` or any other crate needs to locate a
DDC bus without going through SCDC — `connector_ddc_adapter` should move to a dedicated
`linux-drm` (or similarly named) utility crate, and this crate should take a dependency on
it.

---

## Dependencies

```
hdmi-hal  ──►  hdmi-hal-i2c-dev
i2cdev    ──►  hdmi-hal-i2c-dev
```

- `hdmi-hal` — `DdcBus`, `DdcError`, `ScdcTransport`, `ScdcOverDdc`, `FrlPhy`, `TmdsPhy`, `EqParams`,
  `LanePatterns`, `FrlOutput`, `TmdsPhyConfig`, `TmdsPhyCapabilities`
- `piaf` — `read::EdidReadPlan`, the I/O-free plan `read_edid` runs: which block to read next,
  how many the EDID has, and the checks on each block
- `i2cdev` — safe Rust wrappers over the Linux `i2c-dev` kernel interface; provides the
  `I2C_RDWR`, `I2C_SLAVE` and `I2C_SMBUS` ioctl bindings that back `I2cDevBus`. This is what allows
  `#![forbid(unsafe_code)]` to hold: all unsafe is contained inside `i2cdev`.
- `std` — file I/O, sysfs path resolution

This crate is `std`-only. It has no `no_std` or `alloc` story: its entire purpose is to
interact with the Linux kernel via the filesystem and ioctl interface.

---

## Connector and Adapter Discovery

The Linux DRM subsystem exposes the DDC I²C adapter for each connector through sysfs. For
a connector named `card0-HDMI-A-1`, the adapter is at:

```
/sys/class/drm/card0-HDMI-A-1/ddc
```

This is a symlink. Reading it yields a path relative to the sysfs entry, resolving to the
`i2c-N` adapter directory. The device node is then `/dev/i2c-N` where `N` is the adapter
index extracted from the symlink target.

**Parsing strategy.** Only the final path component of the symlink target is examined.
It must match the pattern `i2c-<N>` where `<N>` is a decimal integer; the prefix (`../..`
or otherwise) is ignored. This makes the implementation robust to changes in the number of
`..` components or the absolute sysfs path structure — only the final component is
load-bearing. If the final component does not match `i2c-<N>`, the function returns
`DdcAdapterIndexUnparseable { symlink_target }` with the full symlink target preserved for
diagnostics. The function does not panic on malformed targets.

`connector_ddc_adapter` performs this resolution:

```rust
/// Resolve a DRM connector name to the path of its DDC I²C adapter device.
///
/// `connector` is the connector name as it appears under `/sys/class/drm/`,
/// for example `"card0-HDMI-A-1"`.
///
/// Returns the path to the `/dev/i2c-N` device, e.g. `/dev/i2c-6`.
///
/// # Errors
///
/// Returns an error if the sysfs entry does not exist, the `ddc` symlink
/// cannot be read, or the symlink target does not contain a recognizable
/// adapter index.
pub fn connector_ddc_adapter(connector: &str) -> Result<PathBuf, I2cDevError>;
```

The typical call sequence to open the bus of a known connector is:

```rust
let path = connector_ddc_adapter("card0-HDMI-A-1")?;
let bus = I2cDevBus::open(path)?;
```

**Hotplug sensitivity.** The `ddc` symlink is created by the kernel when a sink is
connected and removed when it disconnects. Calling `connector_ddc_adapter` with no sink
attached produces `ConnectorHasNoDdcAdapter` — the same error returned for a non-HDMI
port. The two conditions are indistinguishable from this crate's perspective. Callers that
need to distinguish them must monitor connector state through other means (e.g. DRM uevents)
before calling this function.

**TOCTOU.** There is an inherent window between `connector_ddc_adapter` returning a path
and `I2cDevBus::open` consuming it. On systems with dynamic adapter numbering, the
adapter index resolved by `connector_ddc_adapter` may no longer refer to the DDC bus by
the time `open` is called — if an unrelated I²C adapter appeared or disappeared in the
interim, `/dev/i2c-N` may have been reassigned. This window cannot be closed in userspace.
Callers that require precise control over open timing — for example, a privileged parent
opening the device before dropping privileges — need a constructor from an open file, which
`i2cdev` does not offer yet (see the [roadmap](roadmap.md)).

---

## Key Types

### `I2cDevBus`

Implements `DdcBus` with `I2C_RDWR`: every operation is one ioctl, one transaction, made of
messages that each carry their own address.

```rust
pub const MAX_MESSAGE_LEN: usize = 8192;

pub struct I2cDevBus { /* the bus behind a mutex, and its capabilities */ }

impl I2cDevBus {
    /// The capabilities from the `adapters` table, by the adapter's sysfs name.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, I2cDevError>;
    /// The caller asserts what the adapter does with a transfer.
    pub fn open_with(path: impl AsRef<Path>, capabilities: DdcCapabilities)
        -> Result<Self, I2cDevError>;
    /// SMBus calls, for an adapter without plain I²C (`i2c-stub`).
    pub fn open_smbus(path: impl AsRef<Path>) -> Result<Self, I2cDevError>;
    /// The adapter's sysfs name, as `open` read it.
    pub fn adapter(&self) -> Option<&str>;
}

impl DdcBus for I2cDevBus { type Error = I2cDevError; /* … */ }
```

- **Over `i2cdev`'s `LinuxI2CBus`, not `LinuxI2CDevice`.** The device's `transfer` overwrites
  every message's address with its own and marks same-direction neighbours `NO_START`, so an
  E-DDC read across 0x30 and 0x50 would become one merged write to a single address.
- **The operations as messages:** `write` one write; `write_read` a write and a read at the
  same address; `read` one read; `read_segmented` the offset written to the target and the
  read, preceded for segment 1 and higher by the segment written to 0x30, all in one transfer
  (segment 0 needs no pointer, E-DDC 8.1.2). A `segmented_piece` capability splits the read
  into one such transfer per piece.
- **Refused before any ioctl** (`I2cDevError::Refused`, kind `Unsupported`), as the
  `DdcBus` contract asks: a write of no bytes; what the capabilities rule out (an address,
  a length, an E-DDC target or segment); an E-DDC read past the segment's 256 bytes; and a
  message longer than the kernel's 8192 bytes, since `i2cdev` passes lengths as `u16`. An
  empty buffer is `Ok` with no bus activity.
- **Checked after:** a transfer the kernel executed only partly (an adapter with a limit on
  messages per transfer returns how many it ran) is `IncompleteTransfer`, kind `Other`. A
  failed ioctl is `Transfer`, with the first message's address and the errno's class,
  classified as for transactions.
- **The capabilities come from the adapter's name** (`open`): the kernel's functionality
  bits do not describe what a transfer gets, and the crate cannot read them anyway, since
  `i2cdev` never issues `I2C_FUNCS` and the crate forbids `unsafe`. `open` reads
  `/sys/class/i2c-dev/i2c-N/name` and takes the capabilities from the `adapters` table
  below; an adapter not in it, or a name that cannot be read, gets `adapters::SMBUS_LEVEL`.
  `open_with` takes them from the caller, as an assertion.
- **Errnos as the adapter's driver means them.** For an adapter in the table, `EAGAIN` is a
  timeout where its driver returns it for one (dw-hdmi, Inno and rk3066, i915 after a GMBUS
  timeout), and `EINVAL`, with which these drivers refuse what they cannot do, is
  `Unsupported`. Otherwise Linux's convention holds: `EAGAIN` is lost arbitration, `Busy`.

- **SMBus mode** (`open_smbus`), for an adapter without plain I²C such as `i2c-stub`, the
  crate's integration target: every operation is one SMBus call on a device bound to the
  address (`I2C_SLAVE`, re-bound as the address changes, so an address a kernel driver holds
  fails with `Busy`), within `adapters::SMBUS_LEVEL`:

  | Operation | SMBus call |
  |---|---|
  | `write` of 1 byte | write byte |
  | `write` of 2 bytes | write byte data (offset, value) |
  | `write` of 3–33 bytes | I²C block write (offset, up to 32 values) |
  | `write_read` of 1 byte and 1 | read byte data |
  | `write_read` of 1 byte and 2–32 | I²C block read |
  | `read` of 1 byte | read byte |
  | `read_segmented`, segment 0 | I²C block reads of 32 bytes at the target |

  `ENODEV` there is no chip at the address, as `i2c-stub` reports it
  (`I2cErrorKind::NoDevice`, `NoAcknowledge(Address)`); a block read that returns another
  length than asked is `EIO`, with the buffer left as it was. The public type stays
  `I2cDevBus`: its backend, `I2C_RDWR` or SMBus, is the opaque `bus::Backend`.
- **A poisoned lock is recovered,** not a panic: the file descriptor is still good, and a
  transfer that was cut short is the kernel's to have finished or not.
- **A failed transfer does not invalidate the bus.** The file descriptor remains open and
  the next call may succeed; transient bus conditions (glitches, brief clock stretching)
  leave no state behind, and the caller may retry. If errors persist, the sink has most
  likely disconnected: `amdgpu` collapses every failure to `EIO`, so a disconnect cannot
  be told from a persistent bus error. Drop the bus and open it again.

### The `adapters` table

Almost every DRM DDC adapter advertises plain I²C, yet several reshape a transfer and report
success, or limit it and fail with `EINVAL`. The adapter's name identifies it, so the table
maps names to capabilities, from the drivers' transfer functions:

| Adapter name | Driver | Capabilities |
|---|---|---|
| `i915 gmbus …`, `gma500 gmbus …`, `mediatek-hdmi-ddc`, `msm hdmi i2c` | i915, gma500, MediaTek v1, msm | general (msm runs at most 4 messages per transfer, more than any operation here uses) |
| `DesignWare HDMI` | dw-hdmi | `write` of at most 2 bytes, `write_read` with a one-byte write, no `read`, 0x37 unreachable, reads split on the wire; E-DDC segment 0 only |
| `DesignWare HDMI QP` | dw-hdmi-qp | as dw-hdmi, and no DisplayID target |
| `Inno HDMI`, `RK3066 HDMI` | inno-hdmi, rk3066 | only `read_segmented` of EDID |
| `oaktrail_hdmi_i2c` | gma500 (Oaktrail) | only `read_segmented` of EDID, segment 0 |
| `mediatek-hdmi-ddc-v2` | MediaTek v2 | 0x50 and 0x54 only, no `read`; E-DDC segment 0 only |
| `sun4i_hdmi_i2c adapter` | sun4i | messages of at most 1023 bytes; E-DDC segment 0 only |
| `AMDGPU DM i2c …` | amdgpu (DC) | a 144-byte buffer, address bytes included: `write` 143, `write_read` 1 + 128, `read` 143, E-DDC in 128-byte pieces |

amdgpu's buffer is 538 bytes from DCE 10.0, but the generation is not visible from the
name, so the table takes the smaller one. Adapters bit-banged through `i2c-algo-bit`
(radeon, amdgpu before DC, nouveau) are not in it: their engines were not audited.
`adapters::SMBUS_LEVEL`, for an adapter not in the table, is what the SMBus calls of the
past could do: `write` 33, `write_read` 1 + 32, `read` 1, E-DDC segment 0 only in 32-byte
pieces.

### SCDC over the bus

SCDC is `hdmi-hal`'s `ScdcOverDdc` over an `I2cDevBus`, borrowed (`&I2cDevBus`) so that the
EDID reader and SCDC share the bus:

```rust
let bus = I2cDevBus::open(path)?;
let scdc = ScdcOverDdc::new(&bus);       // `read_block` in one transfer
let scdc = ScdcOverDdc::bytewise(&bus);  // `read_block` a register at a time
```

- **Register read:** a `write_read` of the offset and one byte. **Write:** a `write` of
  offset and value. **Block read:** one `write_read` of the offset and the block, within the
  bus's capabilities: at `SMBUS_LEVEL` up to 32 bytes, so a longer block is refused, not
  split; on an `I2C_RDWR` adapter in the table, as long as its `write_read` allows.
- **Which constructor.** `new` needs a `write_read` of at least 16 bytes, `bytewise` of
  one; both need SCDC reachable and a two-byte `write`, and hand the bus back otherwise. An
  adapter that is only an EDID engine (Inno, RK3066, Oaktrail) has no SCDC: both refuse it.
  `bytewise` is not one transaction per block, so it is the caller's explicit choice.
- **One transfer is not always one transaction on the wire.** The kernel holds the adapter
  for the duration of the ioctl, so another process's transfer cannot land inside it, but
  what reaches the wire is the adapter driver's. On `i915`'s GMBUS and `amdgpu`'s DC a block
  read is one transaction, the offset write and a repeated-START read; the Synopsys
  DesignWare HDMI controller's I²C master (`dw-hdmi`, `dw-hdmi-qp`) reads one byte per
  hardware operation, each with its own offset write, and its capabilities say so
  (`splits_reads`). That matters for registers read together, such as the CED counters,
  which may increment between two reads: culvert's CED checksum is the one protection that
  holds on every adapter.

### `StubPhy<F>`

Implements `FrlPhy` and `TmdsPhy` with a caller-supplied callback. Each method constructs
a `PhyCall` variant describing the invocation, passes it to the callback, and returns
`Ok(())`. The callback owns the side effect — logging, recording, asserting — and
`StubPhy` itself is unconditionally infallible. It keeps no state, so it does not refuse
`start_output` before `configure` as a real PHY does.

```rust
pub struct StubPhy<F> { /* private */ }

impl<F> StubPhy<F> {
    /// 600 MHz, a scrambler, and scrambling at or below 340 Mcsc.
    pub const DEFAULT_CAPABILITIES: TmdsPhyCapabilities;
}

impl<F: FnMut(PhyCall)> StubPhy<F> {
    pub fn new(on_call: F) -> Self;
    pub fn with_capabilities(self, capabilities: TmdsPhyCapabilities) -> Self;
}

impl<F: FnMut(PhyCall)> FrlPhy for StubPhy<F> {
    type Error = Infallible;

    fn set_frl_rate(&mut self, rate: HdmiForumFrl) -> Result<(), Infallible>;
    fn send_ltp(&mut self, patterns: LanePatterns) -> Result<(), Infallible>;
    fn set_frl_output(&mut self, output: FrlOutput) -> Result<(), Infallible>;
    fn adjust_equalization(&mut self, params: EqParams) -> Result<(), Infallible>;
}

impl<F: FnMut(PhyCall)> TmdsPhy for StubPhy<F> {
    type Error = Infallible;

    fn capabilities(&self) -> TmdsPhyCapabilities;
    fn configure(&mut self, config: TmdsPhyConfig) -> Result<(), Infallible>;
    fn set_scrambling(&mut self, enabled: bool) -> Result<(), Infallible>;
    fn start_output(&mut self) -> Result<(), Infallible>;
    fn stop_output(&mut self) -> Result<(), Infallible>;
}
```

Its capabilities default to everything TMDS allows, so that a caller choosing a link from
them is limited by the sink alone; `with_capabilities` sets others, a PHY without a
scrambler for example.

`type Error = Infallible` is unconditionally correct: the callback returns `()`, so
`StubPhy` cannot fail regardless of what the callback does. Callers that propagate
`TrainingError<_, Infallible>` can statically observe that PHY errors are impossible.

Typical uses:

```rust
// No-op: discard all PHY calls.
let phy = StubPhy::new(|_| {});

// Logging: print each call to stderr.
let phy = StubPhy::new(|call| eprintln!("{call:?}"));

// Test assertion: send calls to a channel for inspection.
let phy = StubPhy::new(|call| tx.send(call).unwrap());
```

### `PhyCall`

A structured description of a single `FrlPhy` or `TmdsPhy` method invocation, used as the
callback argument for `StubPhy`.

```rust
#[derive(Debug)]
#[non_exhaustive]
pub enum PhyCall {
    SetFrlRate(HdmiForumFrl),
    SendLtp(LanePatterns),
    SetFrlOutput(FrlOutput),
    AdjustEqualization(EqParams),
    SetScrambling(bool),
    Configure(TmdsPhyConfig),
    StartOutput,
    StopOutput,
}
```

`#[non_exhaustive]` is intentional: if the PHY traits gain new methods, `PhyCall` gains new
variants. Downstream crates that match on `PhyCall` — test harnesses, diagnostic tools —
should not be forced to recompile and update exhaustive matches for methods they do not
care about. `StubPhy`'s own trait impls are unaffected; the compiler enforces their
coverage through the trait, not through the enum.

### `edid::read_edid`

A thin driver for piaf's I/O-free EDID read plan over any `DdcBus`, not only this crate's.
The plan holds the structure knowledge (which block comes next, at which E-DDC segment and
offset, how many blocks the EDID has from byte 126 and HF-EEODB, and the checks on each
block); the driver reads the block it names with `read_segmented` and hands it back, until
the plan is done.

```rust
pub const BLOCK_LEN: usize = 128;

pub fn read_edid<B: DdcBus + ?Sized>(
    bus: &B,
    blocks: &mut [[u8; BLOCK_LEN]],
    options: ReadOptions,
) -> Result<EdidReadPlan, EdidReadError<B::Error>>;

#[non_exhaustive]
pub enum EdidReadError<E> {
    Bus { block: u8, error: E },   // the bus failed reading `block`
    Refused(ReadError),            // the base block failed its checks
    NoRoom,                        // `blocks` is empty
}
```

- **What bounds the read:** the plan reads no more blocks than `options` allow, `blocks`
  holds, or the bus reaches (`DdcCapabilities::max_segment`, two blocks per segment: an
  EDID engine that reads segment 0 only gets two). An EDID with more blocks than that is the
  plan's `BlocksNotRead` warning, not an error, so a caller learns how much it missed.
- **One attempt per block.** Retries are policy: Linux reads a block up to four times,
  Amlogic once, and a sink with a corrupt EDID returns a bad base block every time, so only
  the caller can bound them. A bus error names its block and keeps the bus's error, whose
  `DdcErrorKind` says whether reading again may help.
- **No allocation:** the blocks are the caller's buffer, and the plan keeps no copy. The
  blocks read go to `piaf::parse_edid` as `blocks.as_flattened()`.

### `I2cDevError`

The error type for all fallible operations in this crate.

```rust
#[non_exhaustive]
pub enum I2cDevError {
    /// The specified DRM connector was not found in sysfs.
    ConnectorNotFound { connector: String },
    /// The connector exists in sysfs but has no `ddc` symlink.
    /// Common for non-HDMI connectors and HDMI ports with no sink attached.
    ConnectorHasNoDdcAdapter { connector: String },
    /// The `ddc` symlink exists but its target does not contain a recognizable
    /// adapter index. This indicates an unexpected sysfs layout.
    DdcAdapterIndexUnparseable { symlink_target: PathBuf },
    /// The `/dev/i2c-N` device could not be opened.
    DeviceOpenFailed { path: PathBuf, source: io::Error },
    /// An `I2C_RDWR` transfer, or an SMBus call, failed.
    #[non_exhaustive]
    Transfer { address: u8, kind: I2cErrorKind },
    /// The kernel executed fewer of a transfer's messages than were sent.
    #[non_exhaustive]
    IncompleteTransfer { sent: u32, done: u32 },
    /// Refused before any ioctl.
    #[non_exhaustive]
    Refused { reason: Refusal },
}

/// Why an operation was refused: a write of no bytes, a message too long, an address,
/// E-DDC target or segment the capabilities do not reach, or a read past the segment.
#[non_exhaustive]
pub enum Refusal {
    EmptyWrite,
    TooLong { len: usize, max: usize },
    Unreachable { address: u8 },
    Target { address: u8 },
    Segment { segment: u8 },
    PastSegmentEnd { offset: u8, len: usize },
}

/// The class of I²C bus condition that caused the failure.
///
/// Variants follow the conventions in `Documentation/i2c/fault-codes.rst`.
/// See the note on `amdgpu` below for current hardware limitations.
#[non_exhaustive]
pub enum I2cErrorKind {
    /// The slave did not acknowledge its I²C address byte (`ENXIO`).
    AddressNack,
    /// The slave acknowledged its address but NACKed a data byte.
    DataNack,
    /// Arbitration was lost to another bus master (`EAGAIN`). Retry is appropriate.
    ArbitrationLost,
    /// The slave held SCL low beyond the adapter timeout threshold (`ETIMEDOUT`).
    ClockStretchTimeout,
    /// The bus was busy and the transaction could not be started (`EBUSY`).
    BusBusy,
    /// An unexpected START or STOP condition was observed on the bus (`EPROTO`).
    BusProtocolError,
    /// The I²C adapter was suspended or removed (`ESHUTDOWN`/`ENODEV`).
    AdapterGone,
    /// No chip at the address: `ENODEV` on a bus opened with `open_smbus`, as `i2c-stub`
    /// reports it.
    NoDevice,
    /// The adapter does not support the operation (`EOPNOTSUPP`).
    Unsupported,
    /// An errno that does not map to any known I²C condition.
    Unknown { errno: i32 },
}
```

In SMBus mode, a driver that reports more than 32 bytes for an I²C block read makes
`i2cdev` panic (it indexes its 34-byte buffer by the reported length); the bus's lock
recovers from it, so later calls are unaffected.

---

## Interface Boundaries

### Below: Linux `i2c-dev` kernel interface

`I2cDevBus` is backed by `ioctl(fd, I2C_RDWR, ...)` per transfer, or in SMBus mode by
`ioctl(fd, I2C_SLAVE, address)` when the address changes and `ioctl(fd, I2C_SMBUS, ...)` per
call, against an open `/dev/i2c-N` file descriptor. The `i2c-dev` kernel module must be loaded; on most Linux desktop systems
it is either built in or loaded automatically. The calling process needs read/write
permission on the device node (typically via the `i2c` group).

The SCDC slave address (0x54) is fixed by HDMI 2.1; `ScdcOverDdc` addresses it, and no
caller passes it.

#### errno mapping and the `amdgpu` limitation

The Linux I²C subsystem specifies a standard errno vocabulary for distinct bus conditions
(`Documentation/i2c/fault-codes.rst`): `ENXIO` for address-phase NACK, `EAGAIN` for
arbitration loss, `ETIMEDOUT` for timeout, `EBUSY` for bus-busy. `I2cErrorKind` is
designed against this vocabulary.

In practice, the `amdgpu` DDC adapter does not follow this convention. Tracing the call
chain from `amdgpu_dm_i2c_xfer` through `dc_submit_i2c` and `dce_i2c_submit_command_hw`
shows that all `i2c_channel_operation_result` values — `NO_RESPONSE` (NACK), `TIMEOUT`,
`ENGINE_BUSY`, `OUT_NB_OF_RETRIES`, and all others — are collapsed to a boolean at each
layer boundary. `amdgpu_dm_i2c_xfer` maps `false` unconditionally to `-EIO`. Every
transaction failure surfaces as `EIO` regardless of the actual bus condition.

On `amdgpu`, a failed transfer's `I2cErrorKind` will always be `Unknown { errno: EIO }`.
The named variants (`AddressNack`, `Timeout`, etc.) are unreachable on this hardware. This
is a driver deficiency — the driver has the information internally and discards it — not a
kernel interface limitation. The richer variant set is retained because it is correct per
the kernel's documented vocabulary and meaningful on compliant adapters.

### Above: `hdmi-hal` traits

`I2cDevBus` satisfies `DdcBus` as defined in `hdmi-hal`, and `ScdcOverDdc` over it
`ScdcTransport`. `StubPhy<F>` satisfies `FrlPhy` and `TmdsPhy`. Callers construct these types and pass them — by value or
by `&mut` reference — to any API that accepts the corresponding trait. Nothing in this crate
is specific to `culvert`, `plumbob` or `spiritlevel`; those crates accept any
`ScdcTransport`, `FrlPhy` and `TmdsPhy` respectively.

The typical end-to-end validation setup:

```rust
let path = connector_ddc_adapter("card0-HDMI-A-1")?;
let bus = I2cDevBus::open(path)?;
let transport = ScdcOverDdc::new(&bus).expect("the adapter reaches SCDC");
let scdc = Scdc::new(transport);                            // culvert
let phy = StubPhy::new(|call| eprintln!("{call:?}"));      // log PHY calls to stderr
let mut trainer = FrlTrainer::new(scdc, phy);               // plumbob
let outcome = trainer.train_at_rate(HdmiForumFrl::Rate6Gbps4Lanes, &TrainingConfig::default())?;
```

---

## `std`-only

This crate is unconditionally `std`. There is no `no_std` path, no `alloc` feature, and no
async companion planned. The entire purpose of this crate is filesystem and ioctl access,
which requires `std`. Any caller that needs `no_std` link training must supply their own
`ScdcTransport` implementation for their platform.

---

## Test Strategy

Testing is split into two tiers based on what each tier actually validates.

### Tier 1: no-hardware tests

**`connector_ddc_adapter`** is tested against a synthetic sysfs tree constructed in a
`tempfile` temporary directory. The test creates the expected symlink structure under a
configurable root, then calls `connector_ddc_adapter` with that root substituted for
`/sys/class/drm`. This covers:

- successful resolution of a valid connector,
- `ConnectorNotFound` when the connector directory is absent,
- `ConnectorHasNoDdcAdapter` when the connector exists but has no `ddc` symlink,
- `DdcAdapterIndexUnparseable` when the symlink target is malformed.

**`I2cErrorKind` mapping** is tested as a unit test against the internal function that
maps errnos to `I2cErrorKind`. Specific errno values are passed directly and the output
variant is asserted. This covers every
named `I2cErrorKind` variant and the `Unknown { errno }` fallback without kernel
involvement.

**`I2cDevBus`** is tested through crate-private seams: a mock of the `I2C_RDWR` transfer
that records each message with its address, and a mock register file behind the SMBus
calls. Each operation's messages or calls, every refusal, the error mapping by adapter, and
`open` against a synthetic sysfs tree are covered; see [`testing.md`](testing.md).

No kernel involvement. Runs in CI on any Linux host.

### Tier 2: `i2c-stub` integration tests

**`I2cDevBus` in SMBus mode**, and SCDC over it, are tested against the Linux `i2c-stub`
kernel module, which registers a fake I²C device at a specified address and exposes it as a
real `/dev/i2c-N` device node. Calls go through the full `I2C_SMBUS` ioctl path; the stub
responds to SMBus byte-data reads and I2C block reads and records writes.

This tier validates:
- SCDC register reads, writes and block reads (9 bytes, the CED block; 2 bytes; the
  32-byte maximum, each in one ioctl; a 33-byte read refused by the capabilities; and a
  register at a time with `bytewise`),
- the bus's own SMBus calls, binding another address and back, and the adapter's sysfs
  name,
- `NoAcknowledge(Address)` when a transfer targets an address with no chip.

These tests require `i2c-stub` to be loaded (`modprobe i2c-stub`) and the calling process
to have write permission on the resulting device node. They are gated behind a feature flag
(`--features integration`) and are not run in standard CI. They are the authoritative test
of the SMBus path and must be run before any release. The `I2C_RDWR` path has no such target
(`i2c-stub` offers SMBus only): its unit tests use a mock that records each message's
address.

**Error conditions not covered:** bus errors, clock stretch timeouts, and arbitration loss
cannot be simulated via `i2c-stub`. `i2c-stub` always ACKs registered addresses; the only
error condition it can produce is a transfer to an unregistered address, which it reports
as `ENODEV` (`NoDevice` on a bus opened for SMBus) rather than the `ENXIO` (`AddressNack`)
of a real adapter's NACK. The mapping from
`LinuxI2CError` to these variants is covered by the Tier 1 unit tests; exercising them
end-to-end requires real misbehaving hardware.

`i2cdev`'s `MockI2CDevice` is intentionally not used for bus tests. The mock does
not validate message structure, addresses, or flags — it would test `i2cdev`'s own mock
rather than this crate's message construction logic. The mock is appropriate for callers
of `ScdcTransport` (`culvert`, `plumbob`) where register-level behaviour matters; it is
not appropriate here.

---

## Design Principles

- **Follows `linux-embedded-hal` precedent.** Platform backends that implement
  hardware-abstraction traits for a specific OS or environment are a well-established
  pattern in the embedded Rust ecosystem. This crate applies that pattern to the HDMI stack.
- **Explicit about what it is not.** `StubPhy` does not silently pretend to configure
  hardware; its `Infallible` error type communicates its nature to the type system, and the
  crate documentation makes the non-production status prominent.
- **Extensible without forking.** Public types are designed so that downstream crates can
  extend behaviour — through callbacks, trait implementations, or wrapper types — without
  requiring changes to this crate. `#[non_exhaustive]` is applied to types that will grow
  alongside the rest of the stack (`I2cDevError`, `I2cErrorKind`, `PhyCall`), so that
  adding variants does not force dependent crates to update exhaustive matches they do not
  own.
- **Minimal scope.** This crate does exactly two things: implement `ScdcTransport` over
  `i2c-dev`, and stub `FrlPhy` and `TmdsPhy`. Discovery helpers are included because they are inseparable
  from usability, not because this is a general-purpose DRM sysfs library.
- **No unsafe code.** `#![forbid(unsafe_code)]`. I²C device access is performed through
  `i2cdev`, which provides safe Rust wrappers over the `i2c-dev` kernel interface. All
  unsafe is contained within that crate.
- **Structured errors.** `I2cDevError` distinguishes connector-not-found from device
  open failure from transaction failure. A caller diagnosing a problem can act on the
  variant rather than parsing an error message.

---

## Open Items

**Multi-card support** — `connector_ddc_adapter` assumes the sysfs path structure is
`/sys/class/drm/<connector>/ddc`. Systems with multiple DRM cards are expected to follow
the same structure (e.g. `card1-HDMI-A-1`). This has not been tested on a multi-GPU system
and may require adjustment if the sysfs layout differs.
