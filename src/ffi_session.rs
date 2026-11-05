//! Control-only C bridge to the same portable session used by Rust tests.

use crate::{ffi::State, *};
use std::{
    ffi::c_void,
    mem::size_of,
    panic::{AssertUnwindSafe, catch_unwind},
};

#[repr(C)]
pub(super) struct Request {
    struct_size: u32,
    abi_version: u32,
    kind: u32,
    selector: u32,
    fields: u64,
    state: State,
    value_ms: u64,
    direction: u32,
}

#[repr(C)]
pub(super) struct RawActivity {
    struct_size: u32,
    flags: u32,
}

#[repr(C)]
pub(super) struct Backend {
    struct_size: u32,
    abi_version: u32,
    context: *mut c_void,
    read_state: Option<unsafe extern "C" fn(*mut c_void, *mut State) -> i32>,
    apply_state: Option<unsafe extern "C" fn(*mut c_void, *const State) -> i32>,
    set_ptt: Option<unsafe extern "C" fn(*mut c_void, u32) -> i32>,
    monotonic_ms: Option<unsafe extern "C" fn(*mut c_void) -> u64>,
    transmit_epoch: Option<unsafe extern "C" fn(*mut c_void) -> u64>,
    set_tune: Option<unsafe extern "C" fn(*mut c_void, u32) -> i32>,
}

#[repr(C)]
pub(super) struct Snapshot {
    struct_size: u32,
    flags: u32,
    requested: State,
    applied: State,
    observed: State,
    failure_code: i32,
    observed_fields: u64,
    scan_rejected_bursts: u64,
}

#[repr(C)]
pub(super) struct RawCompletion {
    request_id: u64,
    result: i32,
}

#[repr(C)]
pub(super) struct RawScanSnapshot {
    struct_size: u32,
    flags: u32,
    list: u32,
    current_memory: u32,
    direction: u32,
    failed_memory: u32,
    last_error: i32,
    priority: u32,
    next_deadline_ms: u64,
    dwell_ms: u64,
    resume_delay_ms: u64,
    skipped: usize,
}

struct ForeignBackend<'a> {
    operations: &'a Backend,
}

impl RigBackend for ForeignBackend<'_> {
    fn set_tune(&mut self, enabled: bool) -> Result<(), i32> {
        let call = self.operations.set_tune.expect("admitted tune callback");
        // SAFETY: the selected physical owner retains callback/context through advance.
        let code = unsafe { call(self.operations.context, u32::from(enabled)) };
        if code == 0 { Ok(()) } else { Err(code) }
    }
    fn monotonic_ms(&self) -> u64 {
        self.operations
            .monotonic_ms
            .map_or(0, |read| unsafe { read(self.operations.context) })
    }
    fn transmit_epoch(&self) -> u64 {
        self.operations
            .transmit_epoch
            .map_or(0, |read| unsafe { read(self.operations.context) })
    }
    fn read_state(&mut self) -> Result<OperatingState, i32> {
        let read = self
            .operations
            .read_state
            .expect("admitted readback callback");
        let mut output = State::from(&OperatingState::default());
        // SAFETY: the validated callback borrows this complete writable record only for return.
        let result = unsafe { read(self.operations.context, &mut output) };
        if result != 0 {
            return Err(result);
        }
        output.resolve().map_err(|error| error as i32)
    }

    fn apply_step(&mut self, state: &OperatingState) -> Result<(), i32> {
        let apply = self
            .operations
            .apply_state
            .expect("admitted apply callback");
        let state = State::from(state);
        // SAFETY: the callback may read the complete copied state through return, without reentry.
        let result = unsafe { apply(self.operations.context, &state) };
        if result == 0 { Ok(()) } else { Err(result) }
    }

    fn set_ptt(&mut self, keyed: bool) -> Result<(), i32> {
        let set = self.operations.set_ptt.expect("admitted PTT callback");
        // SAFETY: the selected physical owner retains the callback and opaque context.
        let result = unsafe { set(self.operations.context, u32::from(keyed)) };
        if result == 0 { Ok(()) } else { Err(result) }
    }
}

fn guard(call: impl FnOnce() -> Result<i32, Error>) -> i32 {
    catch_unwind(AssertUnwindSafe(call))
        .unwrap_or(Err(Error::InvalidConfiguration))
        .unwrap_or_else(|error| error as i32)
}

