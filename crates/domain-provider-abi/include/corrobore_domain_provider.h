#ifndef CORROBORE_DOMAIN_PROVIDER_H
#define CORROBORE_DOMAIN_PROVIDER_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#if defined(_WIN32)
#define CORROBORE_DOMAIN_PROVIDER_EXPORT __declspec(dllexport)
#else
#define CORROBORE_DOMAIN_PROVIDER_EXPORT __attribute__((visibility("default")))
#endif

#define CORROBORE_DOMAIN_PROVIDER_ABI_MAJOR_V1 UINT16_C(1)
/* Minor 2 adds the optional "claim.verify" JSON capability, request/response
 * payloads, and its optional metadata-level "deterministic" boolean. The
 * function table remains unchanged and hosts at minor 2 accept supported
 * providers built against minor 1. claim.verify/1 request payload:
 * {claim,links,observations,sources,evidence_records,as_of:{valid_time,system_time}}
 * response payload:
 * {result:"pass"|"fail"|"inconclusive",rationale,limits,evidence_consumed}. */
#define CORROBORE_DOMAIN_PROVIDER_ABI_MINOR_V1 UINT16_C(2)

enum corrobore_domain_provider_status_v1 {
    CORROBORE_DOMAIN_PROVIDER_STATUS_OK = 0,
    CORROBORE_DOMAIN_PROVIDER_STATUS_INVALID_ARGUMENT = 1,
    CORROBORE_DOMAIN_PROVIDER_STATUS_INVALID_REQUEST = 2,
    CORROBORE_DOMAIN_PROVIDER_STATUS_UNSUPPORTED_CAPABILITY = 3,
    CORROBORE_DOMAIN_PROVIDER_STATUS_PROVIDER_ERROR = 4,
    CORROBORE_DOMAIN_PROVIDER_STATUS_RESPONSE_TOO_LARGE = 5
};

/* Borrowed input. ptr is valid for exactly one call and must not be retained. */
struct corrobore_domain_provider_slice_v1 {
    const uint8_t *ptr;
    size_t len;
};

/* Provider-owned output. Release exactly once with the same API free_buffer. */
struct corrobore_domain_provider_buffer_v1 {
    uint8_t *ptr;
    size_t len;
};

typedef int32_t (*corrobore_domain_provider_metadata_v1_fn)(
    struct corrobore_domain_provider_slice_v1 host_context_json,
    struct corrobore_domain_provider_buffer_v1 *output_json);

typedef int32_t (*corrobore_domain_provider_create_v1_fn)(
    struct corrobore_domain_provider_slice_v1 config_json,
    void **provider_handle);

typedef int32_t (*corrobore_domain_provider_invoke_v1_fn)(
    void *provider_handle,
    struct corrobore_domain_provider_slice_v1 request_json,
    struct corrobore_domain_provider_buffer_v1 *response_json);

typedef int32_t (*corrobore_domain_provider_health_v1_fn)(
    void *provider_handle,
    struct corrobore_domain_provider_buffer_v1 *health_json);

typedef void (*corrobore_domain_provider_destroy_v1_fn)(void *provider_handle);

typedef void (*corrobore_domain_provider_free_buffer_v1_fn)(
    struct corrobore_domain_provider_buffer_v1 buffer);

/*
 * v1 is prefix-versioned. Hosts read abi_major, abi_minor, and struct_size
 * first, and must not read beyond the smaller of struct_size and their known
 * table size. All six function pointers are mandatory for ABI v1.
 *
 * Provider implementations must catch all language exceptions/panics. No
 * unwind may cross these C calls. A handle returned by create is destroyed
 * exactly once before the library is unloaded. A provider owns every output
 * buffer until the host calls its matching free_buffer function.
 */
struct corrobore_domain_provider_api_v1 {
    uint16_t abi_major;
    uint16_t abi_minor;
    size_t struct_size;
    corrobore_domain_provider_metadata_v1_fn metadata;
    corrobore_domain_provider_create_v1_fn create;
    corrobore_domain_provider_invoke_v1_fn invoke;
    corrobore_domain_provider_health_v1_fn health;
    corrobore_domain_provider_destroy_v1_fn destroy;
    corrobore_domain_provider_free_buffer_v1_fn free_buffer;
};

/* The returned immutable table remains valid until the library is unloaded. */
CORROBORE_DOMAIN_PROVIDER_EXPORT
const struct corrobore_domain_provider_api_v1 *
corrobore_domain_provider_get_api_v1(void);

#ifdef __cplusplus
}
#endif

#endif /* CORROBORE_DOMAIN_PROVIDER_H */
