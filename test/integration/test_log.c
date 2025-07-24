#include "../unity/unity.h"
#include "../helpers/test_utils.h"
#include "../../src/wp.h"
#include <string.h>
#include <unistd.h>
#include <time.h>
#include <stdio.h>
#include <sys/stat.h>

// Load the actual log middleware
#include <dlfcn.h>
static void *log_middleware_handle = NULL;
static json_t *(*log_middleware_execute)(json_t *, void *, arena_alloc_func, arena_free_func, const char *, json_t *, char **, json_t *) = NULL;
static void (*log_middleware_post_execute)(json_t *, void *, arena_alloc_func, json_t *) = NULL;
static int (*log_middleware_init)(json_t *) = NULL;
static void (*log_middleware_cleanup)(void) = NULL;

// Arena allocator wrapper for jansson in tests
static MemoryArena *test_jansson_arena = NULL;

static void *test_jansson_malloc(size_t size) {
    if (test_jansson_arena) {
        return arena_alloc(test_jansson_arena, size);
    }
    return malloc(size);
}

static void test_jansson_free(void *ptr) {
    // Arena memory doesn't need explicit freeing
    if (!test_jansson_arena) {
        free(ptr);
    }
}

static int load_log_middleware(void) {
    if (log_middleware_handle) return 0; // Already loaded
    
    log_middleware_handle = dlopen("./middleware/log.so", RTLD_LAZY);
    if (!log_middleware_handle) {
        fprintf(stderr, "Failed to load log middleware: %s\n", dlerror());
        return -1;
    }
    
    // Load middleware_execute function
    void *execute_func = dlsym(log_middleware_handle, "middleware_execute");
    log_middleware_execute = (json_t *(*)(json_t *, void *, arena_alloc_func, arena_free_func, const char *, json_t *, char **, json_t *))
                        (uintptr_t)execute_func;
    if (!execute_func) {
        fprintf(stderr, "Failed to find middleware_execute in log middleware: %s\n", dlerror());
        dlclose(log_middleware_handle);
        log_middleware_handle = NULL;
        return -1;
    }
    
    // Load middleware_post_execute function
    void *post_execute_func = dlsym(log_middleware_handle, "middleware_post_execute");
    log_middleware_post_execute = (void (*)(json_t *, void *, arena_alloc_func, json_t *))
                                   (uintptr_t)post_execute_func;
    if (!post_execute_func) {
        fprintf(stderr, "Failed to find middleware_post_execute in log middleware: %s\n", dlerror());
        dlclose(log_middleware_handle);
        log_middleware_handle = NULL;
        return -1;
    }
    
    // Load middleware_init function
    void *init_func = dlsym(log_middleware_handle, "middleware_init");
    log_middleware_init = (int (*)(json_t *))
                           (uintptr_t)init_func;
    if (!init_func) {
        fprintf(stderr, "Failed to find middleware_init in log middleware: %s\n", dlerror());
        dlclose(log_middleware_handle);
        log_middleware_handle = NULL;
        return -1;
    }
    
    // Load middleware_cleanup function (optional)
    void *cleanup_func = dlsym(log_middleware_handle, "middleware_cleanup");
    log_middleware_cleanup = (void (*)(void))(uintptr_t)cleanup_func;
    // Don't fail if cleanup function doesn't exist - it's optional
    
    return 0;
}

static void unload_log_middleware(void) {
    if (log_middleware_handle) {
        dlclose(log_middleware_handle);
        log_middleware_handle = NULL;
        log_middleware_execute = NULL;
        log_middleware_post_execute = NULL;
        log_middleware_init = NULL;
        log_middleware_cleanup = NULL;
    }
}

void setUp(void) {
    if (load_log_middleware() != 0) {
        TEST_FAIL_MESSAGE("Failed to load log middleware");
    }
    
    // Create arena for jansson allocations to prevent test leaks
    test_jansson_arena = create_test_arena(8192);
    
    // Configure jansson to use arena allocators like in runtime
    json_set_alloc_funcs(test_jansson_malloc, test_jansson_free);
    
    // Initialize log middleware with test configuration
    json_t *config = json_object();
    json_object_set_new(config, "enabled", json_boolean(1));
    json_object_set_new(config, "output", json_string("stdout"));
    json_object_set_new(config, "format", json_string("json"));
    json_object_set_new(config, "level", json_string("info"));
    json_object_set_new(config, "includeBody", json_boolean(0));
    json_object_set_new(config, "includeHeaders", json_boolean(1)); 
    json_object_set_new(config, "maxBodySize", json_integer(1024));
    json_object_set_new(config, "timestamp", json_boolean(1));
    
    if (log_middleware_init(config) != 0) {
        json_decref(config);
        TEST_FAIL_MESSAGE("Failed to initialize log middleware");
    }
    json_decref(config);
}

