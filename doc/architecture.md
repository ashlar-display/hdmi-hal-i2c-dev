# Architecture

## Role

`hdmi-hal-i2c-dev` implements the `hdmi-hal` traits for Linux userspace, backed by
`/dev/i2c-N` via the `i2c-dev` kernel interface. It serves two distinct audiences:

**Protocol validation.** It is the hardware validation backend for the protocol crates —
the bridge from pure Rust trait implementations to a real HDMI sink on real hardware,
without requiring a kernel module. `culvert` and `plumbob` can be exercised against a real
sink by supplying an `I2cDevTransport` in place of any other `ScdcTransport`.

**Diagnostic tooling.** It is the hardware access layer for any tool that needs to inspect
or manipulate SCDC state directly — register dumps, scrambling state inspection, CED
counter monitoring, link status queries. A tool author who has no interest in concordance
or plumbob can depend on this crate alone and use `I2cDevTransport` as a raw conduit to
the SCDC register map.

`ScdcTransport` is implemented by opening the DDC adapter device for an HDMI connector
and issuing raw I²C transactions to the SCDC slave address (0x54); its error classifies
itself as a `DdcError`. `FrlPhy` and `TmdsPhy` are implemented by `StubPhy`, a generic stub that accepts all PHY calls, forwards each as a
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

- `I2cDevTransport` — implements `ScdcTransport` over a `/dev/i2c-N` device file,
- `StubPhy<F>` — implements `FrlPhy` and `TmdsPhy` with a caller-supplied callback;
  forwards each PHY call as a `PhyCall` event and returns `Ok(())`,
- `PhyCall` — structured event type describing a single PHY method invocation,
- `connector_ddc_adapter` — resolves a DRM connector name (e.g. `card0-HDMI-A-1`) to its
  DDC adapter device path (`/dev/i2c-N`) via sysfs,
- `I2cDevError` — the unified error type for I²C device operations.

The following are out of scope:

- **PHY programming** — there is no standard Linux userspace interface for HDMI PHY
  register access. `StubPhy` stubs all methods. A real PHY backend, if one is ever needed
  in userspace, is a separate crate.
- **EDID reads** — EDID is read over the DDC bus at I²C address 0x50, not 0x54. This crate
  only implements `ScdcTransport`, which operates against the SCDC slave at 0x54. EDID
  parsing belongs in `piaf`; reading the raw bytes is the concern of whatever integration
  layer calls `piaf`. (Planned to change with the `DdcBus` implementation, which reads EDID
  over E-DDC: see the [roadmap](roadmap.md).)
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

- `hdmi-hal` — `ScdcTransport`, `DdcError`, `FrlPhy`, `TmdsPhy`, `EqParams`,
  `LanePatterns`, `FrlOutput`, `TmdsPhyConfig`, `TmdsPhyCapabilities`
- `i2cdev` — safe Rust wrappers over the Linux `i2c-dev` kernel interface; provides the
  `I2C_SMBUS` ioctl binding that backs `I2cDevTransport`. This is what allows
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

The typical call sequence to build a transport for a known connector is:

```rust
let path = connector_ddc_adapter("card0-HDMI-A-1")?;
let transport = I2cDevTransport::open(path)?;
```

**Hotplug sensitivity.** The `ddc` symlink is created by the kernel when a sink is
connected and removed when it disconnects. Calling `connector_ddc_adapter` with no sink
attached produces `ConnectorHasNoDdcAdapter` — the same error returned for a non-HDMI
port. The two conditions are indistinguishable from this crate's perspective. Callers that
need to distinguish them must monitor connector state through other means (e.g. DRM uevents)
before calling this function.

**TOCTOU.** There is an inherent window between `connector_ddc_adapter` returning a path
and `I2cDevTransport::open` consuming it. On systems with dynamic adapter numbering, the
adapter index resolved by `connector_ddc_adapter` may no longer refer to the DDC bus by
the time `open` is called — if an unrelated I²C adapter appeared or disappeared in the
interim, `/dev/i2c-N` may have been reassigned. This window cannot be closed in userspace.
Callers that require precise control over open timing — for example, a privileged parent
opening the device before dropping privileges — should open the file themselves and use
`LinuxI2CDevice::new` directly with the raw file descriptor.

---

## Key Types

