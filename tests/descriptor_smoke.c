/** @file
 * @brief Verify the installed remote-base setup and opaque ownership.
 */
#include <rptadvremote/rptadvremote.h>
#include <assert.h>
#include <string.h>

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
    api->destroy(handle);
    api->destroy(NULL);
    return 0;
}
