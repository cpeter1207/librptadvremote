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
    let coverage = HardwareRange {
        frequency: FrequencyRange::new(100_000, 500_000_000),
        modes: vec![Mode::Fm, Mode::Usb],
        minimum_power_mw: None,
        maximum_power_mw: None,
    };
    Capabilities {
        manual_tune: true,
        modes: vec![Mode::Fm, Mode::Usb],
        receive: vec![coverage.clone()],
        transmit: vec![coverage],
        power_mw: true,
        encode_sources: vec![SignalSource::Dsp, SignalSource::External],
        decode_sources: vec![SignalSource::Dsp, SignalSource::External],
        ..Capabilities::default()
    }
}

#[test]
fn rf_limits_distinguish_stored_incremental_offsets_from_enabled_shifts() {
    let config = configuration();
    let mut state = config.memories[0].state.clone();
    state.rit_hz = 8_000_000;
    state.rit_enabled = false;
    assert_eq!(
        config.limits.validate_state(&state, &capabilities()),
        Ok(())
    );
    state.rit_enabled = true;
    assert_eq!(
        config.limits.validate_state(&state, &capabilities()),
        Err(Error::ReceiveFrequency)
    );
    state.rit_enabled = false;
    state.xit_hz = 8_000_000;
    state.xit_enabled = false;
    assert_eq!(
        config.limits.validate_state(&state, &capabilities()),
        Ok(())
    );
    state.xit_enabled = true;
    assert_eq!(
        config.limits.validate_state(&state, &capabilities()),
        Err(Error::TransmitFrequency)
    );
}

#[test]
fn rf_limits_require_both_nominal_and_shifted_frequencies_to_be_permitted() {
    let config = configuration();
    let mut state = config.memories[0].state.clone();
    state.receive_hz = 143_900_000;
    state.rit_hz = 200_000;
    state.rit_enabled = true;
    assert_eq!(
        config.limits.validate_state(&state, &capabilities()),
        Err(Error::ReceiveFrequency)
    );
    state.receive_hz = 146_520_000;
    state.rit_enabled = false;
    state.transmit_hz = Some(143_900_000);
    state.xit_hz = 200_000;
    state.xit_enabled = true;
    assert_eq!(
        config.limits.validate_state(&state, &capabilities()),
        Err(Error::TransmitFrequency)
    );
}

#[test]
fn remote_periodic_readback_waits_a_full_interval_after_slow_io() {
    let mut caps = capabilities();
    caps.readback_fields = 65535;
    let mut owner = RemoteBase::new(configuration(), caps).unwrap();
    let mut backend = Backend::default();
    tick(&mut owner, &mut backend, 0, Activity::default()).unwrap();
    backend.read_delay_ms = 2_000;
    tick(&mut owner, &mut backend, 1_000, Activity::default()).unwrap();
    backend.events.clear();
    tick(&mut owner, &mut backend, 3_001, Activity::default()).unwrap();
    assert!(!backend.events.contains(&"read"));
    tick(&mut owner, &mut backend, 4_000, Activity::default()).unwrap();
    assert!(backend.events.contains(&"read"));
}

#[test]
fn remote_inactive_shared_offset_readback_is_not_a_tune_failure() {
    let mut caps = capabilities();
    caps.readback_fields = 65535;
    let config = configuration();
    let mut observed = config.memories[0].state.clone();
    observed.rit_hz = 125;
    observed.xit_hz = 125;
    let mut backend = Backend {
        read_override: Some(observed.clone()),
        ..Backend::default()
    };
    let mut owner = RemoteBase::new(config, caps).unwrap();
    owner.advance(0, Activity::default(), &mut backend).unwrap();
    assert!(owner.snapshot().verified);
    assert_eq!(owner.snapshot().observed.unwrap(), observed);
}

