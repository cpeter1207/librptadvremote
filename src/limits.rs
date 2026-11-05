//! One shared validation path for direct commands, memories, and scans.

use crate::{Capabilities, OperatingState, SignalSource, Signaling};

/// Adapter-neutral operation failure, retained as the first transaction cause.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum Error {
    /// Configuration cannot establish a safe deterministic radio state.
    InvalidConfiguration = -1,
    /// Effective receive frequency is outside permitted coverage.
    ReceiveFrequency = -2,
    /// Effective transmit frequency is outside permitted coverage.
    TransmitFrequency = -3,
    /// Selected mode is not permitted or supported.
    Mode = -4,
    /// Power cannot be established or exceeds its configured ceiling.
    Power = -5,
    /// Signal source or signal value is unsupported or inconsistent.
    Signaling = -6,
    /// Referenced memory/list/setting does not exist.
    NotFound = -7,
    /// Physical application or observation failed; the snapshot retains its cause.
    Backend = -8,
    /// The bounded pending operation/completion slot is occupied.
    Busy = -9,
    /// The context has retired and cannot admit another operation.
    Stale = -10,
    /// Supported state readback disagrees with the requested tune.
    ReadbackMismatch = -11,
    /// A declared operating function is unavailable in this composition.
    Unsupported = -12,
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for Error {}

/// Inclusive frequency interval in hertz.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct FrequencyRange {
    /// Lowest permitted frequency.
    pub minimum: u64,
    /// Highest permitted frequency.
    pub maximum: u64,
}

impl FrequencyRange {
    /// Construct an interval; setup validation rejects inverted intervals.
    pub const fn new(minimum: u64, maximum: u64) -> Self {
        Self { minimum, maximum }
    }

    /// Test an inclusive frequency without arithmetic overflow.
    pub fn contains(&self, frequency: u64) -> bool {
        (self.minimum..=self.maximum).contains(&frequency)
    }
}

/// Node safety policy, independent of both protocol and physical transport.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Limits {
    /// Permitted receive intervals.
    pub receive: Vec<FrequencyRange>,
    /// Permitted transmit intervals.
    pub transmit: Vec<FrequencyRange>,
    /// Intervals explicitly prohibited for transmit.
    pub receive_only: Vec<FrequencyRange>,
    /// Permitted modes.
    pub modes: Vec<crate::Mode>,
    /// Optional maximum established transmit power.
    pub maximum_power_mw: Option<u32>,
}

impl Limits {
    /// Reject an empty or contradictory safety policy.
    pub fn validate(&self) -> Result<(), Error> {
        if self.receive.is_empty()
            || self.modes.is_empty()
            || self
                .receive
                .iter()
                .chain(&self.transmit)
                .chain(&self.receive_only)
                .any(|range| range.minimum == 0 || range.minimum > range.maximum)
        {
            return Err(Error::InvalidConfiguration);
        }
        Ok(())
    }

    /// Validate complete effective RX/TX state before any physical action.
    pub fn validate_state(&self, state: &OperatingState, caps: &Capabilities) -> Result<(), Error> {
        let receive = state
            .receive_hz
            .checked_add_signed(state.rit_hz)
            .ok_or(Error::ReceiveFrequency)?;
        if !contains(&self.receive, receive)
            || !caps
                .receive
                .iter()
                .any(|range| range.covers(receive, state.mode))
        {
            return Err(Error::ReceiveFrequency);
        }
        if !self.modes.contains(&state.mode) || !caps.modes.contains(&state.mode) {
            return Err(Error::Mode);
        }
        if let Some(base) = state.transmit_hz {
            let transmit = base
                .checked_add_signed(state.offset_hz)
                .and_then(|value| value.checked_add_signed(state.xit_hz))
                .ok_or(Error::TransmitFrequency)?;
            if !contains(&self.transmit, transmit)
                || !caps
                    .transmit
                    .iter()
                    .any(|range| range.covers(transmit, state.mode))
                || contains(&self.receive_only, transmit)
            {
                return Err(Error::TransmitFrequency);
            }
            if self.maximum_power_mw.is_some_and(|maximum| {
                !caps.power_mw || state.power_mw.is_none_or(|power| power > maximum)
            }) || state.power_mw.is_some_and(|power| {
                !caps.power_mw
                    || !caps.transmit.iter().any(|range| {
                        range.covers(transmit, state.mode) && range.permits_power(power)
                    })
            }) {
                return Err(Error::Power);
            }
        }
        validate_signal(state.encode, &caps.encode_sources, &[])?;
        validate_signal(
            state.decode,
            &caps.decode_sources,
            &caps.decode_control_sources,
        )
    }
}

fn contains(ranges: &[FrequencyRange], frequency: u64) -> bool {
    ranges.iter().any(|range| range.contains(frequency))
}

fn validate_signal(
    signal: Signaling,
    supported: &[SignalSource],
    controls: &[SignalSource],
) -> Result<(), Error> {
    let programming = signal.control_source.unwrap_or(signal.source);
    if signal.source != SignalSource::Disabled && !supported.contains(&signal.source)
        || signal
            .control_source
            .is_some_and(|source| source == SignalSource::Disabled || !controls.contains(&source))
        || signal.dcs_code > 511
        || signal.ctcss_tenths_hz != 0 && (signal.dcs_code != 0 || signal.inverted)
        || programming == SignalSource::Rig && signal.inverted
        || signal.enabled
            && (signal.source == SignalSource::Disabled
                || programming != SignalSource::External
                    && signal.ctcss_tenths_hz == 0
                    && signal.dcs_code == 0)
    {
        return Err(Error::Signaling);
    }
    Ok(())
}
