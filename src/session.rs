//! Serialized remote-base tune and transmit admission on the physical owner.

use crate::{Capabilities, Error, OperatingState, RemoteBaseConfig};

/// Physical calls supplied by the selected dependency adapter and GPIO owner.
pub trait RigBackend {
    /// Actual operations and signal providers available during setup.
    fn capabilities(&self) -> Capabilities;
    /// Read actual supported fields; only `Capabilities::readback_fields` are
    /// observations. Other record fields are ignored, never fabricated readback.
    fn read_state(&mut self) -> Result<OperatingState, i32>;
    /// Apply a prepared complete state; failure may leave partial physical changes.
    fn apply_step(&mut self, state: &OperatingState) -> Result<(), i32>;
    /// Apply PTT through its one explicitly selected provider.
    fn set_ptt(&mut self, keyed: bool) -> Result<(), i32>;
}

/// Native radio activity copied from the current generation's published state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Activity {
    /// Qualified local receive activity; open non-FM receive does not block explicit tune.
    pub local_receiving: bool,
    /// Activity from a linked source.
    pub linked_receiving: bool,
    /// The originating DTMF source is still keyed.
    pub command_source_active: bool,
    /// Observed physical PTT, distinct from an intent or queued operation.
    pub physical_ptt: bool,
    /// Current ordinary transmit demand, independent of receive qualification.
    pub transmit_requested: bool,
}

/// Typed partial change applied against the complete current desired state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OperatingPatch {
    /// New receive frequency.
    pub receive_hz: Option<u64>,
    /// New transmit frequency, or an explicit receive-only selection.
    pub transmit_hz: Option<Option<u64>>,
    /// New mode.
    pub mode: Option<crate::Mode>,
    /// New passband.
    pub bandwidth_hz: Option<u32>,
    /// New receive VFO.
    pub receive_vfo: Option<u32>,
    /// New transmit VFO.
    pub transmit_vfo: Option<u32>,
    /// New repeater shift.
    pub offset_hz: Option<i64>,
    /// New receive incremental tuning.
    pub rit_hz: Option<i64>,
    /// New transmit incremental tuning.
    pub xit_hz: Option<i64>,
    /// New established transmit power.
    pub power_mw: Option<u32>,
    /// New independently selected encode state.
    pub encode: Option<crate::Signaling>,
    /// New independently selected decode state.
    pub decode: Option<crate::Signaling>,
    /// New parallel channel.
    pub channel: Option<u8>,
}

impl OperatingPatch {
    fn apply(&self, mut state: OperatingState) -> OperatingState {
        macro_rules! replace {
            ($($field:ident),* $(,)?) => { $(if let Some(value) = self.$field { state.$field = value; })* };
        }
        replace!(
            receive_hz,
            transmit_hz,
            mode,
            bandwidth_hz,
            receive_vfo,
            transmit_vfo,
            offset_hz,
            rit_hz,
            xit_hz,
            encode,
            decode
        );
        if let Some(value) = self.power_mw {
            state.power_mw = Some(value);
        }
        if let Some(value) = self.channel {
            state.channel = Some(value);
        }
        state
    }
}

/// Portable operating request; definitions remain configuration-only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RigRequest {
    /// Validate and apply a partial live change.
    Set(OperatingPatch),
    /// Recall a configured complete memory.
    Recall(u32),
    /// Change only administrative inhibit; a safety fault remains independent.
    Inhibit(bool),
}

/// Context-local monotonically allocated operation identity; zero is startup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequestId(pub u64);

/// One returned completion; ordinary work and urgent inhibit each have a reserved slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completion {
    /// Original admitted request.
    pub id: RequestId,
    /// Final application result, not merely queue acceptance.
    pub result: Result<(), Error>,
}

/// First backend failure; recovery attempts do not overwrite its code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Failure {
    /// Dependency-adapter status code.
    pub code: i32,
}

/// Copied status with configuration, application, and observation distinguished.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RigSnapshot {
    /// Complete desired state.
    pub requested: OperatingState,
    /// Most recent successfully applied state, if any.
    pub applied: Option<OperatingState>,
    /// Last actual supported readback, if any.
    pub observed: Option<OperatingState>,
    /// Fields actually present in the observation, using operating-patch bits.
    pub observed_fields: u64,
    /// Readback confirmed the current applied state.
    pub verified: bool,
    /// Receive routing is safe under the observed/current policy.
    pub receive_permitted: bool,
    /// Administrative, safety, receive-only, or retired state denies new PTT.
    pub transmit_inhibited: bool,
    /// First backend failure since the last successful state application.
    pub failure: Option<Failure>,
    /// An admitted tune still waits for safe application.
    pub pending: bool,
    /// Last successfully serviced PTT command, not a physical RF observation.
    pub ptt_applied: Option<bool>,
}

struct Pending {
    id: RequestId,
    state: OperatingState,
}

