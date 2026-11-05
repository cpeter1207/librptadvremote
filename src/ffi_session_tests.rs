//! Exercise the public session operations against a copied physical-backend fixture.

use super::*;
use crate::ffi::tests::{with_owner, with_owner_mode};
use std::ptr;

struct Rig {
    state: OperatingState,
    keyed: bool,
    tuned: bool,
    writes: u32,
    error: u8,
}
impl Rig {
    fn new() -> Self {
        Self {
            state: OperatingState {
                receive_hz: 146_520_000,
                transmit_hz: Some(146_520_000),
                power_mw: Some(5000),
                ..Default::default()
            },
            keyed: false,
            tuned: false,
            writes: 0,
            error: 0,
        }
    }
    fn backend(&mut self) -> Backend {
        unsafe extern "C" fn read(context: *mut c_void, out: *mut State) -> i32 {
            // SAFETY: advance retains this unique fixture and the complete writable record.
            let rig = unsafe { &mut *context.cast::<Rig>() };
            if rig.error == 1 {
                return -71;
            }
            let state = if rig.error == 2 {
                State::from(&OperatingState::default())
            } else {
                State::from(&rig.state)
            };
            unsafe { out.write(state) };
            if rig.error == 2 {
                unsafe { ptr::write_bytes(out, 0, 1) };
            }
            0
        }
        unsafe extern "C" fn apply(context: *mut c_void, state: *const State) -> i32 {
            // SAFETY: the serialized fixture and borrowed complete state remain live.
            let rig = unsafe { &mut *context.cast::<Rig>() };
            if rig.error == 3 {
                return -73;
            }
            rig.state = unsafe { &*state }.resolve().unwrap();
            rig.writes += 1;
            0
        }
        unsafe extern "C" fn ptt(context: *mut c_void, keyed: u32) -> i32 {
            let rig = unsafe { &mut *context.cast::<Rig>() };
            if rig.error == 4 {
                return -74;
            }
            rig.keyed = keyed != 0;
            0
        }
        unsafe extern "C" fn tune(context: *mut c_void, keyed: u32) -> i32 {
            let rig = unsafe { &mut *context.cast::<Rig>() };
            if rig.error == 5 {
                return -75;
            }
            rig.tuned = keyed != 0;
            0
        }
        unsafe extern "C" fn clock(_: *mut c_void) -> u64 {
            0
        }
        Backend {
            struct_size: size_of::<Backend>() as u32,
            abi_version: 1,
            context: ptr::from_mut(self).cast(),
            read_state: Some(read),
            apply_state: Some(apply),
            set_ptt: Some(ptt),
            monotonic_ms: Some(clock),
            transmit_epoch: Some(clock),
            set_tune: Some(tune),
        }
    }
}

fn activity(flags: u32) -> RawActivity {
    RawActivity {
        struct_size: size_of::<RawActivity>() as u32,
        flags,
    }
}
fn tick(handle: *mut c_void, rig: &mut Rig, flags: u32, now: u64) -> i32 {
    advance(handle, now, &activity(flags), &rig.backend())
}
fn operation(kind: u32) -> Request {
    Request {
        struct_size: size_of::<Request>() as u32,
        abi_version: 1,
        kind,
        selector: 1,
        fields: 0,
        state: State::from(&Rig::new().state),
        value_ms: 250,
        direction: 1,
    }
}
fn current(handle: *mut c_void) -> Snapshot {
    let mut out: Snapshot = unsafe { std::mem::zeroed() };
    out.struct_size = size_of::<Snapshot>() as u32;
    assert_eq!(snapshot(handle, &mut out), 0);
    out
}
fn drain(handle: *mut c_void) {
    let mut completion = RawCompletion {
        request_id: 0,
        result: 0,
    };
    while take_completion(handle, &mut completion) == 0 {
        assert_ne!(completion.request_id, 0);
    }
}

#[test]
fn c_session_requests_preserve_operations_completion_and_scan_status() {
    for kind in 1..=13 {
        with_owner(|handle| {
            let initial = current(handle);
            assert_eq!(initial.flags & (1 | 2), 0);
            assert_ne!(initial.flags & 16, 0);
            let mut rig = Rig::new();
            assert_eq!(tick(handle, &mut rig, 0, 0), 0);
            assert_eq!(rig.writes, 1);
            let mut scan: RawScanSnapshot = unsafe { std::mem::zeroed() };
            scan.struct_size = size_of::<RawScanSnapshot>() as u32;
            assert_eq!(scan_snapshot(handle, &mut scan), 1);
            let mut id = 0;
            if (5..=12).contains(&kind) {
                assert_eq!(request(handle, &operation(4), 1, &mut id), 0);
                assert_eq!(tick(handle, &mut rig, 0, 1), 0);
                drain(handle);
                if kind == 6 {
                    assert_eq!(request(handle, &operation(5), 1, &mut id), 0);
                    drain(handle);
                }
            }
            let mut input = operation(kind);
            if kind == 1 {
                input.fields = 1;
            }
            if kind == 3 {
                input.selector = 0;
            }
            if kind == 8 {
                input.direction = 0;
            }
            assert_eq!(request(handle, &input, 2, &mut id), 0, "kind {kind}");
            assert_ne!(id, 0);
            assert_eq!(tick(handle, &mut rig, 0, 2), 0);
            if kind == 4 || kind == 8 {
                assert_eq!(scan_snapshot(handle, &mut scan), 0);
                assert_eq!(scan.list, 1);
                assert_eq!(scan.direction, u32::from(kind == 4));
            }
            if kind == 13 {
                assert!(rig.tuned);
                assert_eq!(tick(handle, &mut rig, 64 | 16, 3), 0);
                assert_ne!(current(handle).flags & 512, 0);
                assert!(rig.keyed);
                assert_eq!(tick(handle, &mut rig, 32, 4), 0);
                assert!(!rig.tuned);
            }
            drain(handle);
            assert_eq!(retire(handle), 0);
            assert_eq!(
                request(handle, &operation(2), 3, &mut id),
                Error::Stale as i32
            );
        });
    }
}

