use super::*;

fn configuration() -> RemoteBaseConfig {
    RemoteBaseConfig {
        limits: Limits {
            receive: vec![FrequencyRange::new(144_000_000, 148_000_000)],
            transmit: vec![FrequencyRange::new(144_000_000, 148_000_000)],
            receive_only: vec![FrequencyRange::new(145_000_000, 145_100_000)],
            modes: vec![Mode::Fm],
            maximum_power_mw: Some(10_000),
        },
        memories: vec![Memory {
            selector: 1,
            name: "local".into(),
            state: OperatingState {
                receive_hz: 146_520_000,
                transmit_hz: Some(146_520_000),
                mode: Mode::Fm,
                power_mw: Some(5_000),
                ..OperatingState::default()
            },
        }],
        scan_lists: vec![ScanList {
            selector: 1,
            name: "local channels".into(),
            members: vec![1],
            ..ScanList::default()
        }],
        startup_memory: 1,
    }
}

fn capabilities() -> Capabilities {
    Capabilities {
        modes: vec![Mode::Fm, Mode::Usb],
        receive: vec![FrequencyRange::new(100_000, 500_000_000)],
        transmit: vec![FrequencyRange::new(100_000, 500_000_000)],
        power_mw: true,
        encode_sources: vec![SignalSource::Dsp, SignalSource::External],
        decode_sources: vec![SignalSource::Dsp, SignalSource::External],
        ..Capabilities::default()
    }
}

#[test]
fn remote_base_rejects_unsafe_complete_state() {
    let configuration = configuration();
    let capabilities = capabilities();
    assert!(RemoteBase::new(configuration.clone(), capabilities.clone()).is_ok());

    for (receive_hz, transmit_hz, rit_hz, xit_hz, mode, power_mw) in [
        (143_000_000, 146_520_000, 0, 0, Mode::Fm, 5_000),
        (146_520_000, 149_000_000, 0, 0, Mode::Fm, 5_000),
        (147_999_999, 146_520_000, 2, 0, Mode::Fm, 5_000),
        (146_520_000, 147_999_999, 0, 2, Mode::Fm, 5_000),
        (146_520_000, 145_050_000, 0, 0, Mode::Fm, 5_000),
        (146_520_000, 146_520_000, 0, 0, Mode::Usb, 5_000),
        (146_520_000, 146_520_000, 0, 0, Mode::Fm, 10_001),
    ] {
        let mut invalid = configuration.clone();
        let state = &mut invalid.memories[0].state;
        state.receive_hz = receive_hz;
        state.transmit_hz = Some(transmit_hz);
        state.rit_hz = rit_hz;
        state.xit_hz = xit_hz;
        state.mode = mode;
        state.power_mw = Some(power_mw);
        assert!(RemoteBase::new(invalid, capabilities.clone()).is_err());
    }

    let mut unsupported = capabilities.clone();
    unsupported.power_mw = false;
    assert!(RemoteBase::new(configuration.clone(), unsupported).is_err());

    let mut duplicate = configuration.clone();
    duplicate.memories.push(duplicate.memories[0].clone());
    assert!(RemoteBase::new(duplicate, capabilities.clone()).is_err());

    let mut missing = configuration.clone();
    missing.scan_lists[0].members.push(2);
    assert!(RemoteBase::new(missing, capabilities.clone()).is_err());

    let mut unsupported_source = configuration;
    unsupported_source.memories[0].state.encode.source = SignalSource::Rig;
    unsupported_source.memories[0].state.encode.enabled = true;
    assert!(RemoteBase::new(unsupported_source, capabilities).is_err());
}