unsafe fn owner<'a>(handle: *mut c_void) -> Result<&'a mut RemoteBase, Error> {
    let pointer = handle.cast::<RemoteBase>();
    if pointer.is_null() || !pointer.is_aligned() {
        return Err(Error::InvalidConfiguration);
    }
    // SAFETY: the caller serializes a live context and prohibits callback reentry.
    Ok(unsafe { &mut *pointer })
}

unsafe fn sized<'a, T>(pointer: *const T) -> Result<&'a T, Error> {
    if pointer.is_null() || !pointer.is_aligned() {
        return Err(Error::InvalidConfiguration);
    }
    // SAFETY: every sized record begins with a readable u32 size prefix.
    if unsafe { pointer.cast::<u32>().read() } != size_of::<T>() as u32 {
        return Err(Error::InvalidConfiguration);
    }
    // SAFETY: the caller supplies the complete record admitted by its size.
    Ok(unsafe { &*pointer })
}

pub(super) extern "C" fn request(
    handle: *mut c_void,
    input: *const Request,
    now_ms: u64,
    output: *mut u64,
) -> i32 {
    guard(|| {
        if output.is_null() || !output.is_aligned() {
            return Err(Error::InvalidConfiguration);
        }
        // SAFETY: the caller exclusively lends aligned output storage through return.
        unsafe {
            output.write(0);
        }
        // SAFETY: input is a readable sized request and the live context is serialized.
        let (input, owner) = unsafe { (sized(input)?, owner(handle)?) };
        if input.abi_version != 1 || input.kind != 1 && input.fields != 0 {
            return Err(Error::InvalidConfiguration);
        }
        let request = match input.kind {
            1 => RigRequest::Set(input.state.patch(input.fields)?),
            2 => RigRequest::Recall(input.selector),
            3 if input.selector <= 1 => RigRequest::Inhibit(input.selector != 0),
            4 => RigRequest::Scan(ScanRequest::Start {
                selector: input.selector,
                direction: direction(input.direction)?,
                dwell_ms: (input.value_ms != 0).then_some(input.value_ms),
            }),
            5 => RigRequest::Scan(ScanRequest::Pause),
            6 => RigRequest::Scan(ScanRequest::Resume),
            7 => RigRequest::Scan(ScanRequest::Stop),
            8 => RigRequest::Scan(ScanRequest::Direction(direction(input.direction)?)),
            9 => RigRequest::Scan(ScanRequest::Skip),
            10 => RigRequest::Scan(ScanRequest::SetDwell(input.value_ms)),
            11 => RigRequest::Scan(ScanRequest::SetResumeDelay(input.value_ms)),
            12 => RigRequest::Scan(ScanRequest::SetPriority(
                (input.selector != 0).then_some(input.selector),
            )),
            13 => RigRequest::Tune {
                duration_ms: input.value_ms,
            },
            _ => return Err(Error::InvalidConfiguration),
        };
        let id = owner.request(request, now_ms)?;
        // SAFETY: output remains writable for the same admitted call.
        unsafe {
            output.write(id.0);
        }
        Ok(0)
    })
}

fn direction(value: u32) -> Result<ScanDirection, Error> {
    match value {
        0 => Ok(ScanDirection::Down),
        1 => Ok(ScanDirection::Up),
        _ => Err(Error::InvalidConfiguration),
    }
}

pub(super) extern "C" fn scan_snapshot(handle: *const c_void, out: *mut RawScanSnapshot) -> i32 {
    guard(|| {
        // SAFETY: caller serializes the live owner and lends a sized writable output.
        unsafe {
            sized(out.cast_const())?;
        }
        let Some(scan) = unsafe { owner(handle.cast_mut())? }.scan_snapshot() else {
            return Ok(1);
        };
        let flags = u32::from(scan.active)
            | (u32::from(scan.paused) << 1)
            | (u32::from(scan.activity_hold) << 2)
            | (u32::from(scan.next_deadline_ms.is_some()) << 3)
            | (u32::from(scan.last_error.is_some()) << 4);
        // SAFETY: same admitted output record is exclusively writable through return.
        unsafe {
            out.write(RawScanSnapshot {
                struct_size: size_of::<RawScanSnapshot>() as u32,
                flags,
                list: scan.list,
                current_memory: scan.current_memory.unwrap_or(0),
                direction: scan.direction as u32,
                failed_memory: scan.failed_memory.unwrap_or(0),
                last_error: scan.last_error.unwrap_or(0),
                priority: scan.priority.unwrap_or(0),
                next_deadline_ms: scan.next_deadline_ms.unwrap_or(0),
                dwell_ms: scan.dwell_ms,
                resume_delay_ms: scan.resume_delay_ms,
                skipped: scan.skipped,
            });
        }
        Ok(0)
    })
}

