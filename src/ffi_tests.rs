//! C record admission and copied-memory behavior, using the descriptor-smoke fixture.

use super::*;
use std::ptr;

fn state() -> State {
    // Every field is a scalar, and zero is a valid representation before admission.
    let mut state: State = unsafe { std::mem::zeroed() };
    state.receive_hz = 146_520_000;
    state.transmit_hz = 146_520_000;
    state.flags = TX | POWER;
    state.mode = 1;
    state.power_mw = 5000;
    state
}

fn span<T>(data: &[T]) -> Span<T> {
    Span {
        data: data.as_ptr(),
        length: data.len(),
    }
}

fn with_setup(test: impl FnOnce(&mut Setup)) {
    let ranges = [FrequencyRange::new(144_000_000, 148_000_000)];
    let hardware = [RawCoverage {
        minimum: 144_000_000,
        maximum: 148_000_000,
        mode_mask: 2,
        flags: 3,
        minimum_power_mw: 100,
        maximum_power_mw: 10_000,
    }];
    let memories = [RawMemory {
        selector: 1,
        name: c"local".as_ptr(),
        name_length: 5,
        state: state(),
    }];
    let members = [1];
    let scans = [RawScan {
        selector: 1,
        name: c"local".as_ptr(),
        name_length: 5,
        members: members.as_ptr(),
        member_count: 1,
        dwell_ms: 500,
        resume_delay_ms: 2000,
        settle_ms: 100,
        priority: 1,
    }];
    let mut setup = Setup {
        struct_size: size_of::<Setup>() as u32,
        abi_version: 1,
        limits: RawLimits {
            receive: span(&ranges),
            transmit: span(&ranges),
            receive_only: span(&[]),
            mode_mask: 2,
            flags: POWER,
            maximum_power_mw: 10_000,
        },
        capabilities: RawCapabilities {
            receive: span(&hardware),
            transmit: span(&hardware),
            mode_mask: 2,
            readback_fields: 65535,
            scan_mode_mask: 2,
            flags: POWER | 8,
            encode_sources: 15,
            decode_sources: 15,
            decode_control_sources: 15,
        },
        memories: span(&memories),
        scan_lists: span(&scans),
        startup_memory: 1,
        poll_interval_ms: 500,
    };
    test(&mut setup);
}

pub(crate) fn with_owner(test: impl FnOnce(*mut c_void)) {
    with_owner_mode(true, true, true, test);
}

pub(crate) fn with_owner_mode(
    scanning: bool,
    readback: bool,
    tune: bool,
    test: impl FnOnce(*mut c_void),
) {
    struct Owner(*mut c_void);
    impl Drop for Owner {
        fn drop(&mut self) {
            destroy(self.0);
        }
    }
    with_setup(|setup| {
        if !scanning {
            setup.scan_lists = span(&[]);
        }
        if !readback {
            setup.capabilities.readback_fields = 0;
        }
        if !tune {
            setup.capabilities.flags &= !8;
        }
        let mut handle = ptr::null_mut();
        assert_eq!(create(setup, &mut handle), 0);
        let owner = Owner(handle);
        test(owner.0);
    });
}

#[test]
fn c_record_spans_and_enumerations_reject_unreadable_or_unknown_values() {
    let invalid_hardware = [RawCoverage {
        minimum: 144_000_000,
        maximum: 148_000_000,
        mode_mask: 2,
        flags: 4,
        minimum_power_mw: 0,
        maximum_power_mw: 0,
    }];
    assert_eq!(
        unsafe { coverage(&span(&invalid_hardware)) },
        Err(Error::InvalidConfiguration)
    );
    let values = [12_u16, 34];
    let valid = span(&values);
    assert_eq!(unsafe { valid.read() }, Ok(values.as_slice()));
    let empty = Span::<u16> {
        data: ptr::null(),
        length: 0,
    };
    assert_eq!(unsafe { empty.read() }, Ok([].as_slice()));
    for (data, length) in [
        (ptr::null(), 1),
        (ptr::without_provenance(1), 1),
        (values.as_ptr(), usize::MAX),
    ] {
        let invalid = Span::<u16> { data, length };
        assert_eq!(unsafe { invalid.read() }, Err(Error::InvalidConfiguration));
    }
    assert_eq!(unsafe { text(c"local".as_ptr(), 5) }, Ok("local".into()));
    assert_eq!(
        unsafe { text([255_u8].as_ptr().cast(), 1) },
        Err(Error::InvalidConfiguration)
    );
    let expected = [
        Mode::Fm,
        Mode::Am,
        Mode::Usb,
        Mode::Lsb,
        Mode::Cw,
        Mode::CwReverse,
        Mode::Rtty,
        Mode::RttyReverse,
        Mode::WideFm,
        Mode::PacketFm,
        Mode::PacketUsb,
        Mode::PacketLsb,
    ];
    for (value, expected) in (1..=12).zip(expected) {
        assert_eq!(mode(value), Ok(expected));
    }
    assert_eq!(modes(0x1ffe).unwrap(), expected);
    assert!(modes(0).unwrap().is_empty());
    for value in [0, 13, u32::MAX] {
        assert_eq!(mode(value), Err(Error::Mode));
    }
    for mask in [1, 1 << 63] {
        assert_eq!(modes(mask), Err(Error::Mode));
    }
    let expected = [
        SignalSource::Disabled,
        SignalSource::Dsp,
        SignalSource::External,
        SignalSource::Rig,
    ];
    for (value, expected) in (0..=3).zip(expected) {
        assert_eq!(source(value), Ok(expected));
    }
    assert_eq!(sources(15).unwrap(), expected);
    assert!(sources(0).unwrap().is_empty());
    assert_eq!(source(4), Err(Error::Signaling));
    assert_eq!(sources(16), Err(Error::Signaling));
}

#[test]
fn c_state_patches_copy_selected_fields_and_validate_only_selected_values() {
    let mut raw = state();
    raw.flags |= CHANNEL | SPLIT | RIT_ENABLED | XIT_ENABLED;
    raw.channel = 255;
    raw.encode.source = 1;
    raw.encode.control_source = 3;
    raw.encode.enabled = 1;
    raw.encode.ctcss_tenths_hz = 1148;
    raw.decode.source = 2;
    raw.decode.inverted = 1;
    let resolved = raw.resolve().unwrap();
    assert_eq!(resolved.receive_hz, 146_520_000);
    assert_eq!(resolved.transmit_hz, Some(146_520_000));
    assert_eq!(resolved.channel, Some(255));
    assert_eq!(resolved.power_mw, Some(5000));
    assert_eq!(resolved.encode.control_source, Some(SignalSource::Rig));
    assert!(resolved.split && resolved.rit_enabled && resolved.xit_enabled);
    assert_eq!(State::from(&resolved).resolve(), Ok(resolved));
    let patch = raw.patch(65535).unwrap();
    assert_eq!(patch.receive_hz, Some(146_520_000));
    assert_eq!(patch.transmit_hz, Some(Some(146_520_000)));
    assert_eq!(patch.channel, Some(255));
    assert_eq!(patch.split, Some(true));
    assert_eq!(patch.rit_enabled, Some(true));
    assert_eq!(patch.xit_enabled, Some(true));
    assert_eq!(patch.encode.unwrap().ctcss_tenths_hz, 1148);
    assert!(patch.decode.unwrap().inverted);
    raw.flags = 0;
    assert_eq!(raw.patch(2).unwrap().transmit_hz, Some(None));
    let plain = raw.resolve().unwrap();
    assert!(plain.transmit_hz.is_none() && plain.channel.is_none() && plain.power_mw.is_none());
    assert!(!plain.split && !plain.rit_enabled && !plain.xit_enabled);
    assert_eq!(State::from(&plain).resolve(), Ok(plain));
    for fields in [0, 65536] {
        assert_eq!(raw.patch(fields), Err(Error::InvalidConfiguration));
    }
    raw.flags = 64;
    assert_eq!(raw.resolve(), Err(Error::InvalidConfiguration));
    assert_eq!(raw.patch(1), Err(Error::InvalidConfiguration));
    raw.flags = CHANNEL;
    raw.channel = 256;
    assert_eq!(raw.resolve(), Err(Error::InvalidConfiguration));
    assert_eq!(raw.patch(4096), Err(Error::InvalidConfiguration));
    raw.flags = 0;
    raw.mode = 99;
    assert_eq!(raw.patch(4), Err(Error::Mode));
    assert!(raw.patch(1).is_ok());
    for field in [1024, 2048] {
        for bad in [0, 1, 2, 3] {
            let signal = if field == 1024 {
                &mut raw.encode
            } else {
                &mut raw.decode
            };
            *signal = Signal {
                source: 0,
                control_source: 0,
                enabled: 0,
                ctcss_tenths_hz: 0,
                dcs_code: 0,
                inverted: 0,
            };
            match bad {
                0 => signal.source = 4,
                1 => signal.control_source = 4,
                2 => signal.enabled = 2,
                _ => signal.inverted = 2,
            }
            assert_eq!(raw.patch(field), Err(Error::Signaling));
        }
    }
}

#[test]
fn c_descriptor_copies_memory_and_rejects_incomplete_setup_without_publishing_handles() {
    let api = unsafe { &*rptadv_remote_api_v1() };
    assert_eq!(api.abi_version, 1);
    assert_eq!(api.struct_size as usize, size_of::<Api>());
    for defect in 0..17 {
        with_setup(|setup| {
            match defect {
                0 => setup.struct_size -= 1,
                1 => setup.abi_version = 2,
                2 => setup.limits.flags = 1,
                3 => setup.capabilities.flags = 1,
                4 => setup.limits.receive.data = ptr::null(),
                5 => setup.limits.transmit.data = ptr::null(),
                6 => {
                    setup.limits.receive_only = Span {
                        data: ptr::null(),
                        length: 1,
                    }
                }
                7 => setup.capabilities.receive.data = ptr::null(),
                8 => setup.capabilities.transmit.data = ptr::null(),
                9 => setup.memories.data = ptr::null(),
                10 => setup.scan_lists.data = ptr::null(),
                11 => setup.limits.mode_mask = 1,
                12 => setup.capabilities.mode_mask = 1,
                13 => setup.capabilities.scan_mode_mask = 1,
                14 => setup.capabilities.encode_sources = 16,
                15 => setup.capabilities.decode_sources = 16,
                _ => setup.capabilities.decode_control_sources = 16,
            }
            let mut output = ptr::without_provenance_mut(1);
            assert!(create(setup, &mut output) < 0, "defect {defect}");
            assert!(output.is_null());
        });
    }
    with_setup(|setup| {
        assert_eq!(create(setup, ptr::null_mut()), INVALID);
        assert_eq!(create(setup, ptr::without_provenance_mut(1)), INVALID);
        let mut out = ptr::null_mut();
        assert_eq!(create(ptr::null(), &mut out), INVALID);
        assert_eq!(create(ptr::without_provenance(1), &mut out), INVALID);
    });
    with_owner(|handle| {
        let mut output = state();
        assert_eq!(configured_memory(handle, 1, &mut output), 0);
        assert_eq!(output.receive_hz, 146_520_000);
        assert_eq!(output.transmit_hz, 146_520_000);
        assert_eq!(output.power_mw, 5000);
        assert_eq!(configured_memory(handle, 2, &mut output), -7);
        for bad in [ptr::null(), ptr::without_provenance(1)] {
            assert_eq!(configured_memory(bad, 1, &mut output), INVALID);
        }
        for bad in [ptr::null_mut(), ptr::without_provenance_mut(1)] {
            assert_eq!(configured_memory(handle, 1, bad), INVALID);
        }
    });
    destroy(ptr::null_mut());
}
