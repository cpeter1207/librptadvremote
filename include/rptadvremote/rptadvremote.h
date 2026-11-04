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
/** Capability flag: complete state readback is available. */
#define RPTADV_REMOTE_READBACK 8u
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
/** Independent encode/decode signaling; no external encoder programming implied. */
typedef struct RptAdvRemoteSignal {
    uint32_t source; /**< 0 disabled, 1 DSP, 2 external, 3 rig. */
    uint32_t enabled; /**< Exactly zero or one. */
    uint32_t ctcss_tenths_hz; /**< Tone in tenths of hertz, otherwise zero. */
    uint32_t dcs_code; /**< DCS code, otherwise zero. */
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
    RptAdvRemoteRanges receive; /**< Actual RX coverage. */
    RptAdvRemoteRanges transmit; /**< Actual TX coverage. */
    uint64_t mode_mask; /**< Modes the backend can apply. */
    uint32_t flags; /**< POWER and READBACK only. */
    uint32_t encode_sources; /**< Bit mask, one shifted by source identifier. */
    uint32_t decode_sources; /**< Independent decode provider mask. */
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
} RptAdvRemoteSetup;
/** Initial descriptor prefix. One non-audio owner serializes calls per handle. */
typedef struct RptAdvRemoteApi {
    uint32_t struct_size; /**< Complete table byte count. */
    uint32_t abi_version; /**< Capability ABI, one. */
    const char *capability_name; /**< Immutable rptadv.remote identifier. */
    int (*create)(const RptAdvRemoteSetup *, void **); /**< Copy/validate setup; zero on success. */
    void (*destroy)(void *); /**< Release one handle, or accept NULL. */
    int (*configured_memory)(const void *, uint32_t, RptAdvRemoteState *); /**< Copy a definition, not observed state. */
} RptAdvRemoteApi;
/** Return the immutable descriptor. Input pointers must remain readable/aligned. */
const RptAdvRemoteApi *rptadv_remote_api_v1(void);
#ifdef __cplusplus
}
#endif
#endif