pub(super) extern "C" fn advance(
    handle: *mut c_void,
    now_ms: u64,
    activity: *const RawActivity,
    operations: *const Backend,
) -> i32 {
    guard(|| {
        // SAFETY: the caller lends complete input records and the serialized live owner.
        let (activity, operations, owner) =
            unsafe { (sized(activity)?, sized(operations)?, owner(handle)?) };
        if activity.flags & !127 != 0
            || operations.abi_version != 1
            || operations.apply_state.is_none()
            || operations.set_ptt.is_none()
            || owner.capabilities().readback_fields != 0 && operations.read_state.is_none()
            || owner.capabilities().manual_tune && operations.set_tune.is_none()
            || !owner.configuration().scan_lists.is_empty()
                && (operations.monotonic_ms.is_none() || operations.transmit_epoch.is_none())
        {
            return Err(Error::InvalidConfiguration);
        }
        let mut backend = ForeignBackend { operations };
        owner.advance(
            now_ms,
            Activity {
                local_receiving: activity.flags & 1 != 0,
                linked_receiving: activity.flags & 2 != 0,
                command_source_active: activity.flags & 4 != 0,
                command_cancelled: activity.flags & 32 != 0,
                physical_ptt: activity.flags & 8 != 0,
                transmit_requested: activity.flags & 16 != 0,
                rendered_ptt: activity.flags & 64 != 0,
            },
            &mut backend,
        )?;
        Ok(0)
    })
}

pub(super) extern "C" fn snapshot(handle: *const c_void, output: *mut Snapshot) -> i32 {
    guard(|| {
        // SAFETY: the caller serializes all access and provides the exact writable snapshot.
        unsafe {
            sized(output.cast_const())?;
        }
        let value = unsafe { owner(handle.cast_mut())? }.snapshot();
        let flag = |enabled, mask| if enabled { mask } else { 0 };
        let empty = || State::from(&OperatingState::default());
        let result = Snapshot {
            struct_size: size_of::<Snapshot>() as u32,
            flags: flag(value.applied.is_some(), 1)
                | flag(value.observed.is_some(), 2)
                | flag(value.verified, 4)
                | flag(value.receive_permitted, 8)
                | flag(value.transmit_inhibited, 16)
                | flag(value.pending, 32)
                | flag(value.failure.is_some(), 64)
                | flag(value.ptt_applied.is_some(), 128)
                | flag(value.ptt_applied == Some(true), 256)
                | flag(value.manual_tune_active, 512),
            requested: State::from(&value.requested),
            applied: value
                .applied
                .as_ref()
                .map(State::from)
                .unwrap_or_else(empty),
            observed: value
                .observed
                .as_ref()
                .map(State::from)
                .unwrap_or_else(empty),
            failure_code: value.failure.map_or(0, |failure| failure.code),
            observed_fields: value.observed_fields,
            scan_rejected_bursts: value.scan_rejected_bursts,
        };
        // SAFETY: the same admitted output span is exclusively writable through return.
        unsafe {
            output.write(result);
        }
        Ok(0)
    })
}

pub(super) extern "C" fn take_completion(handle: *mut c_void, output: *mut RawCompletion) -> i32 {
    guard(|| {
        if output.is_null() || !output.is_aligned() {
            return Err(Error::InvalidConfiguration);
        }
        // SAFETY: the caller retains and serializes the context.
        let completion = unsafe { owner(handle)? }.take_completion();
        match completion {
            Some(value) => {
                // SAFETY: the caller supplies a complete exclusively writable result.
                unsafe {
                    output.write(RawCompletion {
                        request_id: value.id.0,
                        result: value.result.map_or_else(|error| error as i32, |()| 0),
                    });
                }
                Ok(0)
            }
            None => Ok(1),
        }
    })
}

pub(super) extern "C" fn retire(handle: *mut c_void) -> i32 {
    guard(|| {
        // SAFETY: retirement is serialized with all uses of the live context.
        unsafe { owner(handle)? }.retire();
        Ok(0)
    })
}

#[cfg(test)]
#[cfg_attr(coverage, coverage(off))]
#[path = "ffi_session_tests.rs"]
mod tests;
