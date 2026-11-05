use super::{Backend, capabilities, configuration, running_radio, scanning, tick};
use crate::*;

#[test]
fn session_contract_applies_complete_patch_fields_after_physical_unkey() {
    let mut caps = capabilities();
    caps.readback_fields = 65535;
    let mut owner = RemoteBase::new(configuration(), caps).unwrap();
    let mut backend = Backend::default();
    owner.advance(0, Activity::default(), &mut backend).unwrap();
    let request = owner
        .request(
            RigRequest::Set(OperatingPatch {
                receive_hz: Some(146_600_000),
                transmit_hz: Some(Some(146_600_000)),
                mode: Some(Mode::Fm),
                bandwidth_hz: Some(12_000),
                receive_vfo: Some(1),
                transmit_vfo: Some(2),
                split: Some(true),
                offset_hz: Some(600_000),
                rit_hz: Some(300),
                rit_enabled: Some(true),
                xit_hz: Some(300),
                xit_enabled: Some(true),
                power_mw: Some(2500),
                channel: Some(7),
                encode: Some(Signaling {
                    source: SignalSource::Dsp,
                    enabled: true,
                    ctcss_tenths_hz: 1148,
                    ..Default::default()
                }),
                decode: Some(Signaling {
                    source: SignalSource::Dsp,
                    enabled: true,
                    dcs_code: 0o245,
                    inverted: true,
                    ..Default::default()
                }),
            }),
            1,
        )
        .unwrap();
    tick(&mut owner, &mut backend, 1, Activity::default()).unwrap();
    let applied = owner.snapshot().applied.unwrap();
    assert_eq!(
        (applied.receive_hz, applied.transmit_hz),
        (146_600_000, Some(146_600_000))
    );
    assert_eq!(
        (
            applied.bandwidth_hz,
            applied.receive_vfo,
            applied.transmit_vfo
        ),
        (12_000, 1, 2)
    );
    assert_eq!(
        (applied.offset_hz, applied.rit_hz, applied.xit_hz),
        (600_000, 300, 300)
    );
    assert!(applied.split && applied.rit_enabled && applied.xit_enabled);
    assert_eq!((applied.power_mw, applied.channel), (Some(2500), Some(7)));
    assert_eq!(applied.encode.ctcss_tenths_hz, 1148);
    assert_eq!(applied.decode.dcs_code, 0o245);
    assert!(applied.decode.inverted && owner.snapshot().verified);
    assert_eq!(owner.take_completion().unwrap().id, request);
}

