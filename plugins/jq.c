#include <jansson.h>
#include <jq.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <pthread.h>

// Memory arena type (forward declaration)
typedef struct MemoryArena MemoryArena;

// Thread-local storage for jq state and compiled programs
typedef struct {
    jq_state *jq;
    const char *last_program;  // Just store the pointer, don't copy
    int initialized;
    // Simple result cache for identical inputs
    json_t *last_result;
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
        tls->last_result = NULL;
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
        if (tls->last_result) {
            json_decref(tls->last_result);
        }
        free(tls);
    }
}

// Optimized conversion functions
json_t *jv_to_jansson(jv value) {
    switch (jv_get_kind(value)) {
        case JV_KIND_NULL:
            return json_null();
        case JV_KIND_FALSE:
            return json_false();
        case JV_KIND_TRUE:
            return json_true();
        case JV_KIND_NUMBER:
            return json_real(jv_number_value(value));
        case JV_KIND_STRING: {
            const char *str = jv_string_value(value);
            json_t *result = json_string(str);
            return result;
        }
        case JV_KIND_ARRAY: {
            json_t *array = json_array();
            jv_array_foreach(value, i, item) {
                json_array_append_new(array, jv_to_jansson(item));
            }
            return array;
        }
        case JV_KIND_OBJECT: {
            json_t *object = json_object();
            jv_object_foreach(value, key, val) {
                const char *key_str = jv_string_value(key);
                json_object_set_new(object, key_str, jv_to_jansson(val));
            }
            return object;
        }
        default:
            return json_null();
    }
}

jv jansson_to_jv(json_t *json) {
    if (json_is_null(json)) {
        return jv_null();
    } else if (json_is_true(json)) {
        return jv_true();
    } else if (json_is_false(json)) {
        return jv_false();
    } else if (json_is_number(json)) {
        return jv_number(json_number_value(json));
    } else if (json_is_string(json)) {
        return jv_string(json_string_value(json));
    } else if (json_is_array(json)) {
        jv array = jv_array();
        size_t index;
        json_t *value;
        json_array_foreach(json, index, value) {
            array = jv_array_append(array, jansson_to_jv(value));
        }
        return array;
    } else if (json_is_object(json)) {
        jv object = jv_object();
        const char *key;
        json_t *value;
        json_object_foreach(json, key, value) {
            object = jv_object_set(object, jv_string(key), jansson_to_jv(value));
        }
        return object;
    }
    return jv_null();
}

// Plugin execute function
json_t *plugin_execute(json_t *input, MemoryArena *arena, const char *jq_program) {
    ThreadLocalJQ *tls = get_thread_local_jq();
    if (!tls || !tls->jq) {
        fprintf(stderr, "jq: Failed to get thread-local jq state\n");
        return NULL;
    }
    
    // Check if we need to recompile (program changed or first time)
    if (!tls->last_program || strcmp(tls->last_program, jq_program) != 0) {
        // Clear cache
        if (tls->last_result) {
            json_decref(tls->last_result);
            tls->last_result = NULL;
        }
        
        // Store the program pointer (it's stable during the request)
        tls->last_program = jq_program;
        
        // Compile new program
        if (jq_compile(tls->jq, jq_program) == 0) {
            fprintf(stderr, "jq: Failed to compile program: %s\n", jq_program);
            return NULL;
        }
    }
    
    // For simple cases like identity (`.`), we can cache the result
    // This is a simple optimization for common cases
    if (strcmp(jq_program, ".") == 0 && tls->last_result) {
        return json_incref(tls->last_result);
    }
    
    // Convert input to jv
    jv jv_input = jansson_to_jv(input);
    
    // Execute jq program
    jq_start(tls->jq, jv_input, 0);
    
    jv result = jq_next(tls->jq);
    json_t *output = NULL;
    
    if (jv_is_valid(result)) {
        output = jv_to_jansson(result);
        jv_free(result);
        
        // Cache the result for identity operations
        if (strcmp(jq_program, ".") == 0) {
            if (tls->last_result) {
                json_decref(tls->last_result);
            }
            tls->last_result = json_incref(output);
        }
    }
    
    return output;
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