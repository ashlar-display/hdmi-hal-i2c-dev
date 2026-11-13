//! [`FrlPhy`] and [`TmdsPhy`] stub implementation for userspace validation.
//!
//! [`FrlPhy`]: hdmi_hal::phy::FrlPhy
//! [`TmdsPhy`]: hdmi_hal::phy::TmdsPhy

use core::convert::Infallible;

use display_types::cea861::hdmi_forum::HdmiForumFrl;
use hdmi_hal::phy::{
    EqParams, FrlOutput, FrlPhy, LanePatterns, TmdsPhy, TmdsPhyCapabilities, TmdsPhyConfig,
};

/// A structured description of a single [`FrlPhy`] or [`TmdsPhy`] method invocation.
///
/// Used as the callback argument for [`StubPhy`]. Match on variants to
/// observe or assert specific PHY calls in tests and diagnostic tools.
///
/// This enum is `#[non_exhaustive]`: if the PHY traits gain new methods,
/// `PhyCall` gains new variants. Downstream match arms should include a
/// wildcard (`_`) so they do not need to be updated for methods they do not
/// care about.
///
/// [`FrlPhy`]: hdmi_hal::phy::FrlPhy
/// [`TmdsPhy`]: hdmi_hal::phy::TmdsPhy
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PhyCall {
    /// [`FrlPhy::set_frl_rate`] was called with the given rate.
    ///
    /// [`FrlPhy::set_frl_rate`]: hdmi_hal::phy::FrlPhy::set_frl_rate
    SetFrlRate(HdmiForumFrl),
    /// [`FrlPhy::send_ltp`] was called with the given per-lane pattern set.
    ///
    /// [`FrlPhy::send_ltp`]: hdmi_hal::phy::FrlPhy::send_ltp
    SendLtp(LanePatterns),
    /// [`FrlPhy::set_frl_output`] was called with the given output mode.
    ///
    /// [`FrlPhy::set_frl_output`]: hdmi_hal::phy::FrlPhy::set_frl_output
    SetFrlOutput(FrlOutput),
    /// [`FrlPhy::adjust_equalization`] was called with the given parameters.
    ///
    /// [`FrlPhy::adjust_equalization`]: hdmi_hal::phy::FrlPhy::adjust_equalization
    AdjustEqualization(EqParams),
    /// [`TmdsPhy::set_scrambling`] was called with the given flag.
    ///
    /// [`TmdsPhy::set_scrambling`]: hdmi_hal::phy::TmdsPhy::set_scrambling
    SetScrambling(bool),
    /// [`TmdsPhy::configure`] was called with the given configuration.
    ///
    /// [`TmdsPhy::configure`]: hdmi_hal::phy::TmdsPhy::configure
    Configure(TmdsPhyConfig),
    /// [`TmdsPhy::start_output`] was called.
    ///
    /// [`TmdsPhy::start_output`]: hdmi_hal::phy::TmdsPhy::start_output
    StartOutput,
    /// [`TmdsPhy::stop_output`] was called.
    ///
    /// [`TmdsPhy::stop_output`]: hdmi_hal::phy::TmdsPhy::stop_output
    StopOutput,
}

