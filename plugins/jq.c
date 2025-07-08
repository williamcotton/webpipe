#include <jansson.h>
#include <jq.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <pthread.h>

// Memory arena type (forward declaration)
typedef struct MemoryArena MemoryArena;

// Arena allocation function types
typedef void* (*arena_alloc_func)(void* arena, size_t size);
typedef void (*arena_free_func)(void* arena);

// Thread-local storage for jq state
typedef struct {
    jq_state *jq;
    const char *last_program;  // Just store the pointer, don't copy
    int initialized;
} ThreadLocalJQ;

static pthread_key_t jq_tls_key;
static pthread_once_t jq_tls_once = PTHREAD_ONCE_INIT;

// Initialize thread-local storage key
static void jq_tls_init(void) {
    pthread_key_create(&jq_tls_key, NULL);
}

// Get thread-local jq state
static ThreadLocalJQ* get_thread_local_jq(void) {
    pthread_once(&jq_tls_once, jq_tls_init);
    ThreadLocalJQ *tls = pthread_getspecific(jq_tls_key);
    if (!tls) {
        tls = calloc(1, sizeof(ThreadLocalJQ));
        pthread_setspecific(jq_tls_key, tls);
    }
    if (!tls->initialized) {
        tls->jq = jq_init();
        tls->last_program = NULL;
        tls->initialized = 1;
    }
    return tls;
}

// Cleanup thread-local storage
static void cleanup_thread_local_jq(void *data) {
    ThreadLocalJQ *tls = (ThreadLocalJQ*)data;
    if (tls) {
        if (tls->jq) {
            jq_teardown(&tls->jq);
        }
        free(tls);
    }
}

// Convert jansson to jv
static jv jansson_to_jv(json_t *json) {
    switch (json_typeof(json)) {
    case JSON_OBJECT: {
        jv obj = jv_object();
        const char *key;
        json_t *value;
        json_object_foreach(json, key, value) {
            jv val = jansson_to_jv(value);
            if (!jv_is_valid(val)) {
                jv_free(obj);
                return val;
            }
            obj = jv_object_set(obj, jv_string(key), val);
        }
        return obj;
    }
    case JSON_ARRAY: {
        jv arr = jv_array();
        size_t index;
        json_t *value;
        json_array_foreach(json, index, value) {
            jv val = jansson_to_jv(value);
            if (!jv_is_valid(val)) {
                jv_free(arr);
                return val;
            }
            arr = jv_array_append(arr, val);
        }
        return arr;
    }
    case JSON_STRING:
        return jv_string(json_string_value(json));
    case JSON_INTEGER: {
        json_int_t val = json_integer_value(json);
        return jv_number((double)val);
    }
    case JSON_REAL:
        return jv_number(json_real_value(json));
    case JSON_TRUE:
        return jv_true();
    case JSON_FALSE:
        return jv_false();
    case JSON_NULL:
        return jv_null();
    default:
        return jv_invalid();
    }
}

// Convert jv to jansson
static json_t *jv_to_jansson(jv value) {
    if (!jv_is_valid(value)) {
        jv_free(value);
        return NULL;
    }

    json_t *result = NULL;
    switch (jv_get_kind(value)) {
    case JV_KIND_OBJECT: {
        result = json_object();
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wconditional-uninitialized"
        jv_object_foreach(value, k, v) {
            const char *key_str = jv_string_value(k);
            json_t *json_val = jv_to_jansson(v);
            if (json_val) {
                json_object_set_new(result, key_str, json_val);
            }
            jv_free(k);
        }
#pragma clang diagnostic pop
        break;
    }
    case JV_KIND_ARRAY: {
        result = json_array();
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wconditional-uninitialized"
        jv_array_foreach(value, i, v) {
            json_t *json_val = jv_to_jansson(v);
            if (json_val) {
                json_array_append_new(result, json_val);
            }
        }
#pragma clang diagnostic pop
        break;
    }
    case JV_KIND_STRING:
        result = json_string(jv_string_value(value));
        break;
    case JV_KIND_NUMBER:
        result = json_real(jv_number_value(value));
        break;
    case JV_KIND_TRUE:
        result = json_true();
        break;
    case JV_KIND_FALSE:
        result = json_false();
        break;
    case JV_KIND_NULL:
        result = json_null();
        break;
    case JV_KIND_INVALID:
        break;
    }
    
    jv_free(value);
    return result;
}