### `I2cDevTransport`

Implements `ScdcTransport` for a `/dev/i2c-N` device. On construction it opens the device
file and sets the slave address to 0x54 via `I2C_SLAVE`; each `read` and `write` call
issues an SMBus transaction to the SCDC slave address.

The SCDC register protocol is:
- **Write:** `smbus_write_byte_data(reg, value)` — an `I2C_SMBUS` ioctl issuing a two-byte
  write containing the register address byte followed by the value byte.
- **Read:** `smbus_read_byte_data(reg)` — an `I2C_SMBUS` ioctl issuing a combined
  write-then-read: the register address byte is written, then one byte is read back, all
  within a single ioctl call.
- **Block read** (`read_block`): `smbus_read_i2c_block_data(reg, len)` — an `I2C_SMBUS`
  ioctl with `I2C_SMBUS_I2C_BLOCK_DATA`: the register address byte is written, then, after
  a repeated START, `len` bytes are read, all within a single ioctl call. At most 32 bytes
  (`I2C_SMBUS_BLOCK_MAX`, exported as `transport::MAX_BLOCK_LEN`); a longer buffer is
  `I2cDevError::BlockReadTooLong` before any ioctl, never split into several reads. An
  empty buffer is `Ok` without an ioctl; a result of another length is
  `I2cDevError::UnexpectedBlockLength`, never padded. Past 0xFF the result is the sink's on
  real hardware; `i2c-stub` ends the read at 0xFF (`UnexpectedBlockLength`).
- **Block read, opened with `open_bytewise`**: `read_block_bytewise`, one byte-data read
  per register, for an adapter without the SMBus I2C block read. Not one transaction, no
  length limit, wrapping past 0xFF.

The operations are issued via `LinuxI2CDevice` from `i2cdev`, each as one ioctl. The kernel
holds the adapter for the duration of the ioctl, so another process's transfer cannot land
inside it. What reaches the wire is the adapter driver's: this crate controls the ioctl, not
the bus. For the DRM DDC adapters checked (`i915`'s GMBUS and `amdgpu`'s DC, the latter on
hardware too) a block read is one transaction on the wire, the offset write and a
repeated-START read; the Synopsys DesignWare HDMI controller's I²C master (`dw-hdmi`,
`dw-hdmi-qp`) reads one byte per hardware operation, each with its own offset write, while
advertising `I2C_FUNC_I2C | I2C_FUNC_SMBUS_EMUL` (`dw_hdmi_i2c_read`), so there a block
read returns `Ok` with its registers read one at a time. Other adapters are unchecked.
That matters for registers read together, such as the CED counters, which may increment
between two reads: culvert's CED checksum is the one protection that holds on every
adapter.

SMBus byte-data semantics are equivalent to raw I²C for SCDC's single-byte register model,
and work on both `i2c-stub` (for integration tests) and real DDC adapters.

