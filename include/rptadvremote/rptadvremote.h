/** @file
 * @brief Portable remote-base configuration and opaque owner contract.
 */
#ifndef RPTADVREMOTE_H
#define RPTADVREMOTE_H
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif

/** State flag: an explicit transmit VFO is present. */
#define RPTADV_REMOTE_TX 1u
/** State/limit/capability flag: power is expressed in milliwatts. */
#define RPTADV_REMOTE_POWER 2u
/** State flag: the channel field is present. */
#define RPTADV_REMOTE_CHANNEL 4u
/** Capability flag: the selected owner has a stoppable calibrated test-tone path. */
#define RPTADV_REMOTE_CAP_TUNE 8u
/** Canonical FM mode identifier; masks use one shifted by the identifier. */
#define RPTADV_REMOTE_FM 1u

/** Inclusive interval, in hertz. */
typedef struct RptAdvRemoteRange {
    uint64_t minimum; /**< Lowest frequency. */
    uint64_t maximum; /**< Highest frequency. */
} RptAdvRemoteRange;
/** Borrowed intervals, copied before setup returns. */
typedef struct RptAdvRemoteRanges {
    const RptAdvRemoteRange *data; /**< Readable aligned array, or NULL if empty. */
    size_t length; /**< Number of intervals. */
} RptAdvRemoteRanges;
/** One hardware interval with its own mode and power capabilities. */
typedef struct RptAdvRemoteCoverage {
    uint64_t minimum; /**< Inclusive lower frequency, Hz. */
    uint64_t maximum; /**< Inclusive upper frequency, Hz. */
    uint64_t mode_mask; /**< Supported modes within this interval. */
    uint32_t flags; /**< Bit zero minimum power, bit one maximum power present. */
    uint32_t minimum_power_mw; /**< Hardware lower bound when flag one is present. */
    uint32_t maximum_power_mw; /**< Hardware upper bound when flag two is present. */
} RptAdvRemoteCoverage;
/** Hardware coverage copied during setup; no mode/range relationships are flattened. */
typedef struct RptAdvRemoteCoverages {
    const RptAdvRemoteCoverage *data; /**< Borrowed aligned array. */
    size_t length; /**< Number of hardware intervals. */
} RptAdvRemoteCoverages;
/** Independent encode/decode signaling; no external encoder programming implied. */
typedef struct RptAdvRemoteSignal {
    uint32_t source; /**< 0 disabled, 1 DSP, 2 external, 3 rig. */
    uint32_t control_source; /**< Decoder programming: zero follows source, otherwise 1 DSP, 2 external, 3 rig. Encode must use zero. */
    uint32_t enabled; /**< Exactly zero or one. */
    uint32_t ctcss_tenths_hz; /**< Tone in tenths of hertz, otherwise zero. */
    uint32_t dcs_code; /**< Actual nine-bit DCS code, otherwise zero. */
    uint32_t inverted; /**< Exactly zero or one. */
} RptAdvRemoteSignal;
/** Complete configured state; this record does not claim physical observation. */
typedef struct RptAdvRemoteState {
    uint64_t receive_hz; /**< Base receive VFO. */
    uint64_t transmit_hz; /**< Base transmit VFO, before offset and XIT. */
    int64_t offset_hz; /**< Transmit repeater shift. */
    int64_t rit_hz; /**< Receive incremental tuning. */
    int64_t xit_hz; /**< Transmit incremental tuning. */
    uint32_t flags; /**< Only TX, POWER, and CHANNEL presence flags. */
    uint32_t mode; /**< Canonical mode, independent of Hamlib flags. */
    uint32_t bandwidth_hz; /**< Zero selects normal backend width. */
    uint32_t receive_vfo; /**< Canonical receive VFO selector. */
    uint32_t transmit_vfo; /**< Canonical transmit VFO selector. */
    uint32_t power_mw; /**< Established power when POWER is present. */
    uint32_t channel; /**< Hardware channel when CHANNEL is present. */
    RptAdvRemoteSignal encode; /**< Independent encode provider. */
    RptAdvRemoteSignal decode; /**< Independent decode provider. */
} RptAdvRemoteState;
/** Node RF limits. */
typedef struct RptAdvRemoteLimits {
    RptAdvRemoteRanges receive; /**< Permitted RX intervals. */
    RptAdvRemoteRanges transmit; /**< Permitted TX intervals. */
    RptAdvRemoteRanges receive_only; /**< Explicitly TX-prohibited intervals. */
    uint64_t mode_mask; /**< Permitted canonical modes. */
    uint32_t flags; /**< POWER if the power ceiling is configured. */
    uint32_t maximum_power_mw; /**< Ceiling when POWER is present. */
} RptAdvRemoteLimits;
/** Selected backend capabilities, including available shared signal providers. */
typedef struct RptAdvRemoteCapabilities {
    RptAdvRemoteCoverages receive; /**< Actual RX coverage, retaining per-range modes. */
    RptAdvRemoteCoverages transmit; /**< Actual TX coverage, retaining modes and power. */
    uint64_t mode_mask; /**< Modes the backend can apply. */
    uint64_t readback_fields; /**< Actually readable operating-patch field bits. */
    uint64_t scan_mode_mask; /**< Modes with usable configured activity qualification. */
    uint32_t flags; /**< POWER and CAP_TUNE only. */
    uint32_t encode_sources; /**< Bit mask, one shifted by source identifier. */
    uint32_t decode_sources; /**< Independent decode provider mask. */
    uint32_t decode_control_sources; /**< Decoder-programming provider mask, separate from qualification. */
} RptAdvRemoteCapabilities;
/** Configuration-only named memory. */
typedef struct RptAdvRemoteMemory {
    uint32_t selector; /**< Positive stable DTMF selector. */
    const char *name; /**< Borrowed UTF-8 name. */
    size_t name_length; /**< Name bytes, without terminator. */
    RptAdvRemoteState state; /**< Complete desired state. */
} RptAdvRemoteMemory;
/** Borrowed memory definitions, copied on setup. */
typedef struct RptAdvRemoteMemories {
    const RptAdvRemoteMemory *data; /**< Aligned readable array. */
    size_t length; /**< Number of memories. */
} RptAdvRemoteMemories;
/** Configuration-only named scan definition. */
typedef struct RptAdvRemoteScan {
    uint32_t selector; /**< Positive stable selector. */
    const char *name; /**< Borrowed UTF-8 name. */
    size_t name_length; /**< Name byte count. */
    const uint32_t *members; /**< Borrowed memory selectors. */
    size_t member_count; /**< Number of members. */
    uint64_t dwell_ms; /**< Time after tune and settle. */
    uint64_t resume_delay_ms; /**< Quiet hold after activity. */
    uint64_t settle_ms; /**< Model-specific tune settling time. */
    uint32_t priority; /**< Priority memory, or zero for disabled. */
} RptAdvRemoteScan;
/** Borrowed scan definitions. */
typedef struct RptAdvRemoteScans {
    const RptAdvRemoteScan *data; /**< Aligned readable array or NULL if empty. */
    size_t length; /**< Number of scan definitions. */
} RptAdvRemoteScans;
/** Exact ABI-one setup. All pointed-to storage is borrowed only for create. */
typedef struct RptAdvRemoteSetup {
    uint32_t struct_size; /**< Exact sizeof this record. */
    uint32_t abi_version; /**< Must be one. */
    RptAdvRemoteLimits limits; /**< Common safety policy. */
    RptAdvRemoteCapabilities capabilities; /**< Selected backend capabilities. */
    RptAdvRemoteMemories memories; /**< Immutable configured memories. */
    RptAdvRemoteScans scan_lists; /**< Immutable configured lists. */
    uint32_t startup_memory; /**< Existing safe memory selected at startup. */
    uint64_t poll_interval_ms; /**< Monotonic readback interval; zero selects 500 ms. */
} RptAdvRemoteSetup;
/** Request kinds; definitions themselves cannot be edited. */
enum RptAdvRemoteRequestKind {
    RPTADV_REMOTE_SET = 1, /**< Apply selected live state fields. */
    RPTADV_REMOTE_RECALL = 2, /**< Recall selector. */
    RPTADV_REMOTE_INHIBIT = 3, /**< Selector zero clears, one sets admin inhibit. */
    RPTADV_REMOTE_SCAN_START = 4, /**< List selector, direction, optional value_ms dwell. */
    RPTADV_REMOTE_SCAN_PAUSE = 5, /**< Pause the current scan. */
    RPTADV_REMOTE_SCAN_RESUME = 6, /**< Resume a paused scan. */
    RPTADV_REMOTE_SCAN_STOP = 7, /**< Stop, retaining the applied member. */
    RPTADV_REMOTE_SCAN_DIRECTION = 8, /**< Direction zero down, one up. */
    RPTADV_REMOTE_SCAN_SKIP = 9, /**< Skip the current member for this scan run. */
    RPTADV_REMOTE_SCAN_DWELL = 10, /**< Positive value_ms. */
    RPTADV_REMOTE_SCAN_RESUME_DELAY = 11, /**< Nonnegative value_ms. */
    RPTADV_REMOTE_SCAN_PRIORITY = 12, /**< Member selector, zero disables. */
    RPTADV_REMOTE_TUNE = 13 /**< Bounded existing test-tone operation, value_ms1..5000. */
};
/** Ordered patch bits: RX, TX, mode, width, RX/TX VFO, offset, RIT, XIT,
 * power, encode, decode, channel. Unknown bits are rejected. */
