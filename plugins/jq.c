#include <jansson.h>
#include <jq.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

// Memory arena type (forward declaration)
typedef struct MemoryArena MemoryArena;

// Conversion functions
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
    // Create a new jq state for each execution (thread-safe)
    jq_state *jq = jq_init();
    if (!jq) {
        fprintf(stderr, "jq: Failed to initialize jq state\n");
        return NULL;
    }
    
    // Compile jq program
    if (jq_compile(jq, jq_program) == 0) {
        fprintf(stderr, "jq: Failed to compile program: %s\n", jq_program);
        jq_teardown(&jq);
        return NULL;
    }
    
    // Convert input to jv
    jv jv_input = jansson_to_jv(input);
    
    // Execute jq program
    jq_start(jq, jv_input, 0);
    
    jv result = jq_next(jq);
    json_t *output = NULL;
    
    if (jv_is_valid(result)) {
        output = jv_to_jansson(result);
        jv_free(result);
    }
    
    // Clean up jq state
    jq_teardown(&jq);
    
    return output;
}