**Why the block read is an SMBus I2C block read.** `hdmi-hal`'s `read_block` contract asks
for one combined transfer to the layer below, or an error. Two ioctls give one: `I2C_RDWR` with a write message and a
read message, or the SMBus I2C block read. Both put the same transaction on the wire — the
offset written, a repeated START, the bytes read — which is what the kernel's own
`drm_scdc_read` and embedded `I2c::write_read` backends issue. `I2C_RDWR` has no 32-byte
limit, but needs `I2C_FUNC_I2C`, which `i2c-stub` does not offer: the Tier 2 tests could
not run it, and on an adapter without it `read_ltp_requests`, polled in LTS:3, would fail
where byte reads work today. The SMBus I2C block read is in `i2c-stub`'s default
functionality and in `I2C_FUNC_SMBUS_EMUL`, which the DRM DDC adapters advertise (`i915`'s
GMBUS and `amdgpu`'s DC: `I2C_FUNC_I2C | I2C_FUNC_SMBUS_EMUL`); on them the kernel
emulates it as the offset write and a repeated-START read. Its 32-byte limit is what
`read_block`'s contract allows a backend to document; culvert's block reads are 2 and 9
bytes, and a register dump reads the map in parts. There is no fallback to one register at
a time: an adapter that cannot do the block read returns the ioctl's error, typically
`EOPNOTSUPP`, which is `I2cErrorKind::Unsupported`.

The trade-off is accepted, not avoided: `I2C_RDWR` would fail on adapters without
`I2C_FUNC_I2C`, and the SMBus I2C block read fails on SMBus-only adapters without
`I2C_FUNC_SMBUS_READ_I2C_BLOCK`, where byte reads worked before; on those, LTS:3 now fails.
The error surfaces at the first block read, not at open: an up-front check needs the
`I2C_FUNCS` ioctl, which `i2cdev` 0.6.2 does not expose, and `#![forbid(unsafe_code)]` rules
out issuing it here (a functionality check at open is planned once `i2cdev` offers one). For
such an adapter, `I2cDevTransport::open_bytewise` reads a block a register at a time, as an
explicit choice.

```rust
pub struct I2cDevTransport { /* LinuxI2CDevice in Mutex */ }

impl I2cDevTransport {
    /// Open the given `/dev/i2c-N` device for use as an SCDC transport.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, I2cDevError>;
    /// The same, with `read_block` one register at a time (not one transaction).
    pub fn open_bytewise(path: impl AsRef<Path>) -> Result<Self, I2cDevError>;
}

impl ScdcTransport for I2cDevTransport {
    type Error = I2cDevError;

    fn read(&self, reg: u8) -> Result<u8, I2cDevError>;
    fn write(&mut self, reg: u8, value: u8) -> Result<(), I2cDevError>;
    fn read_block(&self, reg: u8, buf: &mut [u8]) -> Result<(), I2cDevError>;  // ≤ 32 bytes (`open`)
}

/// The longest block read.
pub const MAX_BLOCK_LEN: usize = 32;
```

**Error recovery policy.** A `Transaction` error does not invalidate the transport. The
file descriptor remains open and the next call may succeed — transient bus conditions
(glitches, brief clock stretching) do not corrupt any transport state. The caller may
retry immediately.

If errors persist, the sink has most likely disconnected. There is no `is_connected()`
method and, due to the `amdgpu` driver collapsing all transaction failures to `EIO`, a
disconnect is indistinguishable from a persistent bus error at the API level. The
caller should drop the transport and reconstruct it — via `connector_ddc_adapter` +
`open`, or via `from_file` if controlling the fd lifecycle externally.

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
    /// An I²C transaction failed.
    Transaction(I2cTransactionError),
    /// A block read longer than `MAX_BLOCK_LEN`; nothing was read.
    #[non_exhaustive]
    BlockReadTooLong { register: u8, len: usize, max: usize },
    /// The kernel returned another number of bytes than the block read asked for: an
    /// adapter driver reporting another length, or `i2c-stub` ending a read at 0xFF.
    #[non_exhaustive]
    UnexpectedBlockLength { register: u8, requested: usize, returned: usize },
}

/// The phase and kind of an I²C transaction failure.
pub struct I2cTransactionError {
    pub phase: MessagePhase,
    pub kind: I2cErrorKind,
}

/// Which phase of the SCDC transaction failed.
///
/// The current SMBus-backed implementation always reports `Write` — the
/// `I2C_SMBUS` ioctl does not distinguish between the outbound address byte
/// and the inbound data byte when returning an error. `Read` is retained for
/// future implementations that use raw `I2C_RDWR` and can isolate the failing
/// message by index.
pub enum MessagePhase {
    /// The write-phase message failed (register address byte, or write data).
    Write { register: u8 },
    /// The read-phase message failed (data retrieval).
    Read { register: u8 },
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
    /// The I²C adapter was suspended or removed (`ESHUTDOWN`/`ENODEV`). `ENODEV` can also
    /// mean no device at all: `i2c-stub` returns it for an address with no chip.
    AdapterGone,
    /// The adapter does not support the operation (`EOPNOTSUPP`): for a block read, no
    /// SMBus I2C block read; use `open_bytewise`.
    Unsupported,
    /// An errno that does not map to any known I²C condition.
    Unknown { errno: i32 },
}
```

The two block-read errors are `#[non_exhaustive]` per variant, so a later release can add a
field to either. A driver that reports more than 33 bytes for a block read does not reach
`UnexpectedBlockLength`: `i2cdev` indexes its 34-byte buffer by the reported length and
panics, while the transport's lock is held, so every later call on the transport panics
too (a poisoned lock); open a new transport.

---