void tearDown(void) {
    // Clean up log middleware before unloading
    if (log_middleware_cleanup) {
        log_middleware_cleanup();
    }
    unload_log_middleware();
    
    // Restore default jansson allocators
    json_set_alloc_funcs(malloc, free);
    
    // Clean up jansson arena
    if (test_jansson_arena) {
        destroy_test_arena(test_jansson_arena);
        test_jansson_arena = NULL;
    }
}

static void test_log_middleware_basic_passthrough(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *input = create_test_request("GET", "/test");
    const char *config = "level: info\nenabled: true\n";
    char *content_type = NULL;
    
    json_t *output = log_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, 
                                             config, NULL, &content_type, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    TEST_ASSERT_EQUAL_PTR(input, output); // Should return same object (passthrough)
    
    // Should have log metadata added
    json_t *metadata = json_object_get(output, "_metadata");
    TEST_ASSERT_NOT_NULL(metadata);
    json_t *log_metadata = json_object_get(metadata, "log");
    TEST_ASSERT_NOT_NULL(log_metadata);
    
    json_t *log_enabled = json_object_get(log_metadata, "enabled");
    TEST_ASSERT_NOT_NULL(log_enabled);
    TEST_ASSERT_TRUE(json_boolean_value(log_enabled));
    
    json_t *start_time = json_object_get(log_metadata, "start_time");
    TEST_ASSERT_NOT_NULL(start_time);
    TEST_ASSERT_TRUE(json_is_real(start_time));
    
    if (output != input) {
        json_decref(output);
    }
    json_decref(input);
    destroy_test_arena(arena);
}

static void test_log_middleware_disabled(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *input = create_test_request("GET", "/test");
    const char *config = "enabled: false\nlevel: info\n";
    char *content_type = NULL;
    
    json_t *output = log_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL,
                                             config, NULL, &content_type, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    TEST_ASSERT_EQUAL_PTR(input, output); // Should return same object
    
    // Should NOT have any metadata when disabled
    json_t *metadata = json_object_get(output, "_metadata");
    TEST_ASSERT_NULL(metadata);
    
    if (output != input) {
        json_decref(output);
    }
    json_decref(input);
    destroy_test_arena(arena);
}

static void test_log_middleware_configuration_parsing(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *input = create_test_request("GET", "/test");
    
    // Test various configuration formats
    const char *config1 = "level: debug\nenabled: true\nincludeBody: true\n";
    const char *config3 = "# Comment line\nlevel: warn\n\n# Another comment\nincludeHeaders: false\n";
    
    char *content_type = NULL;
    
    // Test config 1
    json_t *output1 = log_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL,
                                              config1, NULL, &content_type, NULL);
    TEST_ASSERT_NOT_NULL(output1);
    
    json_t *output1_metadata = json_object_get(output1, "_metadata");
    TEST_ASSERT_NOT_NULL(output1_metadata);
    json_t *metadata1 = json_object_get(output1_metadata, "log");
    TEST_ASSERT_NOT_NULL(metadata1);
    
    json_t *level1 = json_object_get(metadata1, "level");
    TEST_ASSERT_EQUAL_STRING("debug", json_string_value(level1));
    
    json_t *include_body1 = json_object_get(metadata1, "include_body");
    TEST_ASSERT_TRUE(json_boolean_value(include_body1));
    
    // Test config 3 (with comments)
    json_t *output3 = log_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL,
                                              config3, NULL, &content_type, NULL);
    TEST_ASSERT_NOT_NULL(output3);
    
    json_t *output3_metadata = json_object_get(output3, "_metadata");
    TEST_ASSERT_NOT_NULL(output3_metadata);
    json_t *metadata3 = json_object_get(output3_metadata, "log");
    TEST_ASSERT_NOT_NULL(metadata3);
    
    json_t *level3 = json_object_get(metadata3, "level");
    TEST_ASSERT_EQUAL_STRING("warn", json_string_value(level3));
    
    json_t *include_headers3 = json_object_get(metadata3, "include_headers");
    TEST_ASSERT_FALSE(json_boolean_value(include_headers3));
    
    if (output1 != input) {
        json_decref(output1);
    }
    if (output3 != input) {
        json_decref(output3);
    }
    json_decref(input);
    destroy_test_arena(arena);
}