#[test]
fn remote_decode_control_does_not_replace_external_qualification() {
    let mut config = configuration();
    config.memories[0].state.decode = Signaling {
        source: SignalSource::External,
        control_source: Some(SignalSource::Rig),
        enabled: true,
        ctcss_tenths_hz: 1148,
        ..Signaling::default()
    };
    let mut caps = capabilities();
    assert_eq!(
        RemoteBase::new(config.clone(), caps.clone()).err(),
        Some(Error::Signaling)
    );
    caps.decode_control_sources.push(SignalSource::Rig);
    let rig = RemoteBase::new(config.clone(), caps.clone()).unwrap();
    assert_eq!(
        rig.snapshot().requested.decode.source,
        SignalSource::External
    );
    config.memories[0].state.decode.ctcss_tenths_hz = 0;
    assert_eq!(
        RemoteBase::new(config.clone(), caps.clone()).err(),
        Some(Error::Signaling)
    );
    config.memories[0].state.decode.ctcss_tenths_hz = 1148;
    config.memories[0].state.encode.control_source = Some(SignalSource::Rig);
    assert_eq!(RemoteBase::new(config, caps).err(), Some(Error::Signaling));
}

#[test]
fn remote_split_selection_is_independent_of_repeater_shift_and_vfo_frequency() {
    let mut owner = RemoteBase::new(configuration(), capabilities()).unwrap();
    let mut backend = Backend::default();
    tick(&mut owner, &mut backend, 0, Activity::default()).unwrap();
    let id = owner
        .request(
            RigRequest::Set(OperatingPatch {
                split: Some(true),
                ..OperatingPatch::default()
            }),
            1,
        )
        .unwrap();
    tick(&mut owner, &mut backend, 1, Activity::default()).unwrap();
    assert_eq!(owner.take_completion().unwrap().id, id);
    let applied = owner.snapshot().applied.unwrap();
    assert!(applied.split);
    assert_eq!(applied.transmit_hz, Some(applied.receive_hz));
    assert_eq!(applied.offset_hz, 0);
    owner
        .request(
            RigRequest::Set(OperatingPatch {
                offset_hz: Some(-600000),
                split: Some(false),
                ..OperatingPatch::default()
            }),
            2,
        )
        .unwrap();
    tick(&mut owner, &mut backend, 2, Activity::default()).unwrap();
    let applied = owner.snapshot().applied.unwrap();
    assert!(!applied.split);
    assert_eq!(applied.offset_hz, -600000);
}

#[test]
fn remote_hardware_coverage_keeps_mode_and_power_on_each_range() {
    let mut config = configuration();
    config.limits.modes.push(Mode::Usb);
    config.memories[0].state.mode = Mode::Usb;
    let range = HardwareRange {
        frequency: FrequencyRange::new(144_000_000, 148_000_000),
        modes: vec![Mode::Fm],
        minimum_power_mw: Some(1000),
        maximum_power_mw: Some(3000),
    };
    let mut caps = capabilities();
    caps.receive = vec![range.clone()];
    caps.transmit = vec![range];
    assert_eq!(
        RemoteBase::new(config.clone(), caps.clone()).err(),
        Some(Error::ReceiveFrequency)
    );
    caps.receive[0].modes.push(Mode::Usb);
    assert_eq!(
        RemoteBase::new(config.clone(), caps.clone()).err(),
        Some(Error::TransmitFrequency)
    );
    caps.transmit[0].modes.push(Mode::Usb);
    assert_eq!(
        RemoteBase::new(config.clone(), caps.clone()).err(),
        Some(Error::Power)
    );
    caps.transmit[0].maximum_power_mw = Some(6000);
    assert!(RemoteBase::new(config.clone(), caps.clone()).is_ok());
    config.memories[0].state.power_mw = Some(500);
    assert_eq!(RemoteBase::new(config, caps).err(), Some(Error::Power));
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
        state.rit_enabled = rit_hz != 0;
        state.xit_enabled = xit_hz != 0;
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
            ..Signaling::default()
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
    caps.readback_fields = 65535;
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
    clock_ms: u64,
    apply_delay_ms: u64,
    read_delay_ms: u64,
    transmit_epoch: u64,
    overlap_next_apply: bool,
}

