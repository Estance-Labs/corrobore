#include "corrobore_domain_provider.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int32_t write_json(
    const char *json,
    struct corrobore_domain_provider_buffer_v1 *output)
{
    size_t len;
    uint8_t *buffer;

    if (json == NULL || output == NULL) {
        return CORROBORE_DOMAIN_PROVIDER_STATUS_INVALID_ARGUMENT;
    }
    len = strlen(json);
    buffer = malloc(len);
    if (buffer == NULL) {
        return CORROBORE_DOMAIN_PROVIDER_STATUS_PROVIDER_ERROR;
    }
    memcpy(buffer, json, len);
    output->ptr = buffer;
    output->len = len;
    return CORROBORE_DOMAIN_PROVIDER_STATUS_OK;
}

static int32_t provider_metadata(
    struct corrobore_domain_provider_slice_v1 host_context_json,
    struct corrobore_domain_provider_buffer_v1 *output_json)
{
    (void)host_context_json;
    return write_json(
        "{\"schema_version\":\"1\",\"provider_id\":\"fr.estance.corrobore.domain.cti.fixture\",\"provider_version\":\"1.0.0-test\",\"domain\":\"cti\",\"thread_safe\":true,\"max_concurrency\":1,\"max_request_bytes\":1048576,\"max_response_bytes\":1048576,\"capabilities\":[{\"name\":\"node.validate\",\"version\":\"1\"}]}",
        output_json);
}

static int32_t provider_create(
    struct corrobore_domain_provider_slice_v1 config_json,
    void **provider_handle)
{
    (void)config_json;
    if (provider_handle == NULL) {
        return CORROBORE_DOMAIN_PROVIDER_STATUS_INVALID_ARGUMENT;
    }
    *provider_handle = malloc(1);
    return *provider_handle == NULL
        ? CORROBORE_DOMAIN_PROVIDER_STATUS_PROVIDER_ERROR
        : CORROBORE_DOMAIN_PROVIDER_STATUS_OK;
}

static int32_t provider_invoke(
    void *provider_handle,
    struct corrobore_domain_provider_slice_v1 request_json,
    struct corrobore_domain_provider_buffer_v1 *response_json)
{
    static const char request_id_key[] = "\"request_id\":\"";
    char request_id[129];
    char response[512];
    size_t request_id_key_len = sizeof(request_id_key) - 1;
    size_t request_id_start = request_json.len;
    size_t request_id_end;
    size_t index;
    int written;

    if (provider_handle == NULL) {
        return CORROBORE_DOMAIN_PROVIDER_STATUS_INVALID_ARGUMENT;
    }
    for (index = 0; index + request_id_key_len <= request_json.len; index++) {
        if (memcmp(request_json.ptr + index, request_id_key, request_id_key_len) == 0) {
            request_id_start = index + request_id_key_len;
            break;
        }
    }
    if (request_id_start == request_json.len) {
        return CORROBORE_DOMAIN_PROVIDER_STATUS_INVALID_ARGUMENT;
    }
    request_id_end = request_id_start;
    while (request_id_end < request_json.len && request_json.ptr[request_id_end] != '"') {
        request_id_end++;
    }
    if (request_id_end == request_json.len
        || request_id_end == request_id_start
        || request_id_end - request_id_start >= sizeof(request_id)) {
        return CORROBORE_DOMAIN_PROVIDER_STATUS_INVALID_ARGUMENT;
    }
    memcpy(
        request_id,
        request_json.ptr + request_id_start,
        request_id_end - request_id_start);
    request_id[request_id_end - request_id_start] = '\0';

    written = snprintf(
        response,
        sizeof(response),
        "{\"schema_version\":\"1\",\"request_id\":\"%s\",\"status\":\"accepted\",\"issues\":[],\"diagnostics\":null}",
        request_id);
    if (written < 0 || (size_t)written >= sizeof(response)) {
        return CORROBORE_DOMAIN_PROVIDER_STATUS_PROVIDER_ERROR;
    }
    return write_json(response, response_json);
}

static int32_t provider_health(
    void *provider_handle,
    struct corrobore_domain_provider_buffer_v1 *health_json)
{
    if (provider_handle == NULL) {
        return CORROBORE_DOMAIN_PROVIDER_STATUS_INVALID_ARGUMENT;
    }
    return write_json(
        "{\"schema_version\":\"1\",\"status\":\"ready\"}",
        health_json);
}

static void provider_destroy(void *provider_handle)
{
    free(provider_handle);
}

static void provider_free_buffer(
    struct corrobore_domain_provider_buffer_v1 buffer)
{
    free(buffer.ptr);
}

static const struct corrobore_domain_provider_api_v1 PROVIDER_API = {
    CORROBORE_DOMAIN_PROVIDER_ABI_MAJOR_V1,
    CORROBORE_DOMAIN_PROVIDER_ABI_MINOR_V1,
    sizeof(struct corrobore_domain_provider_api_v1),
    provider_metadata,
    provider_create,
    provider_invoke,
    provider_health,
    provider_destroy,
    provider_free_buffer
};

CORROBORE_DOMAIN_PROVIDER_EXPORT
const struct corrobore_domain_provider_api_v1 *
corrobore_domain_provider_get_api_v1(void)
{
    return &PROVIDER_API;
}