## Interface Boundaries

### Below: Linux `i2c-dev` kernel interface

`I2cDevTransport` is backed by `ioctl(fd, I2C_SLAVE, 0x54)` on open (to set the slave
address) and `ioctl(fd, I2C_SMBUS, ...)` per transaction, against an open `/dev/i2c-N`
file descriptor. The `i2c-dev` kernel module must be loaded; on most Linux desktop systems
it is either built in or loaded automatically. The calling process needs read/write
permission on the device node (typically via the `i2c` group).

The SCDC slave address (0x54) is fixed by HDMI 2.1. This crate does not
expose it as a parameter.

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

On `amdgpu`, `I2cTransactionError::kind` will always be `Unknown { errno: EIO }`.
The named variants (`AddressNack`, `Timeout`, etc.) are unreachable on this hardware. This
is a driver deficiency — the driver has the information internally and discards it — not a
kernel interface limitation. The richer variant set is retained because it is correct per
the kernel's documented vocabulary and meaningful on compliant adapters.

### Above: `hdmi-hal` traits

`I2cDevTransport` satisfies `ScdcTransport` as defined in `hdmi-hal`. `StubPhy<F>`
satisfies `FrlPhy` and `TmdsPhy`. Callers construct these types and pass them — by value or
by `&mut` reference — to any API that accepts the corresponding trait. Nothing in this crate
is specific to `culvert`, `plumbob` or `spiritlevel`; those crates accept any
`ScdcTransport`, `FrlPhy` and `TmdsPhy` respectively.

The typical end-to-end validation setup:

```rust
let path = connector_ddc_adapter("card0-HDMI-A-1")?;
let transport = I2cDevTransport::open(path)?;
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
maps `LinuxI2CError` values to `I2cTransactionError`. Specific `LinuxI2CError::Errno`
values are constructed directly and the output variant is asserted. This covers every
named `I2cErrorKind` variant and the `Unknown { errno }` fallback without kernel
involvement.

**`read_block`** is tested against an in-memory device behind the crate-private
`SmbusDevice` seam, which records each block read it is asked for: consecutive registers in
one ioctl, the 32-byte maximum in one, an empty buffer with none, a 33-byte buffer refused
with none, a result of another length an error with the buffer unchanged, a read from 0xF0
ending at 0xFF as on `i2c-stub`, `EOPNOTSUPP` as `Unsupported` with no fallback, and ioctl
errors mapped as for `read`; and, opened bytewise, one byte-data read per register, no
length limit, wrapping past 0xFF, and stopping at the first error.

No kernel involvement. Runs in CI on any Linux host.

### Tier 2: `i2c-stub` integration tests

**`I2cDevTransport`** is tested against the Linux `i2c-stub` kernel module, which
registers a fake I²C device at a specified address and exposes it as a real `/dev/i2c-N`
device node. Transactions go through the full `I2C_SMBUS` ioctl path; the stub responds
to SMBus byte-data reads and I2C block reads and records writes.

This tier validates:
- correct SMBus byte-data read (atomic combined write-then-read in a single ioctl),
- correct SMBus byte-data write,
- correct SMBus I2C block read (`read_block`): 9 bytes (the CED block), 2 bytes and the
  32-byte maximum, each in one ioctl, and a 33-byte read refused,
- slave address 0x54 is set at device open time,
- an error is produced when a transaction, byte or block read, targets an address not
  registered with the stub.

These tests require `i2c-stub` to be loaded (`modprobe i2c-stub`) and the calling process
to have write permission on the resulting device node. They are gated behind a feature flag
(`--features integration`) and are not run in standard CI. They are the authoritative test
for transport correctness and must be run before any release.

**Error conditions not covered:** bus errors, clock stretch timeouts, and arbitration loss
cannot be simulated via `i2c-stub`. `i2c-stub` always ACKs registered addresses; the only
error condition it can produce is a transfer to an unregistered address, which it reports
as `ENODEV` (`AdapterGone`) rather than the `ENXIO` (`AddressNack`) of a real adapter's
NACK. The mapping from
`LinuxI2CError` to these variants is covered by the Tier 1 unit tests; exercising them
end-to-end requires real misbehaving hardware.

`i2cdev`'s `MockI2CDevice` is intentionally not used for transport tests. The mock does
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
