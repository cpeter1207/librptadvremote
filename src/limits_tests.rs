use super::{capabilities, configuration};
use crate::*;

#[test]
fn limits_contract_rejects_empty_and_contradictory_safety_ranges() {
    assert_eq!(configuration().limits.validate(), Ok(()));
    type Case = (&'static str, fn(&mut Limits));
    let cases: &[Case] = &[
        ("no receive range", |value| value.receive.clear()),
        ("no mode", |value| value.modes.clear()),
        ("zero receive range", |value| value.receive[0].minimum = 0),
        ("reversed receive range", |value| {
            value.receive[0].minimum = 149_000_000
        }),
        ("zero transmit range", |value| value.transmit[0].minimum = 0),
        ("reversed receive-only range", |value| {
            value.receive_only[0].minimum = 146_000_000
        }),
    ];
    for (name, edit) in cases {
        let mut limits = configuration().limits;
        edit(&mut limits);
        assert_eq!(
            limits.validate(),
            Err(Error::InvalidConfiguration),
            "{name}"
        );
    }
    assert_eq!(
        format!("remote operation failed: {}", Error::Backend),
        "remote operation failed: Backend"
    );
}

#[test]
fn limits_contract_checks_mode_power_and_frequency_without_inventing_radio_support() {
    type Edit = fn(&mut Limits, &mut OperatingState, &mut Capabilities);
    let cases: &[(&str, Edit, Result<(), Error>)] = &[
        (
            "missing transmit coverage",
            |_, _, caps| caps.transmit.clear(),
            Err(Error::TransmitFrequency),
        ),
        (
            "missing receive coverage",
            |_, _, caps| caps.receive.clear(),
            Err(Error::ReceiveFrequency),
        ),
        (
            "disallowed mode",
            |limits, _, _| limits.modes.clear(),
            Err(Error::Mode),
        ),
        (
            "unavailable mode",
            |_, _, caps| caps.modes.clear(),
            Err(Error::Mode),
        ),
        (
            "maximum requires power control",
            |_, _, caps| caps.power_mw = false,
            Err(Error::Power),
        ),
        (
            "maximum requires actual power",
            |_, state, _| state.power_mw = None,
            Err(Error::Power),
        ),
        (
            "specified power without capability",
            |limits, _, caps| {
                limits.maximum_power_mw = None;
                caps.power_mw = false;
            },
            Err(Error::Power),
        ),
        (
            "unspecified power without ceiling",
            |limits, state, caps| {
                limits.maximum_power_mw = None;
                state.power_mw = None;
                caps.power_mw = false;
            },
            Ok(()),
        ),
        (
            "hardware minimum power",
            |_, _, caps| caps.transmit[0].minimum_power_mw = Some(6000),
            Err(Error::Power),
        ),
        (
            "power belongs to selected range",
            |_, _, caps| {
                let mut unrelated = caps.transmit[0].clone();
                unrelated.frequency = FrequencyRange::new(1_000_000, 2_000_000);
                caps.transmit[0].maximum_power_mw = Some(4000);
                caps.transmit.insert(0, unrelated);
            },
            Err(Error::Power),
        ),
        (
            "receive-only needs no transmitter",
            |_, state, caps| {
                state.transmit_hz = None;
                caps.transmit.clear();
                caps.power_mw = false;
            },
            Ok(()),
        ),
        (
            "offset arithmetic cannot underflow",
            |_, state, _| {
                state.transmit_hz = Some(0);
                state.offset_hz = -1;
            },
            Err(Error::TransmitFrequency),
        ),
        (
            "XIT arithmetic cannot underflow",
            |_, state, _| {
                state.transmit_hz = Some(0);
                state.xit_enabled = true;
                state.xit_hz = -1;
            },
            Err(Error::TransmitFrequency),
        ),
        (
            "RIT arithmetic cannot overflow",
            |_, state, _| {
                state.receive_hz = u64::MAX;
                state.rit_enabled = true;
                state.rit_hz = 1;
            },
            Err(Error::ReceiveFrequency),
        ),
    ];
    for (name, edit, expected) in cases {
        let config = configuration();
        let mut limits = config.limits;
        let mut state = config.memories[0].state.clone();
        let mut caps = capabilities();
        edit(&mut limits, &mut state, &mut caps);
        assert_eq!(limits.validate_state(&state, &caps), *expected, "{name}");
    }
}

#[test]
fn limits_contract_validates_independent_signal_sources_codes_and_polarity() {
    type Case = (&'static str, fn(&mut Signaling), Result<(), Error>);
    let cases: &[Case] = &[
        (
            "unsupported source",
            |s| s.source = SignalSource::Rig,
            Err(Error::Signaling),
        ),
        (
            "disabled programming",
            |s| s.control_source = Some(SignalSource::Disabled),
            Err(Error::Signaling),
        ),
        (
            "unsupported programming",
            |s| s.control_source = Some(SignalSource::Rig),
            Err(Error::Signaling),
        ),
        (
            "DCS exceeds nine bits",
            |s| {
                s.ctcss_tenths_hz = 0;
                s.dcs_code = 512;
            },
            Err(Error::Signaling),
        ),
        (
            "simultaneous tone and DCS",
            |s| s.dcs_code = 0o245,
            Err(Error::Signaling),
        ),
        (
            "CTCSS polarity",
            |s| s.inverted = true,
            Err(Error::Signaling),
        ),
        (
            "disabled source cannot be enabled",
            |s| s.source = SignalSource::Disabled,
            Err(Error::Signaling),
        ),
        (
            "DSP requires a tone or code",
            |s| s.ctcss_tenths_hz = 0,
            Err(Error::Signaling),
        ),
        (
            "external signaling can be fixed",
            |s| {
                s.source = SignalSource::External;
                s.ctcss_tenths_hz = 0;
            },
            Ok(()),
        ),
        (
            "inverted DSP DCS",
            |s| {
                s.ctcss_tenths_hz = 0;
                s.dcs_code = 0o245;
                s.inverted = true;
            },
            Ok(()),
        ),
        (
            "disabled DSP need not have a code",
            |s| {
                s.enabled = false;
                s.ctcss_tenths_hz = 0;
            },
            Ok(()),
        ),
        (
            "DSP decoder programming",
            |s| s.control_source = Some(SignalSource::Dsp),
            Ok(()),
        ),
    ];
    for (name, edit, expected) in cases {
        let config = configuration();
        let mut state = config.memories[0].state.clone();
        state.decode = Signaling {
            source: SignalSource::Dsp,
            enabled: true,
            ctcss_tenths_hz: 1148,
            ..Default::default()
        };
        let mut caps = capabilities();
        caps.decode_control_sources = vec![SignalSource::Dsp, SignalSource::External];
        edit(&mut state.decode);
        assert_eq!(
            config.limits.validate_state(&state, &caps),
            *expected,
            "{name}"
        );
    }
    let config = configuration();
    let mut state = config.memories[0].state.clone();
    state.decode = Signaling {
        source: SignalSource::Dsp,
        control_source: Some(SignalSource::Rig),
        enabled: true,
        dcs_code: 0o245,
        inverted: true,
        ..Default::default()
    };
    let mut caps = capabilities();
    caps.decode_control_sources = vec![SignalSource::Rig];
    assert_eq!(
        config.limits.validate_state(&state, &caps),
        Err(Error::Signaling)
    );
    state.decode.inverted = false;
    assert_eq!(config.limits.validate_state(&state, &caps), Ok(()));
}
