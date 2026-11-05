//! Serialized remote-base tune and transmit admission on the physical owner.

use crate::{
    Capabilities, Error, OperatingState, RemoteBaseConfig, ScanRequest, ScanSnapshot, scan::Scanner,
};

/// Physical calls supplied by the selected dependency adapter and GPIO owner.
pub trait RigBackend {
    /// Read actual supported fields; only `Capabilities::readback_fields` are
    /// observations. Other record fields are ignored, never fabricated readback.
    fn read_state(&mut self) -> Result<OperatingState, i32>;
    /// Apply a prepared complete state; failure may leave partial physical changes.
    fn apply_step(&mut self, state: &OperatingState) -> Result<(), i32>;
    /// Apply PTT through its one explicitly selected provider.
    fn set_ptt(&mut self, keyed: bool) -> Result<(), i32>;
    /// Start/stop existing manual key and test-tone intent. The renderer produces
    /// PCM and final PTT demand; clearing this intent retains its normal tails.
    fn set_tune(&mut self, enabled: bool) -> Result<(), i32>;
    /// Current owner clock, in the same monotonic milliseconds as `advance`.
    /// It is read again after a potentially blocking physical call.
    fn monotonic_ms(&self) -> u64 {
        0
    }
    /// Rising-edge count for transmit demand, published independently by audio.
    fn transmit_epoch(&self) -> u64 {
        0
    }
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
    /// Originating source/session was revoked; ordinary unkey is not revocation.
    pub command_cancelled: bool,
    /// Observed physical PTT, distinct from an intent or queued operation.
    pub physical_ptt: bool,
    /// Current ordinary transmit demand, independent of receive qualification.
    pub transmit_requested: bool,
    /// Final radio-renderer PTT demand, including its settle and signaling tails.
    pub rendered_ptt: bool,
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
    /// Explicit split routing, independent of the TX-frequency value.
    pub split: Option<bool>,
    /// New repeater shift.
    pub offset_hz: Option<i64>,
    /// New receive incremental tuning.
    pub rit_hz: Option<i64>,
    /// Independently enable or disable receive incremental tuning.
    pub rit_enabled: Option<bool>,
    /// New transmit incremental tuning.
    pub xit_hz: Option<i64>,
    /// Independently enable or disable transmit incremental tuning.
    pub xit_enabled: Option<bool>,
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
            split,
            offset_hz,
            rit_hz,
            rit_enabled,
            xit_hz,
            xit_enabled,
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
    /// Key a bounded manual test using the existing backend tone path.
    Tune {
        /// Milliseconds after serviced keying, inclusive range one through 5000.
        duration_ms: u64,
    },
    /// Validate and apply a partial live change.
    Set(OperatingPatch),
    /// Recall a configured complete memory.
    Recall(u32),
    /// Change only administrative inhibit; a safety fault remains independent.
    Inhibit(bool),
    /// Operate an immutable configured scan list.
    Scan(ScanRequest),
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
    /// The bounded manual test is currently keyed, not merely waiting for unkey.
    pub manual_tune_active: bool,
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
    /// Number of transmit bursts rejected because they began during scan tuning.
    pub scan_rejected_bursts: u64,
}