#define RPTADV_REMOTE_PATCH_ALL UINT64_C(8191)
/** A live operation, copied before request returns. */
typedef struct RptAdvRemoteRequest {
    uint32_t struct_size; /**< Exact record size. */
    uint32_t abi_version; /**< One. */
    uint32_t kind; /**< RptAdvRemoteRequestKind. */
    uint32_t selector; /**< Memory selector or inhibit value. */
    uint64_t fields; /**< Selected patch bits, only used by SET. */
    RptAdvRemoteState state; /**< Only fields selected by the mask are interpreted. */
    uint64_t value_ms; /**< Live scan duration; START zero uses configured dwell. */
    uint32_t direction; /**< Scan direction, zero down or one up. */
} RptAdvRemoteRequest;
/** Activity flags copied from the current radio generation. */
#define RPTADV_REMOTE_LOCAL_RX 1u
/** Linked source is active. */
#define RPTADV_REMOTE_LINK_RX 2u
/** Command source has not unkeyed. */
#define RPTADV_REMOTE_COMMAND_ACTIVE 4u
/** Physical PTT is observed active. */
#define RPTADV_REMOTE_PHYSICAL_PTT 8u
/** Ordinary controller transmit intent. */
#define RPTADV_REMOTE_REQUEST_PTT 16u
/** Originating source/session was revoked; ordinary unkey is not cancellation. */
#define RPTADV_REMOTE_COMMAND_CANCELLED 32u
/** Final renderer PTT demand, including existing settling/signaling tails. */
#define RPTADV_REMOTE_RENDERED_PTT 64u
/** One current-generation radio activity observation. */
typedef struct RptAdvRemoteActivity {
    uint32_t struct_size; /**< Exact record size. */
    uint32_t flags; /**< Known activity flags only. */
} RptAdvRemoteActivity;
/** Synchronous dependency calls made only by the non-audio physical owner. */
typedef struct RptAdvRemoteBackend {
    uint32_t struct_size; /**< Exact table size. */
    uint32_t abi_version; /**< One. */
    void *context; /**< Borrowed only during advance. */
    int (*read_state)(void *, RptAdvRemoteState *); /**< Fill state completely on success. */
    int (*apply_state)(void *, const RptAdvRemoteState *); /**< May partially apply on error. */
    int (*set_ptt)(void *, uint32_t); /**< Apply zero/one through the chosen provider. */
    uint64_t (*monotonic_ms)(void *); /**< Owner clock after blocking tune; required for scan lists. */
    uint64_t (*transmit_epoch)(void *); /**< Published rising-edge count; required for scan lists. */
    int (*set_tune)(void *, uint32_t); /**< Existing manual key/tone intent; CAP_TUNE. Final PTT follows rendered demand. */
} RptAdvRemoteBackend;
/** Snapshot contains an applied state. */
#define RPTADV_REMOTE_APPLIED 1u
/** Snapshot contains observed state. */
#define RPTADV_REMOTE_OBSERVED 2u
/** Supported readback verified the applied state. */
#define RPTADV_REMOTE_VERIFIED 4u
/** Receive routing is permitted. */
#define RPTADV_REMOTE_RECEIVE_PERMITTED 8u
/** New PTT is inhibited. */
#define RPTADV_REMOTE_INHIBITED 16u
/** A tune awaits safe application. */
#define RPTADV_REMOTE_PENDING 32u
/** A first-cause failure is present. */
#define RPTADV_REMOTE_FAILED 64u
/** Last PTT command was successfully serviced. */
#define RPTADV_REMOTE_PTT_APPLIED 128u
/** The last serviced PTT command requested keying; not physical RF confirmation. */
#define RPTADV_REMOTE_PTT_HIGH 256u
/** Bounded manual test tone is active, not merely waiting for command unkey. */
#define RPTADV_REMOTE_TUNING 512u
/** Copied operating status; no caller storage is retained. */
typedef struct RptAdvRemoteSnapshot {
    uint32_t struct_size; /**< Exact writable record size. */
    uint32_t flags; /**< Snapshot presence and status flags. */
    RptAdvRemoteState requested; /**< Desired state. */
    RptAdvRemoteState applied; /**< Valid when APPLIED is present. */
    RptAdvRemoteState observed; /**< Valid when OBSERVED is present. */
    int32_t failure_code; /**< First dependency cause when FAILED is present. */
    uint64_t observed_fields; /**< Only these operating-patch fields are observations. */
    uint64_t scan_rejected_bursts; /**< Bursts rejected after scan-tune overlap. */
} RptAdvRemoteSnapshot;
/** One retained final result; request ID zero is not a user operation. */
typedef struct RptAdvRemoteCompletion {
    uint64_t request_id; /**< Original request identifier. */
    int32_t result; /**< Zero on successful application, otherwise operation error. */
} RptAdvRemoteCompletion;
/** Copied live scan status, independent of definition storage. */
typedef struct RptAdvRemoteScanSnapshot {
    uint32_t struct_size; /**< Exact record size. */
    uint32_t flags; /**< Active1, paused2, activity hold4, deadline8, error16. */
    uint32_t list; /**< Selected configured list. */
    uint32_t current_memory; /**< Last applied member, zero if none. */
    uint32_t direction; /**< Zero down, one up. */
    uint32_t failed_memory; /**< Most recent failed member, zero if none. */
    int32_t last_error; /**< Original error when flag16 is set. */
    uint32_t priority; /**< Priority member, zero if disabled. */
    uint64_t next_deadline_ms; /**< Valid when flag8 is set. */
    uint64_t dwell_ms; /**< Live dwell interval. */
    uint64_t resume_delay_ms; /**< Live quiet interval. */
    size_t skipped; /**< Skipped list positions. */
} RptAdvRemoteScanSnapshot;
/** Initial descriptor prefix. One non-audio owner serializes calls per handle. */
typedef struct RptAdvRemoteApi {
    uint32_t struct_size; /**< Complete table byte count. */
    uint32_t abi_version; /**< Capability ABI, one. */
    const char *capability_name; /**< Immutable rptadv.remote identifier. */
    int (*create)(const RptAdvRemoteSetup *, void **); /**< Copy/validate setup; zero on success. */
    void (*destroy)(void *); /**< Release one handle, or accept NULL. */
    int (*configured_memory)(const void *, uint32_t, RptAdvRemoteState *); /**< Copy a definition, not observed state. */
    int (*request)(void *, const RptAdvRemoteRequest *, uint64_t, uint64_t *); /**< Admit without physical I/O. */
    int (*advance)(void *, uint64_t, const RptAdvRemoteActivity *, const RptAdvRemoteBackend *); /**< Owner-only service. */
    int (*snapshot)(const void *, RptAdvRemoteSnapshot *); /**< Copy explicit applied/observed status. */
    int (*take_completion)(void *, RptAdvRemoteCompletion *); /**< Zero for a result, one if empty. */
    int (*retire)(void *); /**< Invalidate work; service once more to unkey before destruction. */
    int (*scan_snapshot)(const void *, RptAdvRemoteScanSnapshot *); /**< Zero for scan status; one if no selection. */
} RptAdvRemoteApi;
/** Return the immutable descriptor. Input pointers must remain readable/aligned. */
const RptAdvRemoteApi *rptadv_remote_api_v1(void);
#ifdef __cplusplus
}
#endif
#endif
