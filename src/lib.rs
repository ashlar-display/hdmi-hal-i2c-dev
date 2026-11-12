//! Linux userspace [`hdmi-hal`] backend backed by `/dev/i2c-N` via the
//! [`i2c-dev`] kernel interface.
//!
//! This crate is the **development, validation, and diagnostic backend** for
//! the HDMI stack — not the production path. It lets you run the full FRL link
//! training stack ([`culvert`], [`plumbob`]) against a real HDMI sink from
//! userspace, without a kernel module.
//!
//! # Quick start
//!
//! ```no_run
//! use hdmi_hal_i2c_dev::{connector_ddc_adapter, I2cDevTransport, StubPhy};
//!
//! // Resolve the DDC adapter for a named connector and open the transport.
//! let path = connector_ddc_adapter("card0-HDMI-A-1")?;
//! let transport = I2cDevTransport::open(path)?;
//!
//! // StubPhy accepts all PHY calls and forwards them to a callback.
//! // Use `|_| {}` for a no-op, or a closure for logging or test assertions.
//! let phy = StubPhy::new(|call| eprintln!("{call:?}"));
//!
//! // Pass transport and phy to culvert / plumbob as you would any other backend.
//! # Ok::<(), hdmi_hal_i2c_dev::I2cDevError>(())
//! ```
//!
//! # Reading the EDID
//!
//! [`read_edid`] reads a sink's EDID over any `hdmi-hal` `DdcBus`, as piaf's read plan
//! directs; see the [`edid`] module.
//!
//! # Privileges
//!
//! The calling process needs read/write permission on the `/dev/i2c-N` device
//! node. On most Linux desktop systems this means membership in the `i2c`
//! group, or running as root.
//!
//! # `amdgpu` limitation
//!
//! The `amdgpu` DDC adapter collapses all I²C errors to `EIO`. On that
//! hardware, [`I2cTransactionError::kind`] is always
//! [`I2cErrorKind::Unknown`]`{ errno: EIO }` regardless of the actual bus
//! condition. See `doc/architecture.md` for details.
//!
//! [`hdmi-hal`]: https://crates.io/crates/hdmi-hal
//! [`i2c-dev`]: https://www.kernel.org/doc/Documentation/i2c/dev-interface
//! [`culvert`]: https://crates.io/crates/culvert
//! [`plumbob`]: https://crates.io/crates/plumbob

#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod discovery;
pub mod edid;
pub mod error;
pub mod phy;
pub mod transport;

pub use discovery::connector_ddc_adapter;
pub use edid::{EdidReadError, read_edid};
pub use error::{I2cDevError, I2cErrorKind, I2cTransactionError, MessagePhase};
pub use phy::{PhyCall, StubPhy};
pub use transport::I2cDevTransport;
