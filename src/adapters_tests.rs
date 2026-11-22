use hdmi_hal::ddc::{DISPLAYID, EddcTarget};

use super::*;

#[test]
fn general_adapters_are_general() {
    for name in [
        "i915 gmbus dpb",
        "gma500 gmbus panel",
        "mediatek-hdmi-ddc",
        "msm hdmi i2c",
    ] {
        assert_eq!(
            capabilities(name),
            Some(DdcCapabilities::general()),
            "{name}"
        );
    }
}

#[test]
fn reshaping_adapters_get_what_their_transfer_function_does() {
    let dw = capabilities("DesignWare HDMI").unwrap();
    assert_eq!(dw.write_max(), 2);
    assert_eq!(dw.write_read_max(), (1, u16::MAX));
    assert_eq!(dw.read_max(), 0);
    assert!(!dw.reaches(DDC_CI) && dw.reaches(SCDC));
    assert!(dw.splits_reads());
    assert_eq!(dw.max_segment(), Some(0));
    assert!(dw.reads_target(EddcTarget::DisplayId));

    let qp = capabilities("DesignWare HDMI QP").unwrap();
    assert!(!qp.reads_target(EddcTarget::DisplayId) && qp.reads_target(EddcTarget::Edid));

    for name in ["Inno HDMI", "RK3066 HDMI"] {
        let fifo = capabilities(name).unwrap();
        assert_eq!(fifo.write_max(), 0);
        assert_eq!(fifo.reachable_addresses(), 0);
        assert!(fifo.reads_target(EddcTarget::Edid) && !fifo.reads_target(EddcTarget::DisplayId));
        assert_eq!(fifo.max_segment(), Some(MAX_SEGMENT));
    }
    assert_eq!(
        capabilities("oaktrail_hdmi_i2c").unwrap().max_segment(),
        Some(0)
    );

    let mtk = capabilities("mediatek-hdmi-ddc-v2").unwrap();
    assert!(mtk.reaches(EDID) && mtk.reaches(SCDC) && !mtk.reaches(DISPLAYID));
    assert_eq!(mtk.read_max(), 0);
    assert_eq!(mtk.max_segment(), Some(0));
}

#[test]
fn limited_adapters_get_their_limits() {
    let sun4i = capabilities("sun4i_hdmi_i2c adapter").unwrap();
    assert_eq!(sun4i.write_max(), 1023);
    assert_eq!(sun4i.write_read_max(), (1023, 1023));
    assert_eq!(sun4i.max_segment(), Some(0));

    for name in ["AMDGPU DM i2c hw bus 3", "AMDGPU DM i2c OEM bus"] {
        let dc = capabilities(name).unwrap();
        assert_eq!(dc.write_max(), 143, "{name}");
        assert_eq!(dc.write_read_max(), (1, 128));
        assert_eq!(dc.read_max(), 143);
        assert_eq!(dc.segmented_piece(), Some(128));
    }
}

#[test]
fn an_unknown_adapter_is_not_in_the_table() {
    for name in [
        "",
        "i915",
        "Radeon i2c bit bus DVI",
        "DesignWare HDMI ",
        "i2c-stub",
    ] {
        assert_eq!(capabilities(name), None, "{name:?}");
    }
    assert_eq!(SMBUS_LEVEL.write_max(), 33);
    assert_eq!(SMBUS_LEVEL.write_read_max(), (1, 32));
    assert_eq!(SMBUS_LEVEL.read_max(), 1);
    assert_eq!(SMBUS_LEVEL.max_segment(), Some(0));
    assert_eq!(SMBUS_LEVEL.segmented_piece(), Some(32));
}

#[test]
fn eagain_is_a_timeout_where_the_driver_says_so() {
    for name in [
        "DesignWare HDMI",
        "DesignWare HDMI QP",
        "Inno HDMI",
        "RK3066 HDMI",
        "i915 gmbus dpc",
    ] {
        assert_eq!(known(name).unwrap().eagain, Eagain::Timeout, "{name}");
    }
    for name in [
        "gma500 gmbus lvds",
        "msm hdmi i2c",
        "AMDGPU DM i2c hw bus 0",
    ] {
        assert_eq!(known(name).unwrap().eagain, Eagain::Busy, "{name}");
    }
}
