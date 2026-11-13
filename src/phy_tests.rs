use display_types::cea861::hdmi_forum::HdmiForumFrl;
use hdmi_hal::phy::{LtpPattern, TxFfeLevel};

use super::*;

#[test]
fn new_constructs_stub() {
    // Verifies that `StubPhy::new` is callable and that the resulting stub
    // accepts a PHY call without panicking.
    let mut phy = StubPhy::new(|_| {});
    phy.set_scrambling(false).unwrap();
}

#[test]
fn set_frl_rate_forwards_correct_variant() {
    let mut received = None;
    {
        let mut phy = StubPhy::new(|call| received = Some(call));
        phy.set_frl_rate(HdmiForumFrl::Rate6Gbps4Lanes).unwrap();
    }
    assert_eq!(
        received,
        Some(PhyCall::SetFrlRate(HdmiForumFrl::Rate6Gbps4Lanes))
    );
}

#[test]
fn send_ltp_forwards_correct_variant() {
    let mut received = None;
    {
        let mut phy = StubPhy::new(|call| received = Some(call));
        phy.send_ltp(LanePatterns {
            lane0: Some(LtpPattern::Lfsr0),
            lane1: Some(LtpPattern::Lfsr1),
            lane2: Some(LtpPattern::Lfsr2),
            lane3: None,
        })
        .unwrap();
    }
    assert_eq!(
        received,
        Some(PhyCall::SendLtp(LanePatterns {
            lane0: Some(LtpPattern::Lfsr0),
            lane1: Some(LtpPattern::Lfsr1),
            lane2: Some(LtpPattern::Lfsr2),
            lane3: None,
        }))
    );
}

#[test]
fn set_frl_output_gap_only_forwards_correct_variant() {
    let mut received = None;
    {
        let mut phy = StubPhy::new(|call| received = Some(call));
        phy.set_frl_output(FrlOutput::GapOnly).unwrap();
    }
    assert!(matches!(
        received,
        Some(PhyCall::SetFrlOutput(FrlOutput::GapOnly))
    ));
}

#[test]
fn set_frl_output_active_forwards_correct_variant() {
    let mut received = None;
    {
        let mut phy = StubPhy::new(|call| received = Some(call));
        phy.set_frl_output(FrlOutput::Active).unwrap();
    }
    assert!(matches!(
        received,
        Some(PhyCall::SetFrlOutput(FrlOutput::Active))
    ));
}

#[test]
fn adjust_equalization_forwards_correct_variant() {
    let mut received = None;
    {
        let mut phy = StubPhy::new(|call| received = Some(call));
        phy.adjust_equalization(EqParams::new()).unwrap();
    }
    assert!(matches!(received, Some(PhyCall::AdjustEqualization(_))));
}

#[test]
fn adjust_equalization_forwards_per_lane_tx_ffe_levels() {
    let mut params = EqParams::new();
    params.lane1.tx_ffe_level = TxFfeLevel::MAX;
    let mut received = None;
    {
        let mut phy = StubPhy::new(|call| received = Some(call));
        phy.adjust_equalization(params).unwrap();
    }
    assert!(matches!(
        received,
        Some(PhyCall::AdjustEqualization(p))
            if p == params && p.lane0.tx_ffe_level.value() == 0 && p.lane1.tx_ffe_level.value() == 7
    ));
}

#[test]
fn set_scrambling_true_forwards_correct_variant() {
    let mut received = None;
    {
        let mut phy = StubPhy::new(|call| received = Some(call));
        phy.set_scrambling(true).unwrap();
    }
    assert!(matches!(received, Some(PhyCall::SetScrambling(true))));
}

#[test]
fn set_scrambling_false_forwards_correct_variant() {
    let mut received = None;
    {
        let mut phy = StubPhy::new(|call| received = Some(call));
        phy.set_scrambling(false).unwrap();
    }
    assert!(matches!(received, Some(PhyCall::SetScrambling(false))));
}

#[test]
fn callback_is_invoked_once_per_call() {
    let mut count = 0usize;
    {
        let mut phy = StubPhy::new(|_| count += 1);
        phy.set_frl_rate(HdmiForumFrl::NotSupported).unwrap();
        phy.send_ltp(LanePatterns::default()).unwrap();
        phy.set_frl_output(FrlOutput::GapOnly).unwrap();
        phy.adjust_equalization(EqParams::new()).unwrap();
        phy.set_scrambling(true).unwrap();
    }
    assert_eq!(count, 5);
}

#[test]
fn noop_stub_does_not_panic() {
    let mut phy = StubPhy::new(|_| {});
    phy.set_frl_rate(HdmiForumFrl::Rate3Gbps3Lanes).unwrap();
    phy.send_ltp(LanePatterns::default()).unwrap();
    phy.set_frl_output(FrlOutput::Active).unwrap();
    phy.adjust_equalization(EqParams::new()).unwrap();
    phy.set_scrambling(false).unwrap();
}

#[test]
fn recorded_calls_compare_as_a_sequence() {
    // `PhyCall` is `Copy` and `Eq`: a test can record calls and compare the whole log.
    let mut calls = std::vec::Vec::new();
    {
        let mut phy = StubPhy::new(|call| calls.push(call));
        phy.set_frl_rate(HdmiForumFrl::Rate12Gbps4Lanes).unwrap();
        phy.set_frl_output(FrlOutput::GapOnly).unwrap();
    }
    assert_eq!(
        calls,
        [
            PhyCall::SetFrlRate(HdmiForumFrl::Rate12Gbps4Lanes),
            PhyCall::SetFrlOutput(FrlOutput::GapOnly),
        ]
    );
}

#[test]
fn tmds_calls_forward_their_variants() {
    use hdmi_hal::phy::{TmdsClockRatio, TmdsPhyConfig, TmdsVideo};

    let config = TmdsPhyConfig::new(
        594_000,
        TmdsClockRatio::Div40,
        TmdsVideo::new(
            594_000,
            hdmi_hal::ColorBitDepth::Depth8,
            hdmi_hal::ColorFormat::Rgb444,
        ),
    );
    let mut calls = Vec::new();
    {
        let mut phy = StubPhy::new(|call| calls.push(call));
        phy.stop_output().unwrap();
        phy.configure(config).unwrap();
        phy.set_scrambling(true).unwrap();
        phy.start_output().unwrap();
    }
    assert_eq!(
        calls,
        [
            PhyCall::StopOutput,
            PhyCall::Configure(config),
            PhyCall::SetScrambling(true),
            PhyCall::StartOutput,
        ]
    );
}

#[test]
fn capabilities_are_the_defaults_or_what_the_caller_sets() {
    let phy = StubPhy::new(|_| {});
    assert_eq!(
        phy.capabilities(),
        StubPhy::<fn(PhyCall)>::DEFAULT_CAPABILITIES
    );
    assert!(phy.capabilities().scrambles_at_low_rate());
    let plain = hdmi_hal::phy::TmdsPhyCapabilities::new(340_000);
    let phy = StubPhy::new(|_| {}).with_capabilities(plain);
    assert_eq!(phy.capabilities(), plain);
    assert!(!phy.capabilities().scrambling);
}