static void test_log_middleware_post_execute_logging(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    // Create a response to log
    json_t *final_response = json_object();
    json_object_set_new(final_response, "message", json_string("test response"));
    json_object_set_new(final_response, "timestamp", json_integer(time(NULL)));
    
    // Add original request
    json_t *original_request = create_test_request("GET", "/test/logging");
    json_object_set_new(final_response, "originalRequest", original_request);
    
    // Add log metadata
    json_t *log_metadata = json_object();
    json_object_set_new(log_metadata, "start_time", json_real(1000.0));
    json_object_set_new(log_metadata, "level", json_string("info"));
    json_object_set_new(log_metadata, "include_body", json_boolean(0));
    json_object_set_new(log_metadata, "include_headers", json_boolean(1));
    json_object_set_new(log_metadata, "enabled", json_boolean(1));
    json_t *metadata = json_object();
    json_object_set_new(metadata, "log", log_metadata);
    json_object_set_new(final_response, "_metadata", metadata);
    
    // Call post execute (this should output log to stdout)
    log_middleware_post_execute(final_response, arena, get_arena_alloc_wrapper(), NULL);
    
    // Verify the log metadata was removed from response
    json_t *response_metadata = json_object_get(final_response, "_metadata");
    json_t *remaining_metadata = NULL;
    if (response_metadata) {
        remaining_metadata = json_object_get(response_metadata, "log");
    }
    TEST_ASSERT_NULL(remaining_metadata);
    
    json_decref(final_response);
    destroy_test_arena(arena);
}

static void test_log_middleware_null_input_handling(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    const char *config = "enabled: true\nlevel: info\n";
    char *content_type = NULL;
    
    // Log middleware should handle null input gracefully
    json_t *output = log_middleware_execute(NULL, arena, get_arena_alloc_wrapper(), NULL,
                                             config, NULL, &content_type, NULL);
    
    // Should return null for null input
    TEST_ASSERT_NULL(output);
    
    destroy_test_arena(arena);
}

static void test_log_middleware_null_config_handling(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *input = create_test_request("GET", "/test");
    char *content_type = NULL;
    
    // Should handle null config by using defaults
    json_t *output = log_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL,
                                             NULL, NULL, &content_type, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    // With null config, should still work with global defaults
    
    if (output != input) {
        json_decref(output);
    }
    json_decref(input);
    destroy_test_arena(arena);
}

static void test_log_middleware_empty_config_handling(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *input = create_test_request("GET", "/test");
    const char *config = "";
    char *content_type = NULL;
    
    // Should handle empty config by using defaults
    json_t *output = log_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL,
                                             config, NULL, &content_type, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    
    if (output != input) {
        json_decref(output);
    }
    json_decref(input);
    destroy_test_arena(arena);
}

static void test_log_middleware_memory_arena_usage(void) {
    MemoryArena *arena = create_test_arena(1024);
    size_t initial_used = arena->used;
    
    json_t *input = create_test_request("GET", "/test");
    const char *config = "level: debug\nenabled: true\n";
    char *content_type = NULL;
    
    json_t *output = log_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL,
                                             config, NULL, &content_type, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    
    // Arena should have been used for metadata allocation
    TEST_ASSERT_GREATER_THAN(initial_used, arena->used);
    
    if (output != input) {
        json_decref(output);
    }
    json_decref(input);
    destroy_test_arena(arena);
}

static void test_log_middleware_full_pipeline_simulation(void) {
    MemoryArena *arena = create_test_arena(2048);
    
    // Simulate a complete log execute -> pipeline -> post execute cycle
    
    // Step 1: Log middleware execute (adds metadata)
    json_t *input = create_test_request("GET", "/test/pipeline");
    json_t *params = json_object();
    json_object_set_new(params, "id", json_string("123"));
    json_object_set_new(input, "params", params);
    
    const char *config = "level: info\nenabled: true\nincludeBody: false\n";
    char *content_type = NULL;
    
    json_t *log_result = log_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL,
                                                    config, NULL, &content_type, NULL);
    
    TEST_ASSERT_NOT_NULL(log_result);
    TEST_ASSERT_EQUAL_PTR(input, log_result); // Should be passthrough
    
    // Verify log metadata was added
    json_t *log_result_metadata = json_object_get(log_result, "_metadata");
    TEST_ASSERT_NOT_NULL(log_result_metadata);
    json_t *log_metadata = json_object_get(log_result_metadata, "log");
    TEST_ASSERT_NOT_NULL(log_metadata);
    
    // Step 2: Simulate pipeline execution (jq middleware would do this)
    json_t *final_response = json_object();
    json_object_set_new(final_response, "id", json_string("123"));
    json_object_set_new(final_response, "data", json_string("processed data"));
    json_object_set_new(final_response, "timestamp", json_integer(time(NULL)));
    
    // Add the original request and log metadata from step 1
    json_object_set_new(final_response, "originalRequest", json_deep_copy(input));
    json_t *final_metadata = json_object();
    json_object_set_new(final_metadata, "log", json_deep_copy(log_metadata));
    json_object_set_new(final_response, "_metadata", final_metadata);
    
    // Step 3: Post execute to log the response
    log_middleware_post_execute(final_response, arena, get_arena_alloc_wrapper(), NULL);
    
    // Step 4: Verify cleanup - log metadata should be removed
    json_t *response_metadata = json_object_get(final_response, "_metadata");
    json_t *remaining_metadata = NULL;
    if (response_metadata) {
        remaining_metadata = json_object_get(response_metadata, "log");
    }
    TEST_ASSERT_NULL(remaining_metadata);
    
    json_decref(input);
    json_decref(final_response);
    destroy_test_arena(arena);
}

