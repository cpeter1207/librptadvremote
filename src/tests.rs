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
        poll_interval_ms: 0,
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

#[test]
fn remote_external_signaling_does_not_require_a_programmable_tone() {
    let mut config = configuration();
    config.memories[0].state.encode = Signaling {
        source: SignalSource::External,
        enabled: true,
        ..Signaling::default()
    };
    assert!(RemoteBase::new(config.clone(), capabilities()).is_ok());
    config.memories[0].state.encode.source = SignalSource::Dsp;
    assert_eq!(
        RemoteBase::new(config, capabilities()).err(),
        Some(Error::Signaling)
    );
}

#[test]
fn remote_signaling_validates_actual_dcs_bits_and_exclusive_tones() {
    for (tone, code, inverted) in [(0, 512, false), (1148, 0o23, false), (1148, 0, true)] {
        let mut config = configuration();
        config.memories[0].state.encode = Signaling {
            source: SignalSource::Dsp,
            enabled: true,
            ctcss_tenths_hz: tone,
            dcs_code: code,
            inverted,
        };
        assert_eq!(
            RemoteBase::new(config, capabilities()).err(),
            Some(Error::Signaling)
        );
    }
}

#[test]
fn remote_partial_readback_does_not_invent_missing_observations() {
    let mut caps = capabilities();
    caps.readback_fields = 1 | 4;
    let mut owner = RemoteBase::new(configuration(), caps).unwrap();
    let mut backend = Backend {
        read_override: Some(OperatingState {
            receive_hz: 146_520_000,
            ..OperatingState::default()
        }),
        ..Backend::default()
    };
    owner.advance(0, Activity::default(), &mut backend).unwrap();
    assert_eq!(owner.snapshot().observed_fields, 5);
    assert!(!owner.snapshot().verified);
    assert_eq!(
        owner.snapshot().applied.as_ref().unwrap().power_mw,
        Some(5_000)
    );
    backend.read_override.as_mut().unwrap().receive_hz = 149_000_000;
    assert_eq!(
        owner.advance(500, Activity::default(), &mut backend),
        Err(Error::ReceiveFrequency)
    );
    assert!(!owner.snapshot().receive_permitted);
}

#[test]
fn remote_poll_uses_configured_monotonic_interval() {
    let mut config = configuration();
    config.poll_interval_ms = 200;
    let mut caps = capabilities();
    caps.readback_fields = 8191;
    let mut owner = RemoteBase::new(config, caps).unwrap();
    let mut backend = Backend::default();
    owner.advance(0, Activity::default(), &mut backend).unwrap();
    backend.events.clear();
    owner
        .advance(199, Activity::default(), &mut backend)
        .unwrap();
    assert!(!backend.events.contains(&"read"));
    owner
        .advance(200, Activity::default(), &mut backend)
        .unwrap();
    assert!(backend.events.contains(&"read"));
}

#[derive(Default)]
struct Backend {
    state: Option<OperatingState>,
    writes: Vec<OperatingState>,
    failures: std::collections::VecDeque<i32>,
    keys: Vec<bool>,
    key_failures: std::collections::VecDeque<i32>,
    read_override: Option<OperatingState>,
    events: Vec<&'static str>,
}