impl RigBackend for Backend {
    fn capabilities(&self) -> Capabilities {
        capabilities()
    }
    fn read_state(&mut self) -> Result<OperatingState, i32> {
        self.clock_ms += self.read_delay_ms;
        self.events.push("read");
        self.read_override
            .clone()
            .or_else(|| self.state.clone())
            .ok_or(-100)
    }
    fn apply_step(&mut self, state: &OperatingState) -> Result<(), i32> {
        self.clock_ms += self.apply_delay_ms;
        if self.overlap_next_apply {
            self.transmit_epoch += 1;
            self.overlap_next_apply = false;
        }
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
    fn set_tune(&mut self, _: bool) -> Result<(), i32> {
        Ok(())
    }

    fn monotonic_ms(&self) -> u64 {
        self.clock_ms
    }
    fn transmit_epoch(&self) -> u64 {
        self.transmit_epoch
    }
}

fn scanning() -> (RemoteBase, Backend) {
    let mut config = configuration();
    let mut other = config.memories[0].clone();
    other.selector = 2;
    other.name = "other".into();
    other.state.receive_hz = 146_940_000;
    other.state.transmit_hz = Some(146_940_000);
    config.memories.push(other);
    config.scan_lists[0].members = vec![2, 1];
    let mut caps = capabilities();
    caps.scan_modes = vec![Mode::Fm];
    let mut owner = RemoteBase::new(config, caps).unwrap();
    let mut backend = Backend::default();
    owner.advance(0, Activity::default(), &mut backend).unwrap();
    (owner, backend)
}

fn tick(
    owner: &mut RemoteBase,
    backend: &mut Backend,
    now: u64,
    activity: Activity,
) -> Result<(), Error> {
    backend.clock_ms = now;
    owner.advance(now, activity, backend)
}

#[test]
fn remote_physical_ptt_follows_rendered_audio_not_raw_burst_edges() {
    let (mut owner, mut backend) = scanning();
    let mut activity = Activity {
        transmit_requested: true,
        ..Activity::default()
    };
    tick(&mut owner, &mut backend, 1, activity).unwrap();
    assert_eq!(
        backend.keys.last(),
        Some(&false),
        "no RF key before rendered demand"
    );
    activity.rendered_ptt = true;
    tick(&mut owner, &mut backend, 2, activity).unwrap();
    assert_eq!(backend.keys.last(), Some(&true));
    activity.transmit_requested = false;
    activity.physical_ptt = true;
    tick(&mut owner, &mut backend, 3, activity).unwrap();
    assert_eq!(
        backend.keys.last(),
        Some(&true),
        "retain renderer-owned signaling tail"
    );
    activity.rendered_ptt = false;
    tick(&mut owner, &mut backend, 4, activity).unwrap();
    assert_eq!(backend.keys.last(), Some(&false));
}

#[test]
fn remote_receive_only_policy_survives_a_readable_radio_transmit_vfo() {
    let mut config = configuration();
    let physical = config.memories[0].state.clone();
    config.memories[0].state.transmit_hz = None;
    let mut caps = capabilities();
    caps.readback_fields = 2;
    let mut owner = RemoteBase::new(config, caps).unwrap();
    let mut backend = Backend {
        read_override: Some(physical),
        ..Backend::default()
    };
    tick(&mut owner, &mut backend, 0, Activity::default()).unwrap();
    assert!(owner.snapshot().transmit_inhibited);
    tick(
        &mut owner,
        &mut backend,
        500,
        Activity {
            transmit_requested: true,
            rendered_ptt: true,
            ..Activity::default()
        },
    )
    .unwrap();
    assert_eq!(owner.snapshot().applied.as_ref().unwrap().transmit_hz, None);
    assert!(
        owner
            .snapshot()
            .observed
            .as_ref()
            .unwrap()
            .transmit_hz
            .is_some()
    );
    assert!(owner.snapshot().transmit_inhibited);
    assert_eq!(backend.keys.last(), Some(&false));
}

#[test]
fn remote_unspecified_power_accepts_actual_readback_without_claiming_zero_power() {
    let mut config = configuration();
    let mut physical = config.memories[0].state.clone();
    physical.power_mw = Some(3000);
    config.memories[0].state.power_mw = None;
    config.limits.maximum_power_mw = None;
    let mut caps = capabilities();
    caps.readback_fields = 1 << 9;
    let mut owner = RemoteBase::new(config, caps).unwrap();
    let mut backend = Backend {
        read_override: Some(physical),
        ..Backend::default()
    };
    tick(&mut owner, &mut backend, 0, Activity::default()).unwrap();
    assert_eq!(owner.snapshot().requested.power_mw, None);
    assert_eq!(
        owner.snapshot().observed.as_ref().unwrap().power_mw,
        Some(3000)
    );
    tick(&mut owner, &mut backend, 500, Activity::default()).unwrap();
    assert_eq!(
        owner.snapshot().applied.as_ref().unwrap().power_mw,
        Some(3000)
    );
}

#[test]
fn remote_manual_tune_waits_for_rendering_and_retains_existing_turnoff() {
    let (mut owner, mut backend) = scanning();
    let id = owner
        .request(RigRequest::Tune { duration_ms: 1000 }, 0)
        .unwrap();
    tick(&mut owner, &mut backend, 1, Activity::default()).unwrap();
    assert!(
        !owner.snapshot().manual_tune_active,
        "manual intent cannot key ahead of rendering"
    );
    assert_eq!(backend.keys.last(), Some(&false));
    let mut rendering = Activity {
        rendered_ptt: true,
        transmit_requested: true,
        ..Activity::default()
    };
    tick(&mut owner, &mut backend, 2, rendering).unwrap();
    assert!(owner.snapshot().manual_tune_active);
    assert_eq!(backend.keys.last(), Some(&true));
    rendering.rendered_ptt = false;
    tick(&mut owner, &mut backend, 10, rendering).unwrap();
    assert!(!owner.snapshot().manual_tune_active);
    rendering.rendered_ptt = true;
    tick(&mut owner, &mut backend, 20, rendering).unwrap();
    assert!(owner.snapshot().manual_tune_active);
    tick(&mut owner, &mut backend, 1002, rendering).unwrap();
    assert!(!owner.snapshot().manual_tune_active);
    assert_eq!(
        backend.keys.last(),
        Some(&true),
        "ordinary demand/signaling tails still own PTT"
    );
    assert_eq!(
        owner.take_completion(),
        Some(Completion { id, result: Ok(()) })
    );
    rendering.transmit_requested = false;
    tick(&mut owner, &mut backend, 1003, rendering).unwrap();
    assert_eq!(backend.keys.last(), Some(&true));
    rendering.rendered_ptt = false;
    tick(&mut owner, &mut backend, 1020, rendering).unwrap();
    assert_eq!(backend.keys.last(), Some(&false));
}

#[test]
fn remote_manual_tune_starts_after_unkey_and_expires_from_serviced_key() {
    let (mut owner, mut backend) = scanning();
    for duration in [0, 5001] {
        assert_eq!(
            owner.request(
                RigRequest::Tune {
                    duration_ms: duration
                },
                0
            ),
            Err(Error::InvalidConfiguration)
        );
    }
    let id = owner
        .request(RigRequest::Tune { duration_ms: 1000 }, 1)
        .unwrap();
    let busy = Activity {
        command_source_active: true,
        ..Activity::default()
    };
    tick(&mut owner, &mut backend, 100, busy).unwrap();
    assert!(!owner.snapshot().manual_tune_active);
    let open_receive = Activity {
        local_receiving: true,
        ..Activity::default()
    };
    tick(&mut owner, &mut backend, 200, open_receive).unwrap();
    assert!(!owner.snapshot().manual_tune_active);
    let rendering = Activity {
        rendered_ptt: true,
        ..open_receive
    };
    tick(&mut owner, &mut backend, 201, rendering).unwrap();
    assert!(owner.snapshot().manual_tune_active);
    assert_eq!(backend.keys.last(), Some(&true));
    assert_eq!(owner.request(RigRequest::Recall(1), 200), Err(Error::Busy));
    tick(&mut owner, &mut backend, 1200, rendering).unwrap();
    assert_eq!(backend.keys.last(), Some(&true));
    tick(&mut owner, &mut backend, 1201, open_receive).unwrap();
    assert_eq!(backend.keys.last(), Some(&false));
    assert!(!owner.snapshot().manual_tune_active);
    assert_eq!(
        owner.take_completion(),
        Some(Completion { id, result: Ok(()) })
    );
}

#[test]
fn remote_manual_tune_cancellation_and_failed_unkey_keep_the_original_completion() {
    for mode in 0..4 {
        let (mut owner, mut backend) = scanning();
        let id = owner
            .request(RigRequest::Tune { duration_ms: 1000 }, 0)
            .unwrap();
        tick(&mut owner, &mut backend, 1, Activity::default()).unwrap();
        tick(
            &mut owner,
            &mut backend,
            2,
            Activity {
                rendered_ptt: true,
                ..Activity::default()
            },
        )
        .unwrap();
        let mut activity = Activity::default();
        match mode {
            0 => activity.command_cancelled = true,
            1 => {
                owner.request(RigRequest::Inhibit(true), 2).unwrap();
            }
            2 => owner.retire(),
            _ => backend.key_failures.push_back(-42),
        }
        let result = tick(&mut owner, &mut backend, 1002, activity);
        assert_eq!(backend.keys.last(), Some(&false));
        assert!(!owner.snapshot().manual_tune_active);
        let completion = owner.take_completion().unwrap();
        assert_eq!(completion.id, id);
        assert_eq!(
            completion.result,
            Err(if mode == 3 {
                Error::Backend
            } else {
                Error::Stale
            })
        );
        if mode == 3 {
            assert_eq!(result, Err(Error::Backend));
            assert_eq!(owner.snapshot().failure.unwrap().code, -42);
            tick(&mut owner, &mut backend, 1003, Activity::default()).unwrap();
            assert_eq!(owner.snapshot().ptt_applied, Some(false));
        }
    }
}

#[test]
fn remote_rejected_manual_tune_does_not_cancel_a_waiting_scan_member() {
    let (base, _) = scanning();
    let mut caps = base.capabilities().clone();
    caps.manual_tune = false;
    let mut owner = RemoteBase::new(base.configuration().clone(), caps).unwrap();
    let mut backend = Backend::default();
    tick(&mut owner, &mut backend, 0, Activity::default()).unwrap();
    start_scan(&mut owner, 1);
    tick(&mut owner, &mut backend, 1, Activity::default()).unwrap();
    owner.take_completion().unwrap();
    let due = owner.scan_snapshot().unwrap().next_deadline_ms.unwrap();
    tick(
        &mut owner,
        &mut backend,
        due,
        Activity {
            physical_ptt: true,
            ..Activity::default()
        },
    )
    .unwrap();
    assert!(owner.snapshot().pending);
    let writes = backend.writes.len();
    assert_eq!(
        owner.request(RigRequest::Tune { duration_ms: 1000 }, due),
        Err(Error::Unsupported)
    );
    tick(&mut owner, &mut backend, due + 1, Activity::default()).unwrap();
    assert_eq!(backend.writes.len(), writes + 1);
}

fn start_scan(owner: &mut RemoteBase, now: u64) {
    owner
        .request(
            RigRequest::Scan(ScanRequest::Start {
                selector: 1,
                direction: ScanDirection::Up,
                dwell_ms: None,
            }),
            now,
        )
        .unwrap();
}

#[test]
fn remote_scan_uses_completed_tune_deadlines() {
    let (mut owner, mut backend) = scanning();
    backend.apply_delay_ms = 200;
    start_scan(&mut owner, 100);
    tick(&mut owner, &mut backend, 100, Activity::default()).unwrap();
    assert_eq!(owner.scan_snapshot().unwrap().current_memory, Some(2));
    assert_eq!(owner.scan_snapshot().unwrap().next_deadline_ms, Some(800));
    tick(&mut owner, &mut backend, 799, Activity::default()).unwrap();
    assert_eq!(owner.scan_snapshot().unwrap().current_memory, Some(2));
    tick(&mut owner, &mut backend, 800, Activity::default()).unwrap();
    assert_eq!(owner.scan_snapshot().unwrap().current_memory, Some(1));
    let busy = Activity {
        local_receiving: true,
        ..Activity::default()
    };
    tick(&mut owner, &mut backend, 1500, busy).unwrap();
    tick(&mut owner, &mut backend, 1600, Activity::default()).unwrap();
    tick(&mut owner, &mut backend, 3599, Activity::default()).unwrap();
    assert_eq!(owner.scan_snapshot().unwrap().current_memory, Some(1));
    tick(&mut owner, &mut backend, 3600, Activity::default()).unwrap();
    assert_eq!(owner.scan_snapshot().unwrap().current_memory, Some(2));
}

#[test]
fn remote_scan_rejects_burst_overlapping_tune_until_unkey() {
    let (mut owner, mut backend) = scanning();
    backend.overlap_next_apply = true;
    start_scan(&mut owner, 1);
    tick(&mut owner, &mut backend, 1, Activity::default()).unwrap();
    let transmit = Activity {
        linked_receiving: true,
        transmit_requested: true,
        rendered_ptt: true,
        ..Activity::default()
    };
    tick(&mut owner, &mut backend, 2, transmit).unwrap();
    assert_eq!(backend.keys.last(), Some(&false));
    assert_eq!(owner.snapshot().scan_rejected_bursts, 1);
    tick(&mut owner, &mut backend, 3, Activity::default()).unwrap();
    tick(&mut owner, &mut backend, 4, transmit).unwrap();
    assert_eq!(backend.keys.last(), Some(&true));
}

#[test]
fn remote_scan_stop_cancels_pending_start_and_preserves_applied_member() {
    let (mut owner, mut backend) = scanning();
    start_scan(&mut owner, 1);
    let busy = Activity {
        local_receiving: true,
        ..Activity::default()
    };
    tick(&mut owner, &mut backend, 1, busy).unwrap();
    let before = backend.writes.len();
    owner
        .request(RigRequest::Scan(ScanRequest::Stop), 2)
        .unwrap();
    tick(&mut owner, &mut backend, 3, Activity::default()).unwrap();
    assert_eq!(backend.writes.len(), before);
    assert_eq!(owner.snapshot().applied.unwrap().receive_hz, 146_520_000);
    assert!(!owner.scan_snapshot().unwrap().active);
    assert_eq!(owner.take_completion().unwrap().result, Err(Error::Stale));
    assert!(owner.take_completion().unwrap().result.is_ok());
}

#[test]
fn remote_scan_all_failing_members_stop_safely() {
    let (mut owner, mut backend) = scanning();
    backend.failures.extend([-55, 0, -66, 0]);
    start_scan(&mut owner, 1);
    assert_eq!(
        tick(&mut owner, &mut backend, 1, Activity::default()),
        Err(Error::Backend)
    );
    assert_eq!(
        tick(&mut owner, &mut backend, 2, Activity::default()),
        Err(Error::Backend)
    );
    assert!(!owner.scan_snapshot().unwrap().active);
    assert!(owner.snapshot().transmit_inhibited);
    assert_eq!(owner.scan_snapshot().unwrap().last_error, Some(-66));
}

#[test]
fn remote_scan_priority_never_repeats_as_an_ordinary_member() {
    let (owner, _) = scanning();
    let mut config = owner.configuration().clone();
    let mut third = config.memories[0].clone();
    third.selector = 3;
    third.name = "priority".into();
    third.state.receive_hz = 147_000_000;
    third.state.transmit_hz = Some(147_000_000);
    config.memories.push(third);
    config.scan_lists[0].members.push(3);
    config.scan_lists[0].priority = Some(3);
    let mut caps = capabilities();
    caps.scan_modes = vec![Mode::Fm];
    let mut owner = RemoteBase::new(config, caps).unwrap();
    let mut backend = Backend::default();
    tick(&mut owner, &mut backend, 0, Activity::default()).unwrap();
    start_scan(&mut owner, 1);
    for (now, expected) in [(1, 2), (501, 3), (1001, 1), (1501, 3), (2001, 2)] {
        tick(&mut owner, &mut backend, now, Activity::default()).unwrap();
        assert_eq!(
            owner.scan_snapshot().unwrap().current_memory,
            Some(expected)
        );
    }
}

#[test]
fn remote_scan_receive_only_member_never_hops_to_admit_active_transmit() {
    let (owner, _) = scanning();
    let mut config = owner.configuration().clone();
    config.memories[1].state.transmit_hz = None;
    let mut caps = capabilities();
    caps.scan_modes = vec![Mode::Fm];
    let mut owner = RemoteBase::new(config, caps).unwrap();
    let mut backend = Backend::default();
    tick(&mut owner, &mut backend, 0, Activity::default()).unwrap();
    start_scan(&mut owner, 1);
    tick(&mut owner, &mut backend, 1, Activity::default()).unwrap();
    let transmit = Activity {
        linked_receiving: true,
        transmit_requested: true,
        rendered_ptt: true,
        ..Activity::default()
    };
    tick(&mut owner, &mut backend, 1000, transmit).unwrap();
    assert_eq!(owner.scan_snapshot().unwrap().current_memory, Some(2));
    assert_eq!(backend.keys.last(), Some(&false));
    tick(&mut owner, &mut backend, 1001, Activity::default()).unwrap();
    tick(&mut owner, &mut backend, 3000, Activity::default()).unwrap();
    assert_eq!(owner.scan_snapshot().unwrap().current_memory, Some(2));
    tick(&mut owner, &mut backend, 3001, Activity::default()).unwrap();
    assert_eq!(owner.scan_snapshot().unwrap().current_memory, Some(1));
}

#[test]
fn remote_scan_pause_resume_skip_and_recall_keep_one_operating_state() {
    let (mut owner, mut backend) = scanning();
    start_scan(&mut owner, 1);
    tick(&mut owner, &mut backend, 1, Activity::default()).unwrap();
    owner.take_completion().unwrap();
    for request in [
        ScanRequest::Pause,
        ScanRequest::SetDwell(250),
        ScanRequest::SetResumeDelay(0),
        ScanRequest::Direction(ScanDirection::Down),
    ] {
        owner.request(RigRequest::Scan(request), 2).unwrap();
        assert!(owner.take_completion().unwrap().result.is_ok());
    }
    let count = backend.writes.len();
    tick(&mut owner, &mut backend, 1000, Activity::default()).unwrap();
    assert_eq!(backend.writes.len(), count);
    owner
        .request(RigRequest::Scan(ScanRequest::Resume), 1001)
        .unwrap();
    owner.take_completion();
    tick(&mut owner, &mut backend, 1001, Activity::default()).unwrap();
    assert_eq!(owner.scan_snapshot().unwrap().current_memory, Some(1));
    owner
        .request(RigRequest::Scan(ScanRequest::Skip), 1002)
        .unwrap();
    owner.take_completion();
    tick(&mut owner, &mut backend, 1002, Activity::default()).unwrap();
    assert_eq!(owner.scan_snapshot().unwrap().current_memory, Some(2));
    owner.request(RigRequest::Recall(1), 1003).unwrap();
    tick(&mut owner, &mut backend, 1003, Activity::default()).unwrap();
    assert!(!owner.scan_snapshot().unwrap().active);
    assert_eq!(owner.snapshot().applied.unwrap().receive_hz, 146_520_000);
    let count = backend.writes.len();
    tick(&mut owner, &mut backend, 10_000, Activity::default()).unwrap();
    assert_eq!(backend.writes.len(), count);
}

#[test]
fn remote_scan_requires_usable_activity_qualification() {
    let mut owner = RemoteBase::new(configuration(), capabilities()).unwrap();
    let mut backend = Backend::default();
    owner.advance(0, Activity::default(), &mut backend).unwrap();
    assert_eq!(
        owner.request(
            RigRequest::Scan(ScanRequest::Start {
                selector: 1,
                direction: ScanDirection::Up,
                dwell_ms: None
            }),
            1
        ),
        Err(Error::Unsupported)
    );
    assert!(owner.scan_snapshot().is_none());
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
                rendered_ptt: true,
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
    caps.readback_fields = 65535;
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
    caps.readback_fields = 65535;
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
                rendered_ptt: true,
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
    caps.readback_fields = 65535;
    let mut radio = RemoteBase::new(configuration(), caps).unwrap();
    let mut backend = Backend::default();
    radio.advance(0, Activity::default(), &mut backend).unwrap();
    radio
        .advance(
            1,
            Activity {
                transmit_requested: true,
                rendered_ptt: true,
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
    caps.readback_fields = 65535;
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
                rendered_ptt: true,
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