#[test]
fn session_contract_reserved_inhibit_and_scan_slots_cannot_overwrite_completions() {
    let (mut owner, _) = running_radio();
    let inhibit = owner.request(RigRequest::Inhibit(true), 1).unwrap();
    assert_eq!(
        owner.request(RigRequest::Inhibit(true), 1),
        Err(Error::Busy)
    );
    assert_eq!(owner.request(RigRequest::Recall(1), 1), Err(Error::Busy));
    assert_eq!(
        owner.request(RigRequest::Scan(ScanRequest::Stop), 1),
        Err(Error::Busy)
    );
    assert_eq!(owner.take_completion().unwrap().id, inhibit);
    assert_eq!(
        owner.request(RigRequest::Tune { duration_ms: 1 }, 2),
        Err(Error::Unsupported)
    );
    let clear = owner.request(RigRequest::Inhibit(false), 2).unwrap();
    assert!(!owner.snapshot().transmit_inhibited);
    assert_eq!(owner.request(RigRequest::Recall(1), 2), Err(Error::Busy));
    assert_eq!(owner.take_completion().unwrap().id, clear);
    assert_eq!(
        owner.request(RigRequest::Scan(ScanRequest::Pause), 3),
        Err(Error::NotFound)
    );
    assert_eq!(
        owner.request(RigRequest::Scan(ScanRequest::SetPriority(Some(1))), 3),
        Err(Error::NotFound)
    );
    let stopped = owner
        .request(RigRequest::Scan(ScanRequest::Stop), 3)
        .unwrap();
    assert_eq!(owner.take_completion().unwrap().id, stopped);
    let recall = owner.request(RigRequest::Recall(1), 4).unwrap();
    assert_eq!(
        owner.request(RigRequest::Inhibit(false), 4),
        Err(Error::Busy)
    );
    assert_eq!(
        owner.request(RigRequest::Tune { duration_ms: 1 }, 4),
        Err(Error::Busy)
    );
    let mut backend = Backend::default();
    tick(&mut owner, &mut backend, 4, Activity::default()).unwrap();
    assert_eq!(owner.take_completion().unwrap().id, recall);
    let (mut owner, mut backend) = scanning();
    let start = owner
        .request(
            RigRequest::Scan(ScanRequest::Start {
                selector: 1,
                direction: ScanDirection::Up,
                dwell_ms: None,
            }),
            0,
        )
        .unwrap();
    assert_eq!(owner.request(RigRequest::Recall(1), 0), Err(Error::Busy));
    let pause = owner
        .request(RigRequest::Scan(ScanRequest::Pause), 0)
        .unwrap();
    assert_eq!(
        owner.take_completion(),
        Some(Completion {
            id: start,
            result: Err(Error::Stale)
        })
    );
    assert_eq!(
        owner.take_completion(),
        Some(Completion {
            id: pause,
            result: Ok(())
        })
    );
    let resumes = owner
        .request(RigRequest::Scan(ScanRequest::Resume), 1)
        .unwrap();
    assert_eq!(owner.take_completion().unwrap().id, resumes);
    tick(&mut owner, &mut backend, 1, Activity::default()).unwrap();
    owner
        .request(RigRequest::Scan(ScanRequest::SetPriority(Some(2))), 2)
        .unwrap();
    assert_eq!(owner.scan_snapshot().unwrap().priority, Some(2));
    owner.take_completion().unwrap();
    owner
        .request(RigRequest::Tune { duration_ms: 1 }, 3)
        .unwrap();
    assert!(!owner.scan_snapshot().unwrap().active);
}