/// One radio's portable policy; its host serializes every call outside audio.
pub struct RemoteBase {
    config: RemoteBaseConfig,
    capabilities: Capabilities,
    snapshot: RigSnapshot,
    pending: Option<Pending>,
    completion: Option<Completion>,
    critical_completion: Option<Completion>,
    next_id: u64,
    administrative_inhibit: bool,
    safe: bool,
    retired: bool,
    next_poll_ms: u64,
}

impl RemoteBase {
    /// Validate all configuration and prepare a safe, unkeyed startup request.
    pub fn new(config: RemoteBaseConfig, capabilities: Capabilities) -> Result<Self, Error> {
        config.validate(&capabilities)?;
        let state = config.memory(config.startup_memory)?.state.clone();
        Ok(Self {
            config,
            capabilities,
            pending: Some(Pending {
                id: RequestId(0),
                state: state.clone(),
            }),
            snapshot: RigSnapshot {
                requested: state,
                applied: None,
                observed: None,
                observed_fields: 0,
                verified: false,
                receive_permitted: false,
                transmit_inhibited: true,
                failure: None,
                pending: true,
                ptt_applied: None,
            },
            completion: None,
            critical_completion: None,
            next_id: 1,
            administrative_inhibit: false,
            safe: false,
            retired: false,
            next_poll_ms: 0,
        })
    }

    /// Return immutable configured definitions, never a live persistence store.
    pub fn configuration(&self) -> &RemoteBaseConfig {
        &self.config
    }

    /// Return actual selected capabilities.
    pub fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    /// Return copied status without touching physical I/O or the backend.
    pub fn snapshot(&self) -> RigSnapshot {
        self.snapshot.clone()
    }

    /// Drain a result; an urgent inhibit can complete while an older tune waits.
    pub fn take_completion(&mut self) -> Option<Completion> {
        self.completion
            .take()
            .or_else(|| self.critical_completion.take())
    }

    /// Retire this owner; later service can only try to unkey it.
    pub fn retire(&mut self) {
        self.retired = true;
        self.pending = None;
        self.snapshot.pending = false;
        self.update_admission();
    }

    /// Admit a validated operation without physical I/O or waiting for its completion.
    pub fn request(&mut self, request: RigRequest, _now_ms: u64) -> Result<RequestId, Error> {
        if self.retired {
            return Err(Error::Stale);
        }
        if matches!(request, RigRequest::Inhibit(true)) {
            if self.critical_completion.is_some() {
                return Err(Error::Busy);
            }
            let id = self.allocate_id()?;
            self.administrative_inhibit = true;
            self.update_admission();
            self.critical_completion = Some(Completion { id, result: Ok(()) });
            return Ok(id);
        }
        if self.completion.is_some() || self.pending.is_some() || self.critical_completion.is_some()
        {
            return Err(Error::Busy);
        }
        let desired = match request {
            RigRequest::Set(ref patch) => Some(patch.apply(self.snapshot.requested.clone())),
            RigRequest::Recall(selector) => Some(self.config.memory(selector)?.state.clone()),
            RigRequest::Inhibit(_) => None,
        };
        if let Some(state) = desired.as_ref() {
            self.config
                .limits
                .validate_state(state, &self.capabilities)?;
        }
        let id = self.allocate_id()?;
        if let Some(state) = desired {
            self.snapshot.requested = state.clone();
            self.pending = Some(Pending { id, state });
            self.snapshot.pending = true;
        } else if let RigRequest::Inhibit(inhibit) = request {
            self.administrative_inhibit = inhibit;
            self.update_admission();
            self.completion = Some(Completion { id, result: Ok(()) });
        }
        Ok(id)
    }

    fn allocate_id(&mut self) -> Result<RequestId, Error> {
        let next = self.next_id.checked_add(1).ok_or(Error::Busy)?;
        let id = RequestId(self.next_id);
        self.next_id = next;
        Ok(id)
    }

    /// Advance prepared control work; only the physical owner may invoke this method.
    pub fn advance(
        &mut self,
        now_ms: u64,
        activity: Activity,
        backend: &mut dyn RigBackend,
    ) -> Result<(), Error> {
        if self.retired {
            self.set_key(false, backend)?;
            return Err(Error::Stale);
        }
        if !activity.transmit_requested || self.snapshot.transmit_inhibited {
            self.set_key(false, backend)?;
        }
        if self.pending.is_some() && !activity.physical_ptt && !activity.command_source_active {
            let pending = self
                .pending
                .take()
                .expect("pending application was checked");
            let result = self.apply(&pending.state, backend);
            self.snapshot.pending = false;
            if pending.id.0 != 0 {
                self.completion = Some(Completion {
                    id: pending.id,
                    result,
                });
            }
            self.next_poll_ms = now_ms.saturating_add(self.poll_interval_ms());
            // Physical I/O may have blocked. Read a fresh activity snapshot on the
            // next service iteration before admitting any new PTT.
            return result;
        }
        if self.capabilities.readback_fields != 0 && now_ms >= self.next_poll_ms {
            self.next_poll_ms = now_ms.saturating_add(self.poll_interval_ms());
            self.observe(backend, false)?;
        }
        let keyed = activity.transmit_requested
            && !self.snapshot.transmit_inhibited
            && (self.pending.is_none() || activity.physical_ptt);
        self.set_key(keyed, backend)
    }