/// An [`FrlPhy`] and [`TmdsPhy`] stub that forwards each call as a [`PhyCall`] event to a
/// caller-supplied callback and unconditionally returns `Ok(())`.
///
/// `type Error = Infallible` is guaranteed by the design: the callback
/// returns `()`, so `StubPhy` cannot fail regardless of what the callback
/// does. It keeps no state: it does not refuse `start_output` before
/// `configure`, as a real PHY does.
///
/// Its [`TmdsPhy::capabilities`] report is what [`StubPhy::new`] gives
/// ([`StubPhy::DEFAULT_CAPABILITIES`]) or what
/// [`with_capabilities`](StubPhy::with_capabilities) sets.
///
/// # Examples
///
/// No-op — discard all PHY calls:
///
/// ```
/// use hdmi_hal_i2c_dev::phy::StubPhy;
/// let _phy = StubPhy::new(|_| {});
/// ```
///
/// Logging — print each call to stderr:
///
/// ```
/// use hdmi_hal_i2c_dev::phy::StubPhy;
/// let _phy = StubPhy::new(|call| eprintln!("{call:?}"));
/// ```
///
/// [`FrlPhy`]: hdmi_hal::phy::FrlPhy
/// [`TmdsPhy`]: hdmi_hal::phy::TmdsPhy
/// [`TmdsPhy::capabilities`]: hdmi_hal::phy::TmdsPhy::capabilities
pub struct StubPhy<F> {
    on_call: F,
    capabilities: TmdsPhyCapabilities,
}

impl<F> StubPhy<F> {
    /// What a stub reports unless told otherwise: a TMDS PHY that can do everything TMDS
    /// allows, so that a caller choosing a link from it is limited by the sink alone. 600 MHz
    /// (HDMI 2.0's TMDS maximum), a scrambler, and scrambling at or below 340 Mcsc.
    pub const DEFAULT_CAPABILITIES: TmdsPhyCapabilities = TmdsPhyCapabilities::new(600_000)
        .with_scrambling(true)
        .with_low_rate_scrambling(true);
}

impl<F: FnMut(PhyCall)> StubPhy<F> {
    /// Construct a `StubPhy` that forwards each PHY call to `on_call`, with
    /// [`DEFAULT_CAPABILITIES`](Self::DEFAULT_CAPABILITIES).
    pub fn new(on_call: F) -> Self {
        Self {
            on_call,
            capabilities: Self::DEFAULT_CAPABILITIES,
        }
    }

    /// The same stub, reporting `capabilities` instead: a PHY without a scrambler, for
    /// example, or one with a lower ceiling.
    pub fn with_capabilities(mut self, capabilities: TmdsPhyCapabilities) -> Self {
        self.capabilities = capabilities;
        self
    }
}

impl<F: FnMut(PhyCall)> FrlPhy for StubPhy<F> {
    type Error = Infallible;

    fn set_frl_rate(&mut self, rate: HdmiForumFrl) -> Result<(), Infallible> {
        (self.on_call)(PhyCall::SetFrlRate(rate));
        Ok(())
    }

    fn send_ltp(&mut self, patterns: LanePatterns) -> Result<(), Infallible> {
        (self.on_call)(PhyCall::SendLtp(patterns));
        Ok(())
    }

    fn set_frl_output(&mut self, output: FrlOutput) -> Result<(), Infallible> {
        (self.on_call)(PhyCall::SetFrlOutput(output));
        Ok(())
    }

    fn adjust_equalization(&mut self, params: EqParams) -> Result<(), Infallible> {
        (self.on_call)(PhyCall::AdjustEqualization(params));
        Ok(())
    }
}

impl<F: FnMut(PhyCall)> TmdsPhy for StubPhy<F> {
    type Error = Infallible;

    fn capabilities(&self) -> TmdsPhyCapabilities {
        self.capabilities
    }

    fn configure(&mut self, config: TmdsPhyConfig) -> Result<(), Infallible> {
        (self.on_call)(PhyCall::Configure(config));
        Ok(())
    }

    fn set_scrambling(&mut self, enabled: bool) -> Result<(), Infallible> {
        (self.on_call)(PhyCall::SetScrambling(enabled));
        Ok(())
    }

    fn start_output(&mut self) -> Result<(), Infallible> {
        (self.on_call)(PhyCall::StartOutput);
        Ok(())
    }

    fn stop_output(&mut self) -> Result<(), Infallible> {
        (self.on_call)(PhyCall::StopOutput);
        Ok(())
    }
}

#[cfg(test)]
#[path = "phy_tests.rs"]
mod tests;