impl RigBackend for Backend {
    fn capabilities(&self) -> Capabilities {
        capabilities()
    }
    fn read_state(&mut self) -> Result<OperatingState, i32> {
        self.events.push("read");
        self.read_override
            .clone()
            .or_else(|| self.state.clone())
            .ok_or(-100)
    }
    fn apply_step(&mut self, state: &OperatingState) -> Result<(), i32> {
        self.writes.push(state.clone());
        self.state = Some(state.clone());
        match self.failures.pop_front() {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
    fn set_ptt(&mut self, keyed: bool) -> Result<(), i32> {
        self.events.push(if keyed { "key" } else { "unkey" });
        self.keys.push(keyed);
        self.key_failures.pop_front().map_or(Ok(()), Err)
    }
}

fn running_radio() -> (RemoteBase, Backend) {
    let mut radio = RemoteBase::new(configuration(), capabilities()).unwrap();
    let mut backend = Backend::default();
    radio.advance(0, Activity::default(), &mut backend).unwrap();
    assert!(radio.take_completion().is_none());
    assert!(!radio.snapshot().transmit_inhibited);
    (radio, backend)
}

#[test]
fn remote_tune_waits_for_ptt_but_can_preempt_open_receive() {
    let (mut radio, mut backend) = running_radio();
    let id = radio
        .request(
            RigRequest::Set(OperatingPatch {
                receive_hz: Some(146_560_000),
                transmit_hz: Some(Some(146_560_000)),
                ..OperatingPatch::default()
            }),
            1,
        )
        .unwrap();
    let writes = backend.writes.len();
    radio
        .advance(
            1,
            Activity {
                physical_ptt: true,
                transmit_requested: true,
                ..Activity::default()
            },
            &mut backend,
        )
        .unwrap();
    assert_eq!(backend.writes.len(), writes);
    radio
        .advance(
            2,
            Activity {
                local_receiving: true,
                command_source_active: true,
                ..Activity::default()
            },
            &mut backend,
        )
        .unwrap();
    assert_eq!(backend.writes.len(), writes);
    radio
        .advance(
            3,
            Activity {
                local_receiving: true,
                ..Activity::default()
            },
            &mut backend,
        )
        .unwrap();
    assert_eq!(backend.writes.last().unwrap().receive_hz, 146_560_000);
    let completion = radio.take_completion().unwrap();
    assert_eq!(completion.id, id);
    assert_eq!(completion.result, Ok(()));
    assert!(!radio.snapshot().verified);
}

#[test]
fn remote_partial_apply_retains_inhibit_and_first_cause() {
    let (mut radio, mut backend) = running_radio();
    radio
        .request(
            RigRequest::Set(OperatingPatch {
                receive_hz: Some(146_560_000),
                ..OperatingPatch::default()
            }),
            1,
        )
        .unwrap();
    backend.failures.extend([-41, -42]);
    assert_eq!(
        radio.advance(1, Activity::default(), &mut backend),
        Err(Error::Backend)
    );
    assert!(radio.snapshot().transmit_inhibited);
    assert_eq!(radio.snapshot().failure.unwrap().code, -41);
    assert_eq!(radio.take_completion().unwrap().result, Err(Error::Backend));
    assert_eq!(backend.writes.last().unwrap().receive_hz, 146_520_000);
}

#[test]
fn remote_retired_generation_cannot_apply() {
    let (mut radio, mut backend) = running_radio();
    radio.request(RigRequest::Recall(1), 1).unwrap();
    let writes = backend.writes.len();
    radio.retire();
    assert_eq!(
        radio.advance(2, Activity::default(), &mut backend),
        Err(Error::Stale)
    );
    assert_eq!(backend.writes.len(), writes);
    assert!(radio.snapshot().transmit_inhibited);
    assert_eq!(backend.keys.last(), Some(&false));
    assert_eq!(radio.request(RigRequest::Recall(1), 3), Err(Error::Stale));
}

#[test]
fn remote_observed_unsafe_front_panel_state_closes_receive_and_transmit() {
    let mut caps = capabilities();
    caps.readback_fields = 8191;
    let mut radio = RemoteBase::new(configuration(), caps).unwrap();
    let mut backend = Backend::default();
    radio.advance(0, Activity::default(), &mut backend).unwrap();
    assert!(radio.snapshot().verified);
    backend.state.as_mut().unwrap().receive_hz = 149_000_000;
    assert_eq!(
        radio.advance(1_000, Activity::default(), &mut backend),
        Err(Error::ReceiveFrequency)
    );
    assert!(!radio.snapshot().receive_permitted);
    assert!(radio.snapshot().transmit_inhibited);
}

#[test]
fn remote_readback_mismatch_does_not_report_success() {
    let mut caps = capabilities();
    caps.readback_fields = 8191;
    let config = configuration();
    let mut different = config.memories[0].state.clone();
    different.receive_hz = 146_560_000;
    let mut backend = Backend {
        read_override: Some(different),
        ..Backend::default()
    };
    let mut radio = RemoteBase::new(config, caps).unwrap();
    assert_eq!(
        radio.advance(0, Activity::default(), &mut backend),
        Err(Error::ReadbackMismatch)
    );
    assert!(radio.snapshot().transmit_inhibited);
    assert!(!radio.snapshot().verified);
}

#[test]
fn remote_key_failure_attempts_unkey_and_keeps_first_cause() {
    let (mut radio, mut backend) = running_radio();
    backend.key_failures.extend([-41, -42]);
    assert_eq!(
        radio.advance(
            1,
            Activity {
                transmit_requested: true,
                ..Activity::default()
            },
            &mut backend
        ),
        Err(Error::Backend)
    );
    assert_eq!(&backend.keys[backend.keys.len() - 2..], &[true, false]);
    assert_eq!(radio.snapshot().failure.unwrap().code, -41);
    assert!(radio.snapshot().transmit_inhibited);
    assert_eq!(radio.snapshot().ptt_applied, None);
}

#[test]
fn remote_bounded_completion_is_retained_until_drained() {
    let (mut radio, mut backend) = running_radio();
    let id = radio.request(RigRequest::Recall(1), 1).unwrap();
    assert_eq!(radio.request(RigRequest::Recall(1), 1), Err(Error::Busy));
    radio.advance(1, Activity::default(), &mut backend).unwrap();
    assert_eq!(radio.request(RigRequest::Recall(1), 2), Err(Error::Busy));
    assert_eq!(radio.take_completion().unwrap().id, id);
    assert!(radio.request(RigRequest::Recall(1), 2).is_ok());
}

#[test]
fn remote_unkey_precedes_periodic_readback() {
    let mut caps = capabilities();
    caps.readback_fields = 8191;
    let mut radio = RemoteBase::new(configuration(), caps).unwrap();
    let mut backend = Backend::default();
    radio.advance(0, Activity::default(), &mut backend).unwrap();
    radio
        .advance(
            1,
            Activity {
                transmit_requested: true,
                ..Activity::default()
            },
            &mut backend,
        )
        .unwrap();
    backend.events.clear();
    radio
        .advance(1_000, Activity::default(), &mut backend)
        .unwrap();
    assert_eq!(backend.events.first(), Some(&"unkey"));
    assert!(backend.events.contains(&"read"));
}

#[test]
fn remote_front_panel_tuning_is_preserved_when_changing_another_setting() {
    let mut caps = capabilities();
    caps.readback_fields = 8191;
    let mut radio = RemoteBase::new(configuration(), caps).unwrap();
    let mut backend = Backend::default();
    radio.advance(0, Activity::default(), &mut backend).unwrap();
    backend.state.as_mut().unwrap().receive_hz = 146_560_000;
    radio
        .advance(1_000, Activity::default(), &mut backend)
        .unwrap();
    radio
        .request(
            RigRequest::Set(OperatingPatch {
                power_mw: Some(6_000),
                ..OperatingPatch::default()
            }),
            1_001,
        )
        .unwrap();
    radio
        .advance(1_001, Activity::default(), &mut backend)
        .unwrap();
    assert_eq!(backend.writes.last().unwrap().receive_hz, 146_560_000);
    assert_eq!(backend.writes.last().unwrap().power_mw, Some(6_000));
}

#[test]
fn remote_critical_inhibit_bypasses_a_pending_tune() {
    let (mut radio, mut backend) = running_radio();
    let tune = radio.request(RigRequest::Recall(1), 1).unwrap();
    let inhibit = radio.request(RigRequest::Inhibit(true), 2).unwrap();
    assert!(radio.snapshot().transmit_inhibited);
    radio
        .advance(
            2,
            Activity {
                physical_ptt: true,
                transmit_requested: true,
                ..Activity::default()
            },
            &mut backend,
        )
        .unwrap();
    assert_eq!(backend.keys.last(), Some(&false));
    assert_eq!(radio.take_completion().unwrap().id, inhibit);
    radio.advance(3, Activity::default(), &mut backend).unwrap();
    assert_eq!(radio.take_completion().unwrap().id, tune);
    assert!(radio.snapshot().transmit_inhibited);
}
