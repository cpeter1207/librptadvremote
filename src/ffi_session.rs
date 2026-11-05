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
}

#[repr(C)]
pub(super) struct RawCompletion {
    request_id: u64,
    result: i32,
}

struct ForeignBackend<'a> {
    operations: &'a Backend,
    capabilities: Capabilities,
}

impl RigBackend for ForeignBackend<'_> {
    fn capabilities(&self) -> Capabilities {
        self.capabilities.clone()
    }

    fn read_state(&mut self) -> Result<OperatingState, i32> {
        let read = self.operations.read_state.ok_or(Error::Backend as i32)?;
        let mut output = State::from(&OperatingState::default());
        // SAFETY: the validated callback borrows this complete writable record only for return.
        let result = unsafe { read(self.operations.context, &mut output) };
        if result != 0 {
            return Err(result);
        }
        output.resolve().map_err(|error| error as i32)
    }

    fn apply_step(&mut self, state: &OperatingState) -> Result<(), i32> {
        let apply = self.operations.apply_state.ok_or(Error::Backend as i32)?;
        let state = State::from(state);
        // SAFETY: the callback may read the complete copied state through return, without reentry.
        let result = unsafe { apply(self.operations.context, &state) };
        if result == 0 { Ok(()) } else { Err(result) }
    }

    fn set_ptt(&mut self, keyed: bool) -> Result<(), i32> {
        let set = self.operations.set_ptt.ok_or(Error::Backend as i32)?;
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
        if activity.flags & !31 != 0
            || operations.abi_version != 1
            || operations.apply_state.is_none()
            || operations.set_ptt.is_none()
            || owner.capabilities().readback_fields != 0 && operations.read_state.is_none()
        {
            return Err(Error::InvalidConfiguration);
        }
        let mut backend = ForeignBackend {
            operations,
            capabilities: owner.capabilities().clone(),
        };
        owner.advance(
            now_ms,
            Activity {
                local_receiving: activity.flags & 1 != 0,
                linked_receiving: activity.flags & 2 != 0,
                command_source_active: activity.flags & 4 != 0,
                physical_ptt: activity.flags & 8 != 0,
                transmit_requested: activity.flags & 16 != 0,
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
                | flag(value.ptt_applied == Some(true), 256),
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