struct Pending {
    id: RequestId,
    state: OperatingState,
    scan_member: Option<u32>,
}
#[derive(Clone, Copy)]
struct ManualTune {
    id: RequestId,
    duration_ms: u64,
    deadline_ms: Option<u64>,
    intent_started: bool,
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
    scan: Option<Scanner>,
    blocked_scan_burst: bool,
    last_attempt_code: Option<i32>,
    manual_tune: Option<ManualTune>,
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
                scan_member: None,
            }),
            snapshot: RigSnapshot {
                manual_tune_active: false,
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
                scan_rejected_bursts: 0,
            },
            completion: None,
            critical_completion: None,
            next_id: 1,
            administrative_inhibit: false,
            safe: false,
            retired: false,
            next_poll_ms: 0,
            scan: None,
            blocked_scan_burst: false,
            last_attempt_code: None,
            manual_tune: None,
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

    /// Return scan selection/hold/error state without physical I/O.
    pub fn scan_snapshot(&self) -> Option<ScanSnapshot> {
        self.scan.as_ref().map(Scanner::snapshot)
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
        if let Some(scan) = self.scan.as_mut() {
            scan.stop();
        }
        self.pending = None;
        self.snapshot.pending = false;
        self.update_admission();
    }

    /// Admit a validated operation without physical I/O or waiting for its completion.
    pub fn request(&mut self, request: RigRequest, now_ms: u64) -> Result<RequestId, Error> {
        if self.retired {
            return Err(Error::Stale);
        }
        match request {
            RigRequest::Tune { duration_ms } => {
                if !(1..=5000).contains(&duration_ms) {
                    return Err(Error::InvalidConfiguration);
                }
                self.ordinary_slot(true)?;
                if !self.capabilities.manual_tune || self.snapshot.transmit_inhibited {
                    return Err(Error::Unsupported);
                }
                if let Some(scan) = &mut self.scan {
                    scan.stop();
                }
                let id = self.allocate_id()?;
                self.manual_tune = Some(ManualTune {
                    id,
                    duration_ms,
                    deadline_ms: None,
                    intent_started: false,
                });
                self.snapshot.pending = true;
                Ok(id)
            }
            RigRequest::Scan(request) => self.request_scan(request, now_ms),
            RigRequest::Inhibit(true) => {
                if self.critical_completion.is_some() {
                    return Err(Error::Busy);
                }
                let id = self.allocate_id()?;
                self.administrative_inhibit = true;
                self.update_admission();
                self.critical_completion = Some(Completion { id, result: Ok(()) });
                Ok(id)
            }
            RigRequest::Inhibit(false) => {
                self.ordinary_slot(false)?;
                let id = self.allocate_id()?;
                self.administrative_inhibit = false;
                self.update_admission();
                self.completion = Some(Completion { id, result: Ok(()) });
                Ok(id)
            }
            RigRequest::Set(patch) => self.queue_manual(
                patch.apply(
                    self.snapshot
                        .applied
                        .as_ref()
                        .unwrap_or(&self.snapshot.requested)
                        .clone(),
                ),
            ),
            RigRequest::Recall(selector) => {
                self.queue_manual(self.config.memory(selector)?.state.clone())
            }
        }
    }

    fn ordinary_slot(&mut self, cancel_automatic: bool) -> Result<(), Error> {
        if self.manual_tune.is_some()
            || self.completion.is_some()
            || self.critical_completion.is_some()
            || self
                .scan
                .as_ref()
                .is_some_and(|scan| scan.start_request.is_some())
            || self
                .pending
                .as_ref()
                .is_some_and(|pending| !cancel_automatic || pending.scan_member.is_none())
        {
            return Err(Error::Busy);
        }
        if cancel_automatic {
            self.pending = None;
        }
        Ok(())
    }

    fn queue_manual(&mut self, state: OperatingState) -> Result<RequestId, Error> {
        self.config
            .limits
            .validate_state(&state, &self.capabilities)?;
        self.ordinary_slot(true)?;
        if let Some(scan) = self.scan.as_mut() {
            scan.stop();
        }
        let id = self.allocate_id()?;
        self.snapshot.requested = state.clone();
        self.pending = Some(Pending {
            id,
            state,
            scan_member: None,
        });
        self.snapshot.pending = true;
        self.update_admission();
        Ok(id)
    }

    fn request_scan(&mut self, request: ScanRequest, now: u64) -> Result<RequestId, Error> {
        match request {
            ScanRequest::Pause => self.interrupt_scan(true),
            ScanRequest::Stop => self.interrupt_scan(false),
            ScanRequest::Start {
                selector,
                direction,
                dwell_ms,
            } => {
                self.ordinary_slot(false)?;
                let list = self
                    .config
                    .scan_lists
                    .iter()
                    .find(|list| list.selector == selector)
                    .ok_or(Error::NotFound)?;
                if list.members.iter().any(|selector| {
                    self.config.memory(*selector).is_ok_and(|memory| {
                        !self.capabilities.scan_modes.contains(&memory.state.mode)
                    })
                }) {
                    return Err(Error::Unsupported);
                }
                let list = list.clone();
                let id = self.allocate_id()?;
                let mut scan = Scanner::new(list, direction, dwell_ms, id)?;
                scan.activity(now, false);
                self.scan = Some(scan);
                self.snapshot.pending = true;
                self.update_admission();
                Ok(id)
            }
            ScanRequest::Resume => self.change_scan(Scanner::resume),
            ScanRequest::Direction(direction) => self.change_scan(|scan| {
                scan.direction(direction);
                Ok(())
            }),
            ScanRequest::Skip => self.change_scan(|scan| {
                scan.skip();
                Ok(())
            }),
            ScanRequest::SetDwell(duration) => self.change_scan(|scan| scan.dwell(duration)),
            ScanRequest::SetResumeDelay(duration) => self.change_scan(|scan| {
                scan.resume_delay(duration);
                Ok(())
            }),
            ScanRequest::SetPriority(member) => self.change_scan(|scan| scan.priority(member)),
        }
    }

    fn change_scan(
        &mut self,
        action: impl FnOnce(&mut Scanner) -> Result<(), Error>,
    ) -> Result<RequestId, Error> {
        self.ordinary_slot(false)?;
        action(self.scan.as_mut().ok_or(Error::NotFound)?)?;
        let id = self.allocate_id()?;
        self.completion = Some(Completion { id, result: Ok(()) });
        self.update_admission();
        Ok(id)
    }

    fn interrupt_scan(&mut self, pause: bool) -> Result<RequestId, Error> {
        if self.critical_completion.is_some() {
            return Err(Error::Busy);
        }
        if pause
            && !self
                .scan
                .as_ref()
                .is_some_and(|scan| scan.snapshot().active)
        {
            return Err(Error::NotFound);
        }
        let id = self.allocate_id()?;
        if let Some(scan) = self.scan.as_mut() {
            let cancelled = if pause { scan.pause() } else { scan.stop() };
            if let Some(completion) = cancelled {
                self.completion = Some(completion);
            }
        }
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.scan_member.is_some())
        {
            self.pending = None;
        }
        self.snapshot.pending = self.pending.is_some();
        self.critical_completion = Some(Completion { id, result: Ok(()) });
        self.update_admission();
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
        if let Some(result) = self.advance_manual_tune(now_ms, activity, backend) {
            return result.and(if self.retired {
                Err(Error::Stale)
            } else {
                Ok(())
            });
        }
        if self.retired {
            self.set_key(false, backend)?;
            return Err(Error::Stale);
        }
        if activity.command_cancelled
            && self
                .pending
                .as_ref()
                .is_some_and(|pending| pending.id.0 != 0 && pending.scan_member.is_none())
        {
            let pending = self.pending.take().expect("manual pending was checked");
            self.completion = Some(Completion {
                id: pending.id,
                result: Err(Error::Stale),
            });
            self.snapshot.pending = false;
            if let Some(applied) = &self.snapshot.applied {
                self.snapshot.requested = applied.clone();
            }
            self.update_admission();
        }
        if !activity.transmit_requested && self.blocked_scan_burst {
            self.blocked_scan_burst = false;
            self.update_admission();
        }
        if let Some(scan) = self.scan.as_mut() {
            scan.activity(
                now_ms,
                activity.local_receiving
                    || activity.linked_receiving
                    || activity.transmit_requested,
            );
            if self.pending.is_none() {
                if let Some(member) = scan.due() {
                    let state = self.config.memory(member)?.state.clone();
                    self.snapshot.requested = state.clone();
                    self.pending = Some(Pending {
                        id: RequestId(0),
                        state,
                        scan_member: Some(member),
                    });
                    self.snapshot.pending = true;
                }
            }
        }
        if !activity.rendered_ptt || self.snapshot.transmit_inhibited {
            self.set_key(false, backend)?;
        }
        if self.pending.as_ref().is_some_and(|pending| {
            pending.scan_member.is_none()
                || !(activity.local_receiving
                    || activity.linked_receiving
                    || activity.transmit_requested)
        }) && !activity.physical_ptt
            && !activity.rendered_ptt
            && !activity.command_source_active
        {
            let pending = self
                .pending
                .take()
                .expect("pending application was checked");
            let epoch = backend.transmit_epoch();
            let result = self.apply(&pending.state, backend);
            let completed = now_ms.max(backend.monotonic_ms());
            if let Some(member) = pending.scan_member {
                let after = backend.transmit_epoch();
                if after != epoch {
                    self.blocked_scan_burst = true;
                    self.snapshot.scan_rejected_bursts = self
                        .snapshot
                        .scan_rejected_bursts
                        .saturating_add(after.wrapping_sub(epoch));
                }
                if let Some(scan) = self.scan.as_mut() {
                    if let Some(completion) =
                        scan.complete(member, completed, result, self.last_attempt_code)
                    {
                        self.completion = Some(completion);
                    }
                }
            } else if pending.id.0 != 0 {
                self.completion = Some(Completion {
                    id: pending.id,
                    result,
                });
            }
            self.snapshot.pending = self
                .scan
                .as_ref()
                .is_some_and(|scan| scan.start_request.is_some());
            self.update_admission();
            self.next_poll_ms = completed.saturating_add(self.poll_interval_ms());
            // Physical I/O may have blocked. Read a fresh activity snapshot on the
            // next service iteration before admitting any new PTT.
            return result;
        }
        if self.capabilities.readback_fields != 0 && now_ms >= self.next_poll_ms {
            let result = self.observe(backend, false);
            self.next_poll_ms = now_ms
                .max(backend.monotonic_ms())
                .saturating_add(self.poll_interval_ms());
            result?;
        }
        let keyed = activity.rendered_ptt
            && !self.snapshot.transmit_inhibited
            && (self
                .pending
                .as_ref()
                .is_none_or(|pending| pending.scan_member.is_some())
                || activity.physical_ptt);
        self.set_key(keyed, backend)
    }

    fn poll_interval_ms(&self) -> u64 {
        if self.config.poll_interval_ms == 0 {
            500
        } else {
            self.config.poll_interval_ms
        }
    }

    fn advance_manual_tune(
        &mut self,
        now: u64,
        activity: Activity,
        backend: &mut dyn RigBackend,
    ) -> Option<Result<(), Error>> {
        let tune = self.manual_tune?;
        let cancelled =
            activity.command_cancelled || self.retired || self.snapshot.transmit_inhibited;
        if cancelled || tune.deadline_ms.is_some_and(|deadline| now >= deadline) {
            let tone = backend.set_tune(false).map_err(|code| {
                self.record_failure(code);
                Error::Backend
            });
            // End only the manual intent. The renderer still owns ordinary
            // demand and signaling tails; faults/retirement force immediate off.
            let key = self.set_key(
                activity.rendered_ptt && !self.retired && !self.snapshot.transmit_inhibited,
                backend,
            );
            let result = tone.and(key);
            self.complete_manual_tune(result.and(if cancelled {
                Err(Error::Stale)
            } else {
                Ok(())
            }));
            return Some(result);
        }
        if !tune.intent_started {
            if activity.physical_ptt
                || activity.rendered_ptt
                || activity.command_source_active
                || activity.transmit_requested
            {
                return None;
            }
            let result = backend.set_tune(true).map_err(|code| {
                self.record_failure(code);
                Error::Backend
            });
            if let Err(error) = result {
                let _ = backend
                    .set_tune(false)
                    .map_err(|code| self.record_failure(code));
                self.complete_manual_tune(Err(error));
                return Some(Err(error));
            }
            self.manual_tune
                .as_mut()
                .expect("tune is retained")
                .intent_started = true;
            return Some(Ok(()));
        }
        if let Err(error) = self.set_key(activity.rendered_ptt, backend) {
            let _ = backend
                .set_tune(false)
                .map_err(|code| self.record_failure(code));
            self.complete_manual_tune(Err(error));
            return Some(Err(error));
        }
        self.snapshot.manual_tune_active = activity.rendered_ptt;
        if tune.deadline_ms.is_none() && activity.rendered_ptt {
            self.manual_tune
                .as_mut()
                .expect("tune is retained")
                .deadline_ms = Some(
                now.max(backend.monotonic_ms())
                    .saturating_add(tune.duration_ms),
            );
            self.snapshot.pending = false;
        }
        Some(Ok(()))
    }

    fn complete_manual_tune(&mut self, result: Result<(), Error>) {
        let tune = self
            .manual_tune
            .take()
            .expect("manual tune is retained until unkey");
        self.snapshot.manual_tune_active = false;
        self.snapshot.pending = false;
        self.completion = Some(Completion {
            id: tune.id,
            result,
        });
    }

    fn update_admission(&mut self) {
        self.snapshot.transmit_inhibited = self.retired
            || self.administrative_inhibit
            || !self.safe
            || self.blocked_scan_burst
            || self.scan.as_ref().is_some_and(|scan| {
                let status = scan.snapshot();
                status.active && status.current_memory.is_none()
            })
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
        self.last_attempt_code.get_or_insert(code);
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
        self.last_attempt_code = None;
        self.safe = false;
        self.snapshot.receive_permitted = false;
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
            8 => xit_hz, 9 => power_mw, 10 => encode, 11 => decode, 12 => channel,
            13 => split, 14 => rit_enabled, 15 => xit_enabled);
        // A physical TX VFO still exists for a receive-only memory. Readback
        // cannot grant permission to use it; keep the untouched raw observation.
        if self
            .snapshot
            .applied
            .as_ref()
            .is_some_and(|state| state.transmit_hz.is_none())
        {
            observed.transmit_hz = None;
        }
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
            // Unspecified means leave the radio's power setting unchanged.
            if expected.power_mw.is_none() {
                expected.power_mw = observed.power_mw;
            }
            // Inactive offsets can share a hardware register. Their stored
            // values are observations, but do not change the admitted RF state.
            if !expected.rit_enabled && !observed.rit_enabled {
                expected.rit_hz = observed.rit_hz;
            }
            if !expected.xit_enabled && !observed.xit_enabled {
                expected.xit_hz = observed.xit_hz;
            }
            expected == observed
        });
        self.snapshot.verified = matches && fields == 65535;
        if !applying {
            // An allowed front-panel change becomes the base for later partial
            // commands; a power edit must not restore a stale VFO frequency.
            self.snapshot.applied = Some(observed.clone());
            if self.pending.is_none() {
                self.snapshot.requested = observed;
            }
            self.snapshot.verified = fields == 65535;
            self.snapshot.receive_permitted = true;
            self.update_admission();
        }
        Ok(matches)
    }
}