static void test_log_middleware_different_log_levels(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *input = create_test_request("GET", "/test");
    char *content_type = NULL;
    
    // Test different log levels
    const char *levels[] = {"debug", "info", "warn", "error"};
    
    for (size_t i = 0; i < sizeof(levels) / sizeof(levels[0]); i++) {
        char config[64];
        snprintf(config, sizeof(config), "level: %s\nenabled: true\n", levels[i]);
        
        json_t *output = log_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL,
                                                 config, NULL, &content_type, NULL);
        
        TEST_ASSERT_NOT_NULL(output);
        
        json_t *output_metadata = json_object_get(output, "_metadata");
        json_t *metadata = NULL;
        if (output_metadata) {
            metadata = json_object_get(output_metadata, "log");
        }
        TEST_ASSERT_NOT_NULL(metadata);
        
        json_t *level = json_object_get(metadata, "level");
        TEST_ASSERT_EQUAL_STRING(levels[i], json_string_value(level));
        
        if (output != input) {
            json_decref(output);
        }
    }
    
    json_decref(input);
    destroy_test_arena(arena);
}

static void test_log_middleware_body_and_header_flags(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *input = create_test_request("POST", "/test");
    json_t *body = json_object();
    json_object_set_new(body, "username", json_string("testuser"));
    json_object_set_new(input, "body", body);
    
    json_t *headers = json_object();
    json_object_set_new(headers, "Content-Type", json_string("application/json"));
    json_object_set_new(headers, "Authorization", json_string("Bearer token123"));
    json_object_set_new(input, "headers", headers);
    
    char *content_type = NULL;
    
    // Test with body and headers enabled
    const char *config1 = "level: info\nenabled: true\nincludeBody: true\nincludeHeaders: true\n";
    json_t *output1 = log_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL,
                                              config1, NULL, &content_type, NULL);
    
    TEST_ASSERT_NOT_NULL(output1);
    
    json_t *output1_metadata = json_object_get(output1, "_metadata");
    TEST_ASSERT_NOT_NULL(output1_metadata);
    json_t *metadata1 = json_object_get(output1_metadata, "log");
    TEST_ASSERT_NOT_NULL(metadata1);
    
    json_t *include_body1 = json_object_get(metadata1, "include_body");
    json_t *include_headers1 = json_object_get(metadata1, "include_headers");
    
    TEST_ASSERT_TRUE(json_boolean_value(include_body1));
    TEST_ASSERT_TRUE(json_boolean_value(include_headers1));
    
    // Test with body and headers disabled
    const char *config2 = "level: info\nenabled: true\nincludeBody: false\nincludeHeaders: false\n";
    json_t *output2 = log_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL,
                                              config2, NULL, &content_type, NULL);
    
    TEST_ASSERT_NOT_NULL(output2);
    
    json_t *output2_metadata = json_object_get(output2, "_metadata");
    TEST_ASSERT_NOT_NULL(output2_metadata);
    json_t *metadata2 = json_object_get(output2_metadata, "log");
    TEST_ASSERT_NOT_NULL(metadata2);
    
    json_t *include_body2 = json_object_get(metadata2, "include_body");
    json_t *include_headers2 = json_object_get(metadata2, "include_headers");
    
    TEST_ASSERT_FALSE(json_boolean_value(include_body2));
    TEST_ASSERT_FALSE(json_boolean_value(include_headers2));
    
    if (output1 != input) {
        json_decref(output1);
    }
    if (output2 != input) {
        json_decref(output2);
    }
    json_decref(input);
    destroy_test_arena(arena);
}

int main(void) {
    UNITY_BEGIN();
    
    RUN_TEST(test_log_middleware_basic_passthrough);
    RUN_TEST(test_log_middleware_disabled);
    RUN_TEST(test_log_middleware_configuration_parsing);
    RUN_TEST(test_log_middleware_post_execute_logging);
    RUN_TEST(test_log_middleware_null_input_handling);
    RUN_TEST(test_log_middleware_null_config_handling);
    RUN_TEST(test_log_middleware_empty_config_handling);
    RUN_TEST(test_log_middleware_memory_arena_usage);
    RUN_TEST(test_log_middleware_full_pipeline_simulation);
    RUN_TEST(test_log_middleware_different_log_levels);
    RUN_TEST(test_log_middleware_body_and_header_flags);
    
    return UNITY_END();
}