#[test]
fn c_session_rejects_bad_records_before_calling_a_backend() {
    with_owner(|handle| {
        let mut rig = Rig::new();
        for defect in 0..10 {
            let mut backend = rig.backend();
            let mut input = activity(0);
            match defect {
                0 => input.struct_size = 0,
                1 => input.flags = 128,
                2 => backend.struct_size = 0,
                3 => backend.abi_version = 2,
                4 => backend.apply_state = None,
                5 => backend.set_ptt = None,
                6 => backend.read_state = None,
                7 => backend.set_tune = None,
                8 => backend.monotonic_ms = None,
                _ => backend.transmit_epoch = None,
            }
            assert_eq!(advance(handle, 0, &input, &backend), -1, "defect {defect}");
        }
        assert_eq!(rig.writes, 0);
        for defect in 0..7 {
            let mut input = operation(2);
            match defect {
                0 => input.struct_size = 0,
                1 => input.abi_version = 2,
                2 => input.fields = 1,
                3 => input.kind = 0,
                4 => {
                    input.kind = 3;
                    input.selector = 2;
                }
                5 => {
                    input.kind = 4;
                    input.direction = 2;
                }
                _ => {
                    input.kind = 1;
                    input.fields = 0;
                }
            }
            let mut out = 123;
            assert_eq!(request(handle, &input, 0, &mut out), -1);
            assert_eq!(out, 0);
        }
        let mut id = 0;
        for bad in [ptr::null(), ptr::without_provenance(1)] {
            assert_eq!(request(handle, bad, 0, &mut id), -1);
            assert_eq!(advance(handle, 0, bad.cast(), &rig.backend()), -1);
        }
        for bad in [ptr::null_mut(), ptr::without_provenance_mut(1)] {
            assert_eq!(request(handle, &operation(2), 0, bad), -1);
            assert_eq!(take_completion(handle, bad.cast()), -1);
            assert_eq!(snapshot(handle, bad.cast()), -1);
            assert_eq!(scan_snapshot(handle, bad.cast()), -1);
            assert_eq!(retire(bad.cast()), -1);
        }
        assert_eq!(request(ptr::null_mut(), &operation(2), 0, &mut id), -1);
    });
}

#[test]
fn c_session_retains_actual_backend_failures_and_supports_optional_non_scan_clocks() {
    for (error, expected) in [
        (1, -71),
        (2, Error::Mode as i32),
        (3, -73),
        (4, -74),
        (5, -75),
    ] {
        with_owner(|handle| {
            let mut rig = Rig::new();
            if error >= 4 {
                assert_eq!(tick(handle, &mut rig, 0, 0), 0);
            }
            rig.error = error;
            if error == 5 {
                let mut id = 0;
                assert_eq!(request(handle, &operation(13), 1, &mut id), 0);
            }
            let _ = tick(handle, &mut rig, if error == 4 { 64 } else { 0 }, 1);
            let value = current(handle);
            assert_eq!(value.failure_code, expected, "backend error {error}");
            assert_ne!(value.flags & 64, 0);
        });
    }
    with_owner_mode(false, true, true, |handle| {
        let mut rig = Rig::new();
        let mut backend = rig.backend();
        backend.monotonic_ms = None;
        backend.transmit_epoch = None;
        assert_eq!(advance(handle, 0, &activity(0), &backend), 0);
        assert_eq!(current(handle).flags & 7, 7);
    });
    with_owner_mode(false, false, false, |handle| {
        let mut rig = Rig::new();
        let mut backend = rig.backend();
        backend.read_state = None;
        backend.set_tune = None;
        assert_eq!(advance(handle, 0, &activity(0), &backend), 0);
        let value = current(handle);
        assert_eq!(value.flags & 3, 1);
        assert_eq!(value.observed_fields, 0);
        let mut id = 0;
        assert_eq!(
            request(handle, &operation(13), 1, &mut id),
            Error::Unsupported as i32
        );
        assert_eq!(id, 0);
    });
}