// Process jq filter
static jv process_jq_filter(jq_state *jq, json_t *input_json) {
    if (!jq || !input_json) return jv_invalid();

    jv input = jansson_to_jv(input_json);
    
    if (!jv_is_valid(input)) {
        jv jv_error = jv_invalid_get_msg(input);
        if (jv_is_valid(jv_error)) {
            fprintf(stderr, "JSON conversion error: %s\n", jv_string_value(jv_error));
            jv_free(jv_error);
        }
        return jv_invalid();
    }

    jq_start(jq, input, 0);  // input is consumed here
    
    jv filtered_result = jq_next(jq);

    if (!jv_is_valid(filtered_result)) {
        jv jv_error = jv_invalid_get_msg(filtered_result);
        if (jv_is_valid(jv_error)) {
            fprintf(stderr, "JQ execution error: %s\n", jv_string_value(jv_error));
            jv_free(jv_error);
        }
        return filtered_result;  // Already invalid and freed
    }

    // Drain any remaining results to prevent memory leaks
    jv next_result;
    while (jv_is_valid(next_result = jq_next(jq))) {
        jv_free(next_result);
    }
    jv_free(next_result);  // Free the invalid result that ended the loop

    return filtered_result;
}

// Plugin execute function
json_t *plugin_execute(json_t *input, void *arena, arena_alloc_func alloc_func, arena_free_func free_func, const char *jq_program) {
    ThreadLocalJQ *tls = get_thread_local_jq();
    if (!tls || !tls->jq) {
        fprintf(stderr, "jq: Failed to get thread-local jq state\n");
        json_t *result = json_object();
        json_object_set_new(result, "error", json_string("Failed to get JQ state"));
        return result;
    }
    // Copy program string into arena for jq_compile (if needed)
    size_t len = strlen(jq_program);
    char *arena_program = alloc_func(arena, len + 1);
    if (!arena_program) {
        json_t *result = json_object();
        json_object_set_new(result, "error", json_string("Arena OOM for jq program"));
        return result;
    }
    memcpy(arena_program, jq_program, len);
    arena_program[len] = '\0';
    // Check if we need to recompile (program changed or first time)
    if (!tls->last_program || strcmp(tls->last_program, arena_program) != 0) {
        tls->last_program = arena_program;
        if (jq_compile(tls->jq, arena_program) == 0) {
            fprintf(stderr, "jq: Failed to compile program: %s\n", arena_program);
            json_t *result = json_object();
            json_object_set_new(result, "error", json_string("Failed to compile JQ program"));
            return result;
        }
    }
    jv filtered_jv = process_jq_filter(tls->jq, input);
    if (!jv_is_valid(filtered_jv)) {
        json_t *result = json_object();
        json_object_set_new(result, "error", json_string("Failed to process JQ filter"));
        return result;
    }
    json_t *result = jv_to_jansson(filtered_jv);  // This frees filtered_jv
    if (!result) {
        json_t *error_result = json_object();
        json_object_set_new(error_result, "error", json_string("Failed to convert JQ result to JSON"));
        return error_result;
    }
    return result;
}

// Plugin cleanup function called when plugin is unloaded
__attribute__((destructor))
void plugin_destructor() {
    // Cleanup thread-local storage for current thread
    ThreadLocalJQ *tls = pthread_getspecific(jq_tls_key);
    if (tls) {
        cleanup_thread_local_jq(tls);
        pthread_setspecific(jq_tls_key, NULL);
    }
}