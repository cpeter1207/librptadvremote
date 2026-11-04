//! Sized C configuration records; borrowed input is copied before returning.

use crate::*;
use std::{
    ffi::{c_char, c_void},
    mem::size_of,
    panic::{AssertUnwindSafe, catch_unwind},
};

const TX: u32 = 1;
const POWER: u32 = 2;
const CHANNEL: u32 = 4;
const READBACK: u32 = 8;
const INVALID: i32 = Error::InvalidConfiguration as i32;

#[repr(C)]
struct Span<T> {
    data: *const T,
    length: usize,
}

impl<T> Span<T> {
    unsafe fn read(&self) -> Result<&[T], Error> {
        if self.length == 0 {
            return Ok(&[]);
        }
        if self.data.is_null()
            || !self.data.is_aligned()
            || self.length > isize::MAX as usize / size_of::<T>()
        {
            return Err(Error::InvalidConfiguration);
        }
        // SAFETY: the caller supplies readable storage for the checked extent.
        Ok(unsafe { std::slice::from_raw_parts(self.data, self.length) })
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Signal {
    source: u32,
    enabled: u32,
    ctcss_tenths_hz: u32,
    dcs_code: u32,
    inverted: u32,
}

impl Signal {
    fn resolve(self) -> Result<Signaling, Error> {
        if self.enabled > 1 || self.inverted > 1 {
            return Err(Error::Signaling);
        }
        Ok(Signaling {
            source: source(self.source)?,
            enabled: self.enabled != 0,
            ctcss_tenths_hz: self.ctcss_tenths_hz,
            dcs_code: self.dcs_code,
            inverted: self.inverted != 0,
        })
    }
}

#[repr(C)]
pub(super) struct State {
    receive_hz: u64,
    transmit_hz: u64,
    offset_hz: i64,
    rit_hz: i64,
    xit_hz: i64,
    flags: u32,
    mode: u32,
    bandwidth_hz: u32,
    receive_vfo: u32,
    transmit_vfo: u32,
    power_mw: u32,
    channel: u32,
    encode: Signal,
    decode: Signal,
}

impl State {
    pub(super) fn resolve(&self) -> Result<OperatingState, Error> {
        if self.flags & !(TX | POWER | CHANNEL) != 0
            || self.flags & CHANNEL != 0 && self.channel > u32::from(u8::MAX)
        {
            return Err(Error::InvalidConfiguration);
        }
        Ok(OperatingState {
            receive_hz: self.receive_hz,
            transmit_hz: (self.flags & TX != 0).then_some(self.transmit_hz),
            offset_hz: self.offset_hz,
            rit_hz: self.rit_hz,
            xit_hz: self.xit_hz,
            mode: mode(self.mode)?,
            bandwidth_hz: self.bandwidth_hz,
            receive_vfo: self.receive_vfo,
            transmit_vfo: self.transmit_vfo,
            power_mw: (self.flags & POWER != 0).then_some(self.power_mw),
            channel: (self.flags & CHANNEL != 0).then_some(self.channel as u8),
            encode: self.encode.resolve()?,
            decode: self.decode.resolve()?,
        })
    }

    pub(super) fn patch(&self, fields: u64) -> Result<OperatingPatch, Error> {
        if fields == 0 || fields & !8191 != 0 || self.flags & !(TX | POWER | CHANNEL) != 0 {
            return Err(Error::InvalidConfiguration);
        }
        let mut patch = OperatingPatch::default();
        macro_rules! selected {
            ($bit:literal, $field:ident) => {
                if fields & (1 << $bit) != 0 {
                    patch.$field = Some(self.$field);
                }
            };
        }
        selected!(0, receive_hz);
        if fields & 2 != 0 {
            patch.transmit_hz = Some((self.flags & TX != 0).then_some(self.transmit_hz));
        }
        if fields & 4 != 0 {
            patch.mode = Some(mode(self.mode)?);
        }
        selected!(3, bandwidth_hz);
        selected!(4, receive_vfo);
        selected!(5, transmit_vfo);
        selected!(6, offset_hz);
        selected!(7, rit_hz);
        selected!(8, xit_hz);
        selected!(9, power_mw);
        if fields & (1 << 10) != 0 {
            patch.encode = Some(self.encode.resolve()?);
        }
        if fields & (1 << 11) != 0 {
            patch.decode = Some(self.decode.resolve()?);
        }
        if fields & (1 << 12) != 0 {
            patch.channel =
                Some(u8::try_from(self.channel).map_err(|_| Error::InvalidConfiguration)?);
        }
        Ok(patch)
    }
}

impl From<&OperatingState> for State {
    fn from(state: &OperatingState) -> Self {
        let signal = |value: Signaling| Signal {
            source: value.source as u32,
            enabled: u32::from(value.enabled),
            ctcss_tenths_hz: value.ctcss_tenths_hz,
            dcs_code: value.dcs_code,
            inverted: u32::from(value.inverted),
        };
        Self {
            receive_hz: state.receive_hz,
            transmit_hz: state.transmit_hz.unwrap_or(0),
            offset_hz: state.offset_hz,
            rit_hz: state.rit_hz,
            xit_hz: state.xit_hz,
            flags: (u32::from(state.transmit_hz.is_some()) * TX)
                | (u32::from(state.power_mw.is_some()) * POWER)
                | (u32::from(state.channel.is_some()) * CHANNEL),
            mode: state.mode as u32,
            bandwidth_hz: state.bandwidth_hz,
            receive_vfo: state.receive_vfo,
            transmit_vfo: state.transmit_vfo,
            power_mw: state.power_mw.unwrap_or(0),
            channel: u32::from(state.channel.unwrap_or(0)),
            encode: signal(state.encode),
            decode: signal(state.decode),
        }
    }
}

#[repr(C)]
struct RawLimits {
    receive: Span<FrequencyRange>,
    transmit: Span<FrequencyRange>,
    receive_only: Span<FrequencyRange>,
    mode_mask: u64,
    flags: u32,
    maximum_power_mw: u32,
}
#[repr(C)]
struct RawCapabilities {
    receive: Span<FrequencyRange>,
    transmit: Span<FrequencyRange>,
    mode_mask: u64,
    flags: u32,
    encode_sources: u32,
    decode_sources: u32,
}
#[repr(C)]
struct RawMemory {
    selector: u32,
    name: *const c_char,
    name_length: usize,
    state: State,
}
#[repr(C)]
struct RawScan {
    selector: u32,
    name: *const c_char,
    name_length: usize,
    members: *const u32,
    member_count: usize,
    dwell_ms: u64,
    resume_delay_ms: u64,
    settle_ms: u64,
    priority: u32,
}
#[repr(C)]
struct Setup {
    struct_size: u32,
    abi_version: u32,
    limits: RawLimits,
    capabilities: RawCapabilities,
    memories: Span<RawMemory>,
    scan_lists: Span<RawScan>,
    startup_memory: u32,
}

unsafe fn text(data: *const c_char, length: usize) -> Result<String, Error> {
    let bytes = Span {
        data: data.cast::<u8>(),
        length,
    };
    // SAFETY: the C caller provides the readable UTF-8 name extent.
    std::str::from_utf8(unsafe { bytes.read()? })
        .map(str::to_owned)
        .map_err(|_| Error::InvalidConfiguration)
}

fn mode(value: u32) -> Result<Mode, Error> {
    match value {
        1 => Ok(Mode::Fm),
        2 => Ok(Mode::Am),
        3 => Ok(Mode::Usb),
        4 => Ok(Mode::Lsb),
        5 => Ok(Mode::Cw),
        6 => Ok(Mode::CwReverse),
        7 => Ok(Mode::Rtty),
        8 => Ok(Mode::RttyReverse),
        9 => Ok(Mode::WideFm),
        10 => Ok(Mode::PacketFm),
        11 => Ok(Mode::PacketUsb),
        12 => Ok(Mode::PacketLsb),
        _ => Err(Error::Mode),
    }
}

fn modes(mask: u64) -> Result<Vec<Mode>, Error> {
    if mask & !0x1ffe != 0 {
        return Err(Error::Mode);
    }
    (1..=12)
        .filter(|bit| mask & (1_u64 << bit) != 0)
        .map(mode)
        .collect()
}

fn source(value: u32) -> Result<SignalSource, Error> {
    match value {
        0 => Ok(SignalSource::Disabled),
        1 => Ok(SignalSource::Dsp),
        2 => Ok(SignalSource::External),
        3 => Ok(SignalSource::Rig),
        _ => Err(Error::Signaling),
    }
}

fn sources(mask: u32) -> Result<Vec<SignalSource>, Error> {
    if mask & !15 != 0 {
        return Err(Error::Signaling);
    }
    (0..=3)
        .filter(|bit| mask & (1_u32 << bit) != 0)
        .map(source)
        .collect()
}

unsafe fn resolve(setup: &Setup) -> Result<RemoteBase, Error> {
    let limits = &setup.limits;
    let caps = &setup.capabilities;
    if limits.flags & !POWER != 0 || caps.flags & !(POWER | READBACK) != 0 {
        return Err(Error::InvalidConfiguration);
    }
    // SAFETY: setup spans are borrowed for this call and copied below.
    let (config, capabilities) = unsafe {
        let memories = setup
            .memories
            .read()?
            .iter()
            .map(|memory| {
                Ok(Memory {
                    selector: memory.selector,
                    name: text(memory.name, memory.name_length)?,
                    state: memory.state.resolve()?,
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let scan_lists = setup
            .scan_lists
            .read()?
            .iter()
            .map(|list| {
                Ok(ScanList {
                    selector: list.selector,
                    name: text(list.name, list.name_length)?,
                    members: Span {
                        data: list.members,
                        length: list.member_count,
                    }
                    .read()?
                    .to_vec(),
                    dwell_ms: list.dwell_ms,
                    resume_delay_ms: list.resume_delay_ms,
                    settle_ms: list.settle_ms,
                    priority: (list.priority != 0).then_some(list.priority),
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        (
            RemoteBaseConfig {
                limits: Limits {
                    receive: limits.receive.read()?.to_vec(),
                    transmit: limits.transmit.read()?.to_vec(),
                    receive_only: limits.receive_only.read()?.to_vec(),
                    modes: modes(limits.mode_mask)?,
                    maximum_power_mw: (limits.flags & POWER != 0)
                        .then_some(limits.maximum_power_mw),
                },
                memories,
                scan_lists,
                startup_memory: setup.startup_memory,
            },
            Capabilities {
                receive: caps.receive.read()?.to_vec(),
                transmit: caps.transmit.read()?.to_vec(),
                modes: modes(caps.mode_mask)?,
                power_mw: caps.flags & POWER != 0,
                readback: caps.flags & READBACK != 0,
                encode_sources: sources(caps.encode_sources)?,
                decode_sources: sources(caps.decode_sources)?,
            },
        )
    };
    RemoteBase::new(config, capabilities)
}

extern "C" fn create(setup: *const Setup, out: *mut *mut c_void) -> i32 {
    catch_unwind(AssertUnwindSafe(|| {
        if out.is_null() || !out.is_aligned() {
            return INVALID;
        }
        // SAFETY: callers supply aligned writable out storage when non-null.
        let Some(out) = (unsafe { out.as_mut() }) else {
            return INVALID;
        };
        *out = std::ptr::null_mut();
        if setup.is_null() || !setup.is_aligned() {
            return INVALID;
        }
        // SAFETY: the ABI prefix is readable even when its declared size is wrong.
        if unsafe { std::ptr::addr_of!((*setup).struct_size).read() } != size_of::<Setup>() as u32
            || unsafe { std::ptr::addr_of!((*setup).abi_version).read() } != 1
        {
            return INVALID;
        }
        // SAFETY: the caller supplies the complete setup after exact size admission.
        match unsafe { resolve(&*setup) } {
            Ok(owner) => {
                *out = Box::into_raw(Box::new(owner)).cast();
                0
            }
            Err(error) => error as i32,
        }
    }))
    .unwrap_or(INVALID)
}

extern "C" fn destroy(handle: *mut c_void) {
    if !handle.is_null() {
        // SAFETY: the caller transfers one live handle after all uses have stopped.
        unsafe {
            drop(Box::from_raw(handle.cast::<RemoteBase>()));
        }
    }
}

extern "C" fn configured_memory(handle: *const c_void, selector: u32, out: *mut State) -> i32 {
    if handle.is_null()
        || !handle.cast::<RemoteBase>().is_aligned()
        || out.is_null()
        || !out.is_aligned()
    {
        return INVALID;
    }
    // SAFETY: the caller retains the live owner and exclusively lends writable output.
    let owner = unsafe { &*handle.cast::<RemoteBase>() };
    match owner.configuration().memory(selector) {
        Ok(memory) => {
            unsafe {
                out.write(State::from(&memory.state));
            }
            0
        }
        Err(error) => error as i32,
    }
}

/// Immutable descriptor prefix shared by installed C and Rust clients.
#[repr(C)]
pub struct Api {
    struct_size: u32,
    abi_version: u32,
    capability_name: *const c_char,
    create: extern "C" fn(*const Setup, *mut *mut c_void) -> i32,
    destroy: extern "C" fn(*mut c_void),
    configured_memory: extern "C" fn(*const c_void, u32, *mut State) -> i32,
    request: extern "C" fn(*mut c_void, *const crate::ffi_session::Request, u64, *mut u64) -> i32,
    advance: extern "C" fn(
        *mut c_void,
        u64,
        *const crate::ffi_session::RawActivity,
        *const crate::ffi_session::Backend,
    ) -> i32,
    snapshot: extern "C" fn(*const c_void, *mut crate::ffi_session::Snapshot) -> i32,
    take_completion: extern "C" fn(*mut c_void, *mut crate::ffi_session::RawCompletion) -> i32,
    retire: extern "C" fn(*mut c_void) -> i32,
}

// SAFETY: all descriptor fields and the pointed-to identity are immutable static data.
unsafe impl Sync for Api {}

static API: Api = Api {
    struct_size: size_of::<Api>() as u32,
    abi_version: 1,
    capability_name: c"rptadv.remote".as_ptr(),
    create,
    destroy,
    configured_memory,
    request: crate::ffi_session::request,
    advance: crate::ffi_session::advance,
    snapshot: crate::ffi_session::snapshot,
    take_completion: crate::ffi_session::take_completion,
    retire: crate::ffi_session::retire,
};

/// Return the initial shared remote-base descriptor; its handle methods are control-only.
#[unsafe(no_mangle)]
pub extern "C" fn rptadv_remote_api_v1() -> *const Api {
    &API
}