#[test]
fn session_contract_source_cancellation_discards_queued_tunes_without_restoring_unknown_state() {
    for initial_failure in [false, true] {
        let mut owner = RemoteBase::new(configuration(), capabilities()).unwrap();
        let mut backend = Backend::default();
        if initial_failure {
            backend.failures.push_back(-41);
        }
        let result = owner.advance(0, Activity::default(), &mut backend);
        assert_eq!(result.is_err(), initial_failure);
        let id = owner
            .request(
                RigRequest::Set(OperatingPatch {
                    receive_hz: Some(146_560_000),
                    ..Default::default()
                }),
                1,
            )
            .unwrap();
        let before = backend.writes.len();
        tick(
            &mut owner,
            &mut backend,
            1,
            Activity {
                command_cancelled: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(backend.writes.len(), before);
        assert!(!owner.snapshot().pending);
        assert_eq!(
            owner.take_completion(),
            Some(Completion {
                id,
                result: Err(Error::Stale)
            })
        );
        assert_eq!(
            owner.snapshot().requested.receive_hz,
            if initial_failure {
                146_560_000
            } else {
                146_520_000
            }
        );
        assert_eq!(owner.snapshot().transmit_inhibited, initial_failure);
    }
}

#[test]
fn session_contract_front_panel_observation_preserves_a_waiting_manual_request() {
    let mut caps = capabilities();
    caps.readback_fields = 65535;
    let mut config = configuration();
    config.poll_interval_ms = 1;
    let mut owner = RemoteBase::new(config, caps).unwrap();
    let mut backend = Backend::default();
    owner.advance(0, Activity::default(), &mut backend).unwrap();
    let mut actual = backend.state.clone().unwrap();
    actual.receive_hz = 146_560_000;
    actual.rit_enabled = true;
    actual.xit_enabled = true;
    backend.read_override = Some(actual);
    tick(&mut owner, &mut backend, 1, Activity::default()).unwrap();
    assert_eq!(owner.snapshot().requested.receive_hz, 146_560_000);
    let id = owner
        .request(
            RigRequest::Set(OperatingPatch {
                receive_hz: Some(146_580_000),
                ..Default::default()
            }),
            2,
        )
        .unwrap();
    tick(
        &mut owner,
        &mut backend,
        2,
        Activity {
            command_source_active: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(owner.snapshot().applied.unwrap().receive_hz, 146_560_000);
    assert_eq!(owner.snapshot().requested.receive_hz, 146_580_000);
    backend.read_override = None;
    tick(&mut owner, &mut backend, 3, Activity::default()).unwrap();
    assert_eq!(
        owner.take_completion(),
        Some(Completion { id, result: Ok(()) })
    );
}

#[test]
fn session_contract_automatic_work_waits_for_activity_and_stop_cancels_only_that_work() {
    for activity in [
        Activity {
            local_receiving: true,
            ..Default::default()
        },
        Activity {
            linked_receiving: true,
            ..Default::default()
        },
        Activity {
            transmit_requested: true,
            ..Default::default()
        },
    ] {
        let (mut owner, mut backend) = scanning();
        let start = owner
            .request(
                RigRequest::Scan(ScanRequest::Start {
                    selector: 1,
                    direction: ScanDirection::Up,
                    dwell_ms: None,
                }),
                1,
            )
            .unwrap();
        tick(
            &mut owner,
            &mut backend,
            1,
            Activity {
                physical_ptt: true,
                ..Default::default()
            },
        )
        .unwrap();
        let writes = backend.writes.len();
        assert!(owner.snapshot().pending);
        tick(&mut owner, &mut backend, 2, activity).unwrap();
        assert_eq!(backend.writes.len(), writes);
        let stop = owner
            .request(RigRequest::Scan(ScanRequest::Stop), 2)
            .unwrap();
        assert!(!owner.snapshot().pending);
        assert_eq!(
            owner.take_completion(),
            Some(Completion {
                id: start,
                result: Err(Error::Stale)
            })
        );
        assert_eq!(
            owner.take_completion(),
            Some(Completion {
                id: stop,
                result: Ok(())
            })
        );
    }
    let mut owner = RemoteBase::new(configuration(), capabilities()).unwrap();
    let mut backend = Backend::default();
    tick(
        &mut owner,
        &mut backend,
        0,
        Activity {
            command_cancelled: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(owner.snapshot().applied.is_some());
    assert!(owner.take_completion().is_none());
    tick(
        &mut owner,
        &mut backend,
        1,
        Activity {
            command_cancelled: true,
            ..Default::default()
        },
    )
    .unwrap();
}

#[test]
fn session_contract_pending_retune_waits_for_rendered_audio_and_failed_key_confirms_unkey() {
    let (mut owner, mut backend) = running_radio();
    let id = owner.request(RigRequest::Recall(1), 1).unwrap();
    let writes = backend.writes.len();
    tick(
        &mut owner,
        &mut backend,
        1,
        Activity {
            rendered_ptt: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(backend.writes.len(), writes);
    tick(
        &mut owner,
        &mut backend,
        2,
        Activity {
            physical_ptt: true,
            ..Default::default()
        },
    )
    .unwrap();
    tick(&mut owner, &mut backend, 3, Activity::default()).unwrap();
    assert_eq!(owner.take_completion().unwrap().id, id);
    backend.key_failures.push_back(-41);
    assert_eq!(
        tick(
            &mut owner,
            &mut backend,
            4,
            Activity {
                rendered_ptt: true,
                ..Default::default()
            }
        ),
        Err(Error::Backend)
    );
    assert_eq!(owner.snapshot().ptt_applied, Some(false));
    assert_eq!(owner.snapshot().failure.unwrap().code, -41);
}
