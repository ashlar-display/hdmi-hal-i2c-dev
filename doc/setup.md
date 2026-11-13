# Development Setup

**Requirements:** Rust 1.85+ (stable), Linux. Install Rust via [rustup](https://rustup.rs/).

## Clone and build

```sh
git clone https://github.com/ashlar-display/hdmi-hal-i2c-dev.git
cd hdmi-hal-i2c-dev
cargo build
```

## Running checks

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo clippy --locked --all-targets --features integration -- -D warnings
cargo rustdoc --locked --all-features -- -D warnings
```

The second clippy run lints the Tier 2 tests, which need `i2c-stub` to run.

## Checking the MSRV

The MSRV is `rust-version` in `Cargo.toml`. CI checks it with
[`cargo-msrv`](https://github.com/foresterre/cargo-msrv), which runs the Tier 1 tests on
that toolchain:

```sh
cargo install cargo-msrv --locked
cargo msrv verify -- cargo test --locked
```

## Running Tier 1 tests

No hardware or special privileges required.

```sh
cargo test --locked
```

## Running Tier 2 integration tests

Tier 2 tests exercise `I2cDevTransport` against the Linux `i2c-stub` kernel module.
They require `i2c-stub` to be loaded and the calling user to have write permission on
the resulting device node.

### Step 1 — Load `i2c-stub` with the SCDC slave address

```sh
sudo modprobe i2c-stub chip_addr=0x54
```

This creates a new `/dev/i2c-N` adapter. Find `N`:

```sh
# Most recently created i2c adapter:
ls -lt /dev/i2c-* | head -3

# Or via sysfs — look for the i2c-stub entry:
ls /sys/bus/i2c/drivers/i2c-stub/
```

### Step 2 — Grant device permission

```sh
sudo chmod 0666 /dev/i2c-N
# or: add yourself to the i2c group (requires re-login)
sudo usermod -aG i2c $USER
```

### Step 3 — Run the tests

```sh
I2C_STUB_ADAPTER=N cargo test --locked --features integration -- --nocapture
```

A test whose adapter variable is not set is skipped, not failed, and prints
`skipped: <variable> not set`. `cargo test` hides the output of passing tests, so run with
`--nocapture` to see which tests were skipped: a skipped test still reports `ok`.

In CI and for a release, set `I2C_STUB_REQUIRED=1`: a test whose adapter variable is unset
then fails instead of skipping, so a forgotten variable cannot pass for tier 2 coverage.
Each pass then runs only its own tests:

```sh
I2C_STUB_REQUIRED=1 I2C_STUB_ADAPTER=N cargo test --locked --features integration -- --skip slave_not_registered
```

### NACK tests (a second pass)

The two NACK tests need an adapter where nothing answers at 0x54, so that a read gets no
acknowledgement. `i2c-stub` registers a single adapter, whatever `chip_addr` lists (up to
ten addresses, all on that one bus), and a second `modprobe` of the loaded module changes
nothing. So run them in a second pass, with the stub reloaded at another address:

```sh
sudo modprobe -r i2c-stub
sudo modprobe i2c-stub chip_addr=0x55      # one stub bus, nothing at 0x54
ls -lt /dev/i2c-* | head -3                # the stub's adapter number, M, may differ from N
sudo chmod 0666 /dev/i2c-M
I2C_STUB_ADAPTER_NACK=M cargo test --locked --features integration slave_not_registered -- --nocapture
```

(In CI and for a release, with `I2C_STUB_REQUIRED=1` too.) The test name filter runs only
the two NACK tests. The byte-read one prints the error it got. Against `i2c-stub` the kind
is `AdapterGone`: for an
address with no chip the stub returns `ENODEV` (`i2c-stub.c`, `if (!chip) return
-ENODEV;`), where a real adapter returns `ENXIO`, which the crate maps to `AddressNack`
(`EIO`, `Unknown`, on `amdgpu`). The tests check only that the error is a `Transaction`
error.

Do not point `I2C_STUB_ADAPTER_NACK` at a real adapter instead: a DDC bus with an HDMI
sink attached does answer at 0x54 (that is SCDC), and other buses belong to hardware that
is better left alone.

### Unloading the stub

```sh
sudo modprobe -r i2c-stub
```

## Coverage

```sh
cargo llvm-cov --locked
```

To check against the baseline:

```sh
cargo llvm-cov --locked | grep -oP '\d+\.\d+(?=%)' | tail -1
cat .coverage-baseline
```
