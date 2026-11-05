//! Owned configuration and adapter-neutral radio state.

use crate::{Error, Limits};

/// Canonical operating modes, independent of Hamlib bit values.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum Mode {
    /// Frequency modulation.
    #[default]
    Fm = 1,
    /// Amplitude modulation.
    Am = 2,
    /// Upper sideband.
    Usb = 3,
    /// Lower sideband.
    Lsb = 4,
    /// Continuous wave.
    Cw = 5,
    /// Reverse continuous wave.
    CwReverse = 6,
    /// Radioteletype.
    Rtty = 7,
    /// Reverse radioteletype.
    RttyReverse = 8,
    /// Wide frequency modulation.
    WideFm = 9,
    /// Packet/data frequency modulation.
    PacketFm = 10,
    /// Packet/data upper sideband.
    PacketUsb = 11,
    /// Packet/data lower sideband.
    PacketLsb = 12,
}

/// Exactly one provider for an independently selected signaling direction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u32)]
pub enum SignalSource {
    /// No tone generation or qualification.
    #[default]
    Disabled = 0,
    /// Shared native DSP.
    Dsp = 1,
    /// External hardware encoder/decoder.
    External = 2,
    /// Radio's own supported encoder/decoder.
    Rig = 3,
}

/// CTCSS or DCS selection for one direction.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Signaling {
    /// Selected provider; encode and decode use separate records.
    pub source: SignalSource,
    /// Whether the selected signal is currently enabled.
    pub enabled: bool,
    /// CTCSS in tenths of hertz, or zero when DCS/no tone is selected.
    pub ctcss_tenths_hz: u32,
    /// Actual nine-bit DCS value (not decimal-spelled octal digits), or zero.
    pub dcs_code: u32,
    /// Inverted DCS polarity.
    pub inverted: bool,
}

/// Complete desired operating state; no physical handle or host type is retained.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OperatingState {
    /// Receive base VFO frequency, before RIT.
    pub receive_hz: u64,
    /// Transmit base VFO frequency, before offset/XIT; absent means receive-only.
    pub transmit_hz: Option<u64>,
    /// Mode on this operating memory.
    pub mode: Mode,
    /// Requested passband width in hertz; zero selects the backend's normal width.
    pub bandwidth_hz: u32,
    /// Selected receive VFO, using the adapter-neutral catalog.
    pub receive_vfo: u32,
    /// Selected transmit VFO.
    pub transmit_vfo: u32,
    /// Repeater shift applied to the transmit VFO.
    pub offset_hz: i64,
    /// Receive incremental tuning.
    pub rit_hz: i64,
    /// Transmit incremental tuning.
    pub xit_hz: i64,
    /// Established transmit power in milliwatts, when available.
    pub power_mw: Option<u32>,
    /// Independently selected transmit signaling.
    pub encode: Signaling,
    /// Independently selected receive qualification.
    pub decode: Signaling,
    /// Optional parallel hardware channel number.
    pub channel: Option<u8>,
}

/// Physical capabilities of the selected rig and its shared signaling providers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    /// Modes the backend can actually apply.
    pub modes: Vec<Mode>,
    /// Hardware receive coverage.
    pub receive: Vec<crate::FrequencyRange>,
    /// Hardware transmit coverage.
    pub transmit: Vec<crate::FrequencyRange>,
    /// Whether power can be established in milliwatts.
    pub power_mw: bool,
    /// Supported encode providers.
    pub encode_sources: Vec<SignalSource>,
    /// Supported decode providers; does not imply encode support.
    pub decode_sources: Vec<SignalSource>,
    /// Readable state-field mask, in the same order as an operating patch.
    /// Zero means write-only; unavailable fields are never observations.
    pub readback_fields: u64,
}

/// Named configuration-only memory with a stable numeric selector.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Memory {
    /// Stable public DTMF selector.
    pub selector: u32,
    /// Operator-supplied name.
    pub name: String,
    /// Complete validated operating state.
    pub state: OperatingState,
}

/// Named configuration-only scan definition; live cursor changes are separate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScanList {
    /// Stable public DTMF selector.
    pub selector: u32,
    /// Operator-supplied name.
    pub name: String,
    /// Ordered memory selectors.
    pub members: Vec<u32>,
    /// Dwell after completed tune and settle, in milliseconds.
    pub dwell_ms: u64,
    /// Quiet period after activity, in milliseconds.
    pub resume_delay_ms: u64,
    /// Additional model-specific settle time after applying a member.
    pub settle_ms: u64,
    /// Optional priority member belonging to this list.
    pub priority: Option<u32>,
}

impl Default for ScanList {
    fn default() -> Self {
        Self {
            selector: 0,
            name: String::new(),
            members: Vec::new(),
            dwell_ms: 500,
            resume_delay_ms: 2_000,
            settle_ms: 0,
            priority: None,
        }
    }
}

/// Complete setup retained by one radio owner.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RemoteBaseConfig {
    /// Node RF limits.
    pub limits: Limits,
    /// Configuration-only memories.
    pub memories: Vec<Memory>,
    /// Configuration-only lists.
    pub scan_lists: Vec<ScanList>,
    /// Safe memory selected on startup.
    pub startup_memory: u32,
    /// Monotonic readback interval; zero selects the 500 ms default.
    pub poll_interval_ms: u64,
}

impl RemoteBaseConfig {
    /// Validate all references and states before a physical owner is created.
    pub fn validate(&self, capabilities: &Capabilities) -> Result<(), Error> {
        if capabilities.readback_fields & !8191 != 0 {
            return Err(Error::InvalidConfiguration);
        }
        self.limits.validate()?;
        for (index, memory) in self.memories.iter().enumerate() {
            if memory.selector == 0
                || memory.name.is_empty()
                || self.memories[..index].iter().any(|previous| {
                    previous.selector == memory.selector || previous.name == memory.name
                })
            {
                return Err(Error::InvalidConfiguration);
            }
            self.limits.validate_state(&memory.state, capabilities)?;
        }
        self.memory(self.startup_memory)?;
        for (index, list) in self.scan_lists.iter().enumerate() {
            if list.selector == 0
                || list.name.is_empty()
                || list.members.is_empty()
                || list.dwell_ms == 0
                || list.settle_ms.checked_add(list.dwell_ms).is_none()
                || self.scan_lists[..index].iter().any(|previous| {
                    previous.selector == list.selector || previous.name == list.name
                })
                || list
                    .priority
                    .is_some_and(|value| !list.members.contains(&value))
            {
                return Err(Error::InvalidConfiguration);
            }
            for member in &list.members {
                self.memory(*member)?;
            }
        }
        Ok(())
    }

    /// Find one configured memory without interpreting its name as an address.
    pub fn memory(&self, selector: u32) -> Result<&Memory, Error> {
        self.memories
            .iter()
            .find(|memory| memory.selector == selector)
            .ok_or(Error::NotFound)
    }
}
