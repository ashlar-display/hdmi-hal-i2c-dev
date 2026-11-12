# Testing Strategy

Testing is split into two tiers based on what each tier validates and what it requires.

## Tier 1 — No hardware

Runs with `cargo test --locked`. No kernel module, no device node, no special
privileges. Runs in standard CI on any Linux host.

### `connector_ddc_adapter` (in `src/discovery.rs`)

Tested against a synthetic sysfs tree constructed in a `tempfile` temporary directory.
Each test creates the expected symlink structure under a configurable root and calls the
internal `connector_ddc_adapter_in` function with that root substituted for
`/sys/class/drm`.

**Covered:**
- Successful resolution of a valid connector with a realistic multi-component symlink target
- `ConnectorNotFound` when the connector directory is absent
- `ConnectorHasNoDdcAdapter` when the connector directory exists but has no `ddc` symlink
- `DdcAdapterIndexUnparseable` when the symlink target's final component does not start
  with `i2c-`
- `DdcAdapterIndexUnparseable` when the suffix after `i2c-` is not all digits
- `DdcAdapterIndexUnparseable` when the suffix is empty (`i2c-` with nothing after it)
- Multi-digit adapter indices

### `I2cErrorKind` errno mapping (in `src/transport.rs`)

Unit tests for the `map_errno` function, which maps raw errno integers to `I2cErrorKind`
variants following `Documentation/i2c/fault-codes.rst`.

**Covered:**
- `ENXIO` → `AddressNack`
- `EAGAIN` → `ArbitrationLost`
- `ETIMEDOUT` → `ClockStretchTimeout`
- `EBUSY` → `BusBusy`
- `EPROTO` → `BusProtocolError`
- `ESHUTDOWN` → `AdapterGone`
- `ENODEV` → `AdapterGone`
- `EIO` → `Unknown { errno: EIO }` (the only errno produced by `amdgpu`)
- Unrecognised errno → `Unknown { errno }`
- Zero errno → `Unknown { errno: 0 }` (produced by the partial-completion path)

### `read_block` (in `src/transport_tests.rs`)

Unit tests against an in-memory device behind the crate-private `SmbusDevice` seam, which
records every block read it is asked for, so a test can count ioctls.

**Covered:**
- Consecutive registers read in one ioctl (9 bytes, the CED block)
- The 32-byte maximum read in one ioctl
- An empty buffer: `Ok`, no ioctl
- A 33-byte buffer: `BlockReadTooLong`, no ioctl, the buffer unchanged
- A result of another length than requested: `UnexpectedBlockLength`, the buffer unchanged
  (not padded)
- A 32-byte read from 0xF0: `UnexpectedBlockLength` with 16 returned, as on `i2c-stub`,
  which ends a block read at 0xFF (the mock does the same rather than wrapping)
- `EOPNOTSUPP` (no SMBus I2C block read): `Transaction` with `Unsupported`, and no byte
  reads after it
- An ioctl error: `Transaction`, with the kind mapped as for `read` (the phase is not
  checked: one ioctl does not say which message failed, and the crate reports `Write`)
- Opened bytewise (`open_bytewise`): one byte-data read per register and no block read,
  no length limit, wrapping past 0xFF, and stopping at the first error with the bytes
  before it in the buffer
- `Display` of both new errors, and no `source` for either (in `src/error_tests.rs`)

### `StubPhy` (in `src/phy_tests.rs`)

Unit tests for the PHY stub, which forwards every `FrlPhy` and `TmdsPhy` call to a
callback as a `PhyCall`.

**Covered:**
- Each `FrlPhy` method forwards the matching `PhyCall` variant with its arguments:
  `set_frl_rate`, `send_ltp`, `set_frl_output` (`GapOnly` and `Active`) and
  `adjust_equalization` (including per-lane TxFFE levels)
- Each `TmdsPhy` method does too: `stop_output`, `configure` (with its configuration),
  `set_scrambling` (on and off) and `start_output`, in the order called
- `capabilities` reports the defaults, or what `with_capabilities` set
- The callback runs once per call
- A stub with a no-op callback accepts calls
- Recorded calls compare as a whole sequence (`PhyCall` is `Copy` and `Eq`)

### `I2cDevBus` (in `src/bus_tests.rs`)

Unit tests for the `DdcBus` implementation, through the crate-private `I2cBus` seam: a mock
records each transfer's messages with their addresses, answers reads, and can fail with an
errno or execute fewer messages than sent.

**Covered:**
- Each operation is one transfer, with the addresses and lengths it should have
- `read_segmented` writes the segment pointer only past segment 0, and a split read is one
  transfer per piece at the right offsets
