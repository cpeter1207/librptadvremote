/** @file
 * @brief Verify the installed remote-base setup and opaque ownership.
 */
#include <rptadvremote/rptadvremote.h>
#include <assert.h>
#include <string.h>

typedef struct BackendFixture {
    RptAdvRemoteState state;
    uint32_t keyed;
    uint32_t writes;
} BackendFixture;

static int apply_state(void *context, const RptAdvRemoteState *state)
{
    BackendFixture *backend = context;
    backend->state = *state;
    backend->writes++;
    return 0;
}

static int read_state(void *context, RptAdvRemoteState *state)
{
    const BackendFixture *backend = context;
    *state = backend->state;
    return 0;
}

static int set_ptt(void *context, uint32_t keyed)
{
    BackendFixture *backend = context;
    backend->keyed = keyed;
    return 0;
}

int main(void)
{
    const RptAdvRemoteApi *api = rptadv_remote_api_v1();
    const RptAdvRemoteRange range = {144000000, 148000000};
    const RptAdvRemoteMemory memory = {
        .selector = 1,
        .name = "local",
        .name_length = 5,
        .state = {
            .receive_hz = 146520000,
            .transmit_hz = 146520000,
            .flags = RPTADV_REMOTE_TX | RPTADV_REMOTE_POWER,
            .mode = RPTADV_REMOTE_FM,
            .power_mw = 5000,
        },
    };
    RptAdvRemoteSetup setup = {
        .struct_size = sizeof(setup),
        .abi_version = 1,
        .limits = {
            .receive = {&range, 1},
            .transmit = {&range, 1},
            .mode_mask = UINT64_C(1) << RPTADV_REMOTE_FM,
            .flags = RPTADV_REMOTE_POWER,
            .maximum_power_mw = 10000,
        },
        .capabilities = {
            .receive = {&range, 1},
            .transmit = {&range, 1},
            .mode_mask = UINT64_C(1) << RPTADV_REMOTE_FM,
            .flags = RPTADV_REMOTE_POWER,
            .readback_fields = RPTADV_REMOTE_PATCH_ALL,
        },
        .memories = {&memory, 1},
        .startup_memory = 1,
    };
    void *handle = NULL;
    RptAdvRemoteState output = {0};

    assert(api != NULL && api->abi_version == 1);
    assert(api->struct_size == sizeof(*api));
    assert(strcmp(api->capability_name, "rptadv.remote") == 0);
    assert(api->create(NULL, &handle) == -1 && handle == NULL);
    assert(api->create(&setup, NULL) == -1);
    setup.struct_size--;
    assert(api->create(&setup, &handle) == -1 && handle == NULL);
    setup.struct_size++;
    setup.abi_version++;
    assert(api->create(&setup, &handle) == -1 && handle == NULL);
    setup.abi_version--;
    assert(api->create(&setup, &handle) == 0 && handle != NULL);
    assert(api->configured_memory(handle, 1, &output) == 0);
    assert(output.receive_hz == memory.state.receive_hz);
    assert(output.transmit_hz == memory.state.transmit_hz);
    assert(output.power_mw == memory.state.power_mw);
    assert(api->configured_memory(handle, 2, &output) == -7);
    assert(api->configured_memory(handle, 1, NULL) == -1);
    assert(api->configured_memory(NULL, 1, &output) == -1);

    BackendFixture fixture = {0};
    const RptAdvRemoteBackend backend = {
        .struct_size = sizeof(backend),
        .abi_version = 1,
        .context = &fixture,
        .read_state = read_state,
        .apply_state = apply_state,
        .set_ptt = set_ptt,
    };
    RptAdvRemoteActivity activity = {.struct_size = sizeof(activity)};
    RptAdvRemoteSnapshot snapshot = {.struct_size = sizeof(snapshot)};
    const RptAdvRemoteRequest request = {
        .struct_size = sizeof(request), .abi_version = 1,
        .kind = RPTADV_REMOTE_RECALL, .selector = 1,
    };
    uint64_t request_id = 0;
    RptAdvRemoteCompletion completion = {0};
    assert(api->advance(handle, 0, &activity, &backend) == 0);
    assert(fixture.writes == 1 && fixture.keyed == 0);
    assert(api->snapshot(handle, &snapshot) == 0);
    assert((snapshot.flags & RPTADV_REMOTE_APPLIED) != 0);
    assert((snapshot.flags & RPTADV_REMOTE_INHIBITED) == 0);
    assert(api->request(handle, &request, 1, &request_id) == 0 && request_id != 0);
    activity.flags = RPTADV_REMOTE_PHYSICAL_PTT;
    assert(api->advance(handle, 1, &activity, &backend) == 0);
    assert(fixture.writes == 1);
    activity.flags = 0;
    assert(api->advance(handle, 2, &activity, &backend) == 0);
    assert(fixture.writes == 2);
    assert(api->take_completion(handle, &completion) == 0);
    assert(completion.request_id == request_id && completion.result == 0);
    assert(api->take_completion(handle, &completion) == 1);
    assert(api->retire(handle) == 0);
    assert(api->advance(handle, 3, &activity, &backend) == -10);
    api->destroy(handle);
    api->destroy(NULL);
    return 0;
}