    fn poll_interval_ms(&self) -> u64 {
        if self.config.poll_interval_ms == 0 {
            500
        } else {
            self.config.poll_interval_ms
        }
    }

    fn update_admission(&mut self) {
        self.snapshot.transmit_inhibited = self.retired
            || self.administrative_inhibit
            || !self.safe
            || self
                .snapshot
                .applied
                .as_ref()
                .is_none_or(|state| state.transmit_hz.is_none());
    }

    fn set_key(&mut self, keyed: bool, backend: &mut dyn RigBackend) -> Result<(), Error> {
        if self.snapshot.ptt_applied == Some(keyed) {
            return Ok(());
        }
        if let Err(code) = backend.set_ptt(keyed) {
            self.record_failure(code);
            self.snapshot.ptt_applied = if keyed && backend.set_ptt(false).is_ok() {
                Some(false)
            } else {
                None
            };
            return Err(Error::Backend);
        }
        self.snapshot.ptt_applied = Some(keyed);
        Ok(())
    }

    fn record_failure(&mut self, code: i32) {
        self.snapshot.failure.get_or_insert(Failure { code });
        self.safe = false;
        self.snapshot.receive_permitted = false;
        self.snapshot.verified = false;
        self.update_admission();
    }

    fn apply(
        &mut self,
        desired: &OperatingState,
        backend: &mut dyn RigBackend,
    ) -> Result<(), Error> {
        self.safe = false;
        self.update_admission();
        self.set_key(false, backend)?;
        let previous = self.snapshot.applied.clone();
        let result = backend.apply_step(desired);
        if let Err(code) = result {
            self.record_failure(code);
            if let Some(previous) = previous {
                let _ = backend.apply_step(&previous);
            }
            return Err(Error::Backend);
        }
        self.snapshot.applied = Some(desired.clone());
        self.snapshot.observed = None;
        self.snapshot.observed_fields = 0;
        self.snapshot.verified = false;
        if self.capabilities.readback_fields != 0 && !self.observe(backend, true)? {
            self.record_failure(Error::ReadbackMismatch as i32);
            return Err(Error::ReadbackMismatch);
        }
        self.snapshot.failure = None;
        self.safe = true;
        self.snapshot.receive_permitted = true;
        self.update_admission();
        Ok(())
    }

    fn observe(&mut self, backend: &mut dyn RigBackend, applying: bool) -> Result<bool, Error> {
        let raw = backend.read_state().map_err(|code| {
            self.record_failure(code);
            Error::Backend
        })?;
        let mut observed = self
            .snapshot
            .applied
            .as_ref()
            .unwrap_or(&self.snapshot.requested)
            .clone();
        let fields = self.capabilities.readback_fields;
        macro_rules! merge {
            ($($bit:literal => $field:ident),* $(,)?) => { $(if fields & (1 << $bit) != 0 { observed.$field = raw.$field; })* };
        }
        merge!(0 => receive_hz, 1 => transmit_hz, 2 => mode, 3 => bandwidth_hz,
            4 => receive_vfo, 5 => transmit_vfo, 6 => offset_hz, 7 => rit_hz,
            8 => xit_hz, 9 => power_mw, 10 => encode, 11 => decode, 12 => channel);
        let validation = self
            .config
            .limits
            .validate_state(&observed, &self.capabilities);
        self.snapshot.observed = Some(raw);
        self.snapshot.observed_fields = fields;
        if let Err(error) = validation {
            self.safe = false;
            self.snapshot.verified = false;
            self.snapshot.receive_permitted = error != Error::ReceiveFrequency;
            self.update_admission();
            self.set_key(false, backend)?;
            return Err(error);
        }
        let matches = self.snapshot.applied.as_ref().is_some_and(|applied| {
            let mut expected = applied.clone();
            if expected.bandwidth_hz == 0 {
                expected.bandwidth_hz = observed.bandwidth_hz;
            }
            if expected.receive_vfo == 0 {
                expected.receive_vfo = observed.receive_vfo;
            }
            if expected.transmit_vfo == 0 {
                expected.transmit_vfo = observed.transmit_vfo;
            }
            expected == observed
        });
        self.snapshot.verified = matches && fields == 8191;
        if !applying {
            // An allowed front-panel change becomes the base for later partial
            // commands; a power edit must not restore a stale VFO frequency.
            self.snapshot.applied = Some(observed.clone());
            if self.pending.is_none() {
                self.snapshot.requested = observed;
            }
            self.snapshot.verified = fields == 8191;
            self.snapshot.receive_permitted = true;
            self.update_admission();
        }
        Ok(matches)
    }
}