- Empty buffers make no transfer; a write of no bytes is refused
- Every capability limit (address, write, write-read and read lengths, E-DDC target and
  segment) and the contract's (segment past E-DDC's highest, a read past the segment, a
  message over 8192 bytes) is refused before any transfer, as `Unsupported`
- A failed transfer keeps the first message's address and the errno's class; a transfer the
  kernel cut short is an error
- A poisoned lock is recovered
- The real `LinuxI2CBus` path: opening a missing path fails; on a file that is no I²C
  device, the transfer reaches the ioctl and fails with `ENOTTY`
- `open` against a synthetic sysfs tree: a table adapter's capabilities and name, an
  adapter not in the table and a name that cannot be read (`SMBUS_LEVEL`), a path that is
  not `i2c-N`
- A table adapter's `EAGAIN` and `EINVAL` read as its driver means them; the caller's
  capabilities keep Linux's convention

### The `adapters` table (in `src/adapters_tests.rs`)

**Covered:**
- The general adapters, by name and name prefix
- Each reshaping and limited adapter's characteristic limits
- Names not in the table (including near misses), and `SMBUS_LEVEL`'s values
- Which adapters' `EAGAIN` is a timeout

### `read_edid` (in `src/edid_tests.rs`)

Unit tests for the EDID read driver, against a double of a sink's EDID store reached only
through `read_segmented`, which refuses what its capabilities rule out, can fail from a
block on, and records each call.

**Covered:**
- Every block read across segments, at the segment and offset the plan names, one block per
  call, and the result parsed by `piaf::parse_edid`
- A bus that reads segment 0 only gets two blocks, and the plan reports the rest as not read
- The caller's buffer and `ReadOptions::capacity` bound the read too; an empty buffer is
  `NoRoom`
- A bus error names its block, with one attempt per block; a bus without segmented reads
  fails at the base block
- A bad base block is refused; a bad extension block is read and reported
- The errors print and chain; an unsized (`dyn`) bus works

## Tier 2 — `i2c-stub` integration tests

Gated behind `--features integration`. Requires the `i2c-stub` kernel module and a
device node with appropriate permissions. See [`doc/setup.md`](setup.md) for setup
instructions. **Not run in standard CI.** Must be run before any release.

These tests exercise the full SMBus byte-data and I2C block-read (`I2C_SMBUS`) ioctl paths
through
`I2cDevTransport` and validate things that the Tier 1 tests cannot: that messages are
constructed correctly, that the slave address 0x54 is set, and that the ioctl round-trip
through the kernel produces correct results.

**Covered:**
- An unwritten register reads back as `0x00` (stub initialises all registers to zero)
- A write operation returns `Ok(())`
- A value written to a register can be read back (round-trip correctness)
- Multiple registers are independent
- A block read returns consecutive registers: 9 bytes (0x50–0x58, the CED block), 2 bytes,
  and the 32-byte maximum, each written first; `i2c-stub` supports the SMBus I2C block read
  by default (`I2C_FUNC_SMBUS_I2C_BLOCK`)
- A 33-byte block read is refused (`BlockReadTooLong`)
- A 32-byte block read from 0xF0 is `UnexpectedBlockLength` with 16 bytes returned:
  `i2c-stub` returns at most `256 - command` bytes
- A transport opened with `open_bytewise` reads a block a register at a time
- A transaction to an unregistered address, a byte read or a block read, produces a
  `Transaction` error (the kind is `AdapterGone` on `i2c-stub`, which returns `ENODEV` for
  an address with no chip; `AddressNack` on a compliant adapter, which returns `ENXIO`;
  `Unknown { errno: EIO }` on `amdgpu`). These two tests need the stub without a chip at
  0x54, so they run in a second pass ([`setup.md`](setup.md), "NACK tests")

A test whose adapter variable is unset is skipped and prints `skipped: <variable> not set`;
run with `--nocapture` to see skips, as a skipped test still reports `ok`. With
`I2C_STUB_REQUIRED=1`, as in CI and for a release, an unset variable fails the test
instead ([`setup.md`](setup.md), "Step 3").

The block-read tests each use registers no other test writes (0x50–0x58, 0x60–0x61,
0xA0–0xBF, 0xD0–0xD3, 0xF0–0xFF), as all tests share one stub.

**Intentionally not covered:**
- Bus errors, clock stretch timeouts, and arbitration loss — `i2c-stub` always ACKs
  registered addresses; these conditions require misbehaving real hardware. Their
  errno→`I2cErrorKind` mapping is validated by the Tier 1 unit tests.
- `connector_ddc_adapter` end-to-end against a real sysfs tree — the Tier 1 synthetic
  tests are sufficient; the sysfs parsing logic has no dependency on a live system.
