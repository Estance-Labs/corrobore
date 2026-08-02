#include "corrobore_domain_provider.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int32_t write_json(const char *json, struct corrobore_domain_provider_buffer_v1 *output)
{
    size_t len = strlen(json);
    uint8_t *buffer = malloc(len);
    if (buffer == NULL || output == NULL) return CORROBORE_DOMAIN_PROVIDER_STATUS_PROVIDER_ERROR;
    memcpy(buffer, json, len);
    output->ptr = buffer;
    output->len = len;
    return CORROBORE_DOMAIN_PROVIDER_STATUS_OK;
}

static int32_t metadata(struct corrobore_domain_provider_slice_v1 ignored, struct corrobore_domain_provider_buffer_v1 *output)
{
    (void)ignored;
    return write_json("{\"schema_version\":\"1\",\"provider_id\":\"fr.estance.corrobore.domain.cti.report-to-stix-acceptance\",\"provider_version\":\"1.0.0-test\",\"domain\":\"cti\",\"thread_safe\":true,\"max_concurrency\":1,\"max_request_bytes\":1048576,\"max_response_bytes\":1048576,\"capabilities\":[{\"name\":\"node.validate\",\"version\":\"1\"}]}", output);
}

static int32_t create(struct corrobore_domain_provider_slice_v1 ignored, void **handle)
{
    (void)ignored;
    if (handle == NULL) return CORROBORE_DOMAIN_PROVIDER_STATUS_INVALID_ARGUMENT;
    *handle = malloc(1);
    return *handle == NULL ? CORROBORE_DOMAIN_PROVIDER_STATUS_PROVIDER_ERROR : CORROBORE_DOMAIN_PROVIDER_STATUS_OK;
}

static int32_t invoke(void *handle, struct corrobore_domain_provider_slice_v1 request, struct corrobore_domain_provider_buffer_v1 *output)
{
    char *json;
    char request_id[96] = {0};
    char response[2048];
    const char *marker;
    const char *end;
    const char *issues = "";
    double confidence = 1.0;
    int missing_evidence;
    int low_confidence;

    if (handle == NULL || request.ptr == NULL || output == NULL) return CORROBORE_DOMAIN_PROVIDER_STATUS_INVALID_ARGUMENT;
    json = malloc(request.len + 1);
    if (json == NULL) return CORROBORE_DOMAIN_PROVIDER_STATUS_PROVIDER_ERROR;
    memcpy(json, request.ptr, request.len);
    json[request.len] = '\0';

    marker = strstr(json, "\"request_id\":\"");
    if (marker == NULL) { free(json); return CORROBORE_DOMAIN_PROVIDER_STATUS_INVALID_ARGUMENT; }
    marker += strlen("\"request_id\":\"");
    end = strchr(marker, '\"');
    if (end == NULL || (size_t)(end - marker) >= sizeof(request_id)) { free(json); return CORROBORE_DOMAIN_PROVIDER_STATUS_INVALID_ARGUMENT; }
    memcpy(request_id, marker, (size_t)(end - marker));

    marker = strstr(json, "\"confidence\":");
    if (marker != NULL) confidence = strtod(marker + strlen("\"confidence\":"), NULL);
    missing_evidence = strstr(json, "\"evidence_refs\":[]") != NULL;
    low_confidence = marker != NULL && confidence < 0.8;
    if (low_confidence && missing_evidence) {
        issues = "{\"code\":\"CTI_CONFIDENCE_TOO_LOW\",\"message\":\"native confidence is below 0.8\",\"field\":\"confidence\",\"severity\":\"error\",\"node_id\":null},{\"code\":\"CTI_EVIDENCE_REQUIRED\",\"message\":\"native evidence is required\",\"field\":\"evidence_refs\",\"severity\":\"error\",\"node_id\":null}";
    } else if (low_confidence) {
        issues = "{\"code\":\"CTI_CONFIDENCE_TOO_LOW\",\"message\":\"native confidence is below 0.8\",\"field\":\"confidence\",\"severity\":\"error\",\"node_id\":null}";
    } else if (missing_evidence) {
        issues = "{\"code\":\"CTI_EVIDENCE_REQUIRED\",\"message\":\"native evidence is required\",\"field\":\"evidence_refs\",\"severity\":\"error\",\"node_id\":null}";
    }
    snprintf(response, sizeof(response), "{\"schema_version\":\"1\",\"request_id\":\"%s\",\"status\":\"accepted\",\"issues\":[%s],\"diagnostics\":null}", request_id, issues);
    free(json);
    return write_json(response, output);
}

static int32_t health(void *handle, struct corrobore_domain_provider_buffer_v1 *output)
{
    if (handle == NULL) return CORROBORE_DOMAIN_PROVIDER_STATUS_INVALID_ARGUMENT;
    return write_json("{\"schema_version\":\"1\",\"status\":\"ready\"}", output);
}

static void destroy(void *handle) { free(handle); }
static void free_buffer(struct corrobore_domain_provider_buffer_v1 buffer) { free(buffer.ptr); }

static const struct corrobore_domain_provider_api_v1 API = {
    CORROBORE_DOMAIN_PROVIDER_ABI_MAJOR_V1,
    CORROBORE_DOMAIN_PROVIDER_ABI_MINOR_V1,
    sizeof(struct corrobore_domain_provider_api_v1),
    metadata, create, invoke, health, destroy, free_buffer
};

CORROBORE_DOMAIN_PROVIDER_EXPORT
const struct corrobore_domain_provider_api_v1 *corrobore_domain_provider_get_api_v1(void) { return &API; }
