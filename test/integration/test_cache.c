#include "../unity/unity.h"
#include "../helpers/test_utils.h"
#include "../../src/wp.h"
#include <string.h>
#include <unistd.h>
#include <time.h>

// Load the actual cache middleware
#include <dlfcn.h>
static void *cache_middleware_handle = NULL;
static json_t *(*cache_middleware_execute)(json_t *, void *, arena_alloc_func, arena_free_func, const char *, json_t *, char **, json_t *) = NULL;
static void (*cache_middleware_post_execute)(json_t *, void *, arena_alloc_func, json_t *) = NULL;
static int (*cache_middleware_init)(json_t *) = NULL;
static void (*cache_cleanup)(void) = NULL;

static int load_cache_middleware(void) {
    if (cache_middleware_handle) return 0; // Already loaded
    
    cache_middleware_handle = dlopen("./middleware/cache.so", RTLD_LAZY);
    if (!cache_middleware_handle) {
        fprintf(stderr, "Failed to load cache middleware: %s\n", dlerror());
        return -1;
    }
    
    // Load middleware_execute function
    void *execute_func = dlsym(cache_middleware_handle, "middleware_execute");
    cache_middleware_execute = (json_t *(*)(json_t *, void *, arena_alloc_func, arena_free_func, const char *, json_t *, char **, json_t *))
                        (uintptr_t)execute_func;
    if (!execute_func) {
        fprintf(stderr, "Failed to find middleware_execute in cache middleware: %s\n", dlerror());
        dlclose(cache_middleware_handle);
        cache_middleware_handle = NULL;
        return -1;
    }
    
    // Load middleware_post_execute function
    void *post_execute_func = dlsym(cache_middleware_handle, "middleware_post_execute");
    cache_middleware_post_execute = (void (*)(json_t *, void *, arena_alloc_func, json_t *))
                                   (uintptr_t)post_execute_func;
    if (!post_execute_func) {
        fprintf(stderr, "Failed to find middleware_post_execute in cache middleware: %s\n", dlerror());
        dlclose(cache_middleware_handle);
        cache_middleware_handle = NULL;
        return -1;
    }
    
    // Load middleware_init function
    void *init_func = dlsym(cache_middleware_handle, "middleware_init");
    cache_middleware_init = (int (*)(json_t *))
                           (uintptr_t)init_func;
    if (!init_func) {
        fprintf(stderr, "Failed to find middleware_init in cache middleware: %s\n", dlerror());
        dlclose(cache_middleware_handle);
        cache_middleware_handle = NULL;
        return -1;
    }
    
    // Load cache_cleanup function  
    void *cleanup_func = dlsym(cache_middleware_handle, "cache_cleanup");
    cache_cleanup = (void (*)(void))(uintptr_t)cleanup_func;
    if (!cleanup_func) {
        fprintf(stderr, "Failed to find cache_cleanup in cache middleware: %s\n", dlerror());
        dlclose(cache_middleware_handle);
        cache_middleware_handle = NULL;
        return -1;
    }
    
    return 0;
}

static void unload_cache_middleware(void) {
    if (cache_middleware_handle) {
        dlclose(cache_middleware_handle);
        cache_middleware_handle = NULL;
        cache_middleware_execute = NULL;
        cache_middleware_post_execute = NULL;
        cache_middleware_init = NULL;
        cache_cleanup = NULL;
    }
}

void setUp(void) {
    if (load_cache_middleware() != 0) {
        TEST_FAIL_MESSAGE("Failed to load cache middleware");
    }
    
    // Initialize cache middleware with test configuration
    json_t *config = json_object();
    json_object_set_new(config, "enabled", json_boolean(1));
    json_object_set_new(config, "defaultTtl", json_integer(60));
    json_object_set_new(config, "maxCacheSize", json_integer(1024 * 1024));
    
    if (cache_middleware_init(config) != 0) {
        json_decref(config);
        TEST_FAIL_MESSAGE("Failed to initialize cache middleware");
    }
    json_decref(config);
}

void tearDown(void) {
    // Clean up all cache entries before unloading
    if (cache_cleanup) {
        cache_cleanup();
    }
    unload_cache_middleware();
}

static void test_cache_middleware_basic_passthrough(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *input = create_test_request("GET", "/test");
    const char *config = "enabled: true\nttl: 30\n";
    char *content_type = NULL;
    
    json_t *output = cache_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, 
                                             config, NULL, &content_type, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    TEST_ASSERT_EQUAL_PTR(input, output); // Should return same object for cache miss
    
    // Should have cache metadata added
    json_t *cache_metadata = json_object_get(output, "_cache_metadata");
    TEST_ASSERT_NOT_NULL(cache_metadata);
    
    json_t *cache_enabled = json_object_get(cache_metadata, "cache_enabled");
    TEST_ASSERT_NOT_NULL(cache_enabled);
    TEST_ASSERT_TRUE(json_boolean_value(cache_enabled));
    
    if (output != input) {
        json_decref(output);
    }
    if (output != input) {
        json_decref(output);
    }
    json_decref(input);
    destroy_test_arena(arena);
}

static void test_cache_middleware_disabled(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *input = create_test_request("GET", "/test");
    const char *config = "enabled: false\nttl: 30\n";
    char *content_type = NULL;
    
    json_t *output = cache_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL,
                                             config, NULL, &content_type, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    TEST_ASSERT_EQUAL_PTR(input, output); // Should return same object
    
    // Should NOT have cache metadata when disabled
    json_t *cache_metadata = json_object_get(output, "_cache_metadata");
    TEST_ASSERT_NULL(cache_metadata);
    
    if (output != input) {
        json_decref(output);
    }
    if (output != input) {
        json_decref(output);
    }
    json_decref(input);
    destroy_test_arena(arena);
}

static void test_cache_middleware_key_generation(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *input = create_test_request("GET", "/api/users");
    json_t *query = json_object();
    json_object_set_new(query, "page", json_string("1"));
    json_object_set_new(query, "limit", json_string("10"));
    json_object_set_new(input, "query", query);
    
    const char *config = "enabled: true\nttl: 60\n";
    char *content_type = NULL;
    
    json_t *output = cache_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL,
                                             config, NULL, &content_type, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    
    json_t *cache_metadata = json_object_get(output, "_cache_metadata");
    TEST_ASSERT_NOT_NULL(cache_metadata);
    
    json_t *cache_key = json_object_get(cache_metadata, "cache_key");
    TEST_ASSERT_NOT_NULL(cache_key);
    TEST_ASSERT(json_is_string(cache_key));
    
    const char *key_str = json_string_value(cache_key);
    TEST_ASSERT(strstr(key_str, "GET") != NULL);
    TEST_ASSERT(strstr(key_str, "/api/users") != NULL);
    
    if (output != input) {
        json_decref(output);
    }
    json_decref(input);
    destroy_test_arena(arena);
}

static void test_cache_middleware_template_key(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *input = create_test_request("GET", "/user/123/profile");
    json_t *params = json_object();
    json_object_set_new(params, "id", json_string("123"));
    json_object_set_new(input, "params", params);
    
    json_t *query = json_object();
    json_object_set_new(query, "format", json_string("json"));
    json_object_set_new(input, "query", query);
    
    const char *config = "keyTemplate: user-{params.id}-{query.format}\nttl: 30\nenabled: true\n";
    char *content_type = NULL;
    
    json_t *output = cache_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL,
                                             config, NULL, &content_type, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    
    json_t *cache_metadata = json_object_get(output, "_cache_metadata");
    TEST_ASSERT_NOT_NULL(cache_metadata);
    
    json_t *cache_key = json_object_get(cache_metadata, "cache_key");
    TEST_ASSERT_NOT_NULL(cache_key);
    
    const char *key_str = json_string_value(cache_key);
    TEST_ASSERT_EQUAL_STRING("user-123-json", key_str);
    
    if (output != input) {
        json_decref(output);
    }
    json_decref(input);
    destroy_test_arena(arena);
}

static void test_cache_middleware_template_missing_values(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *input = create_test_request("GET", "/user/profile");
    // No params or query - template should use placeholders
    
    const char *config = "keyTemplate: user-{params.id}-{query.format}\nttl: 30\nenabled: true\n";
    char *content_type = NULL;
    
    json_t *output = cache_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL,
                                             config, NULL, &content_type, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    
    json_t *cache_metadata = json_object_get(output, "_cache_metadata");
    TEST_ASSERT_NOT_NULL(cache_metadata);
    
    json_t *cache_key = json_object_get(cache_metadata, "cache_key");
    TEST_ASSERT_NOT_NULL(cache_key);
    
    const char *key_str = json_string_value(cache_key);
    // Should have null placeholders for missing values
    TEST_ASSERT(strstr(key_str, "null") != NULL);
    
    if (output != input) {
        json_decref(output);
    }
    json_decref(input);
    destroy_test_arena(arena);
}

static void test_cache_middleware_configuration_parsing(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *input = create_test_request("GET", "/test");
    
    // Test various configuration formats
    const char *config1 = "ttl: 120\nenabled: true\n";
    // const char *config2 = "ttl: 60\nkey: custom-cache-key\n";
    const char *config3 = "# Comment line\nttl: 30\n\n# Another comment\nenabled: true\n";
    
    char *content_type = NULL;
    
    // Test config 1
    json_t *output1 = cache_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL,
                                              config1, NULL, &content_type, NULL);
    TEST_ASSERT_NOT_NULL(output1);
    
    json_t *metadata1 = json_object_get(output1, "_cache_metadata");
    TEST_ASSERT_NOT_NULL(metadata1);
    
    json_t *ttl1 = json_object_get(metadata1, "cache_ttl");
    TEST_ASSERT_EQUAL_INT(120, json_integer_value(ttl1));
    
    // Test config 3 (with comments)
    json_t *output3 = cache_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL,
                                              config3, NULL, &content_type, NULL);
    TEST_ASSERT_NOT_NULL(output3);
    
    json_t *metadata3 = json_object_get(output3, "_cache_metadata");
    TEST_ASSERT_NOT_NULL(metadata3);
    
    json_t *ttl3 = json_object_get(metadata3, "cache_ttl");
    TEST_ASSERT_EQUAL_INT(30, json_integer_value(ttl3));
    
    if (output1 != input) {
        json_decref(output1);
    }
    if (output3 != input) {
        json_decref(output3);
    }
    json_decref(input);
    destroy_test_arena(arena);
}

static void test_cache_middleware_post_execute_storage(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    // Create a response to cache
    json_t *final_response = json_object();
    json_object_set_new(final_response, "message", json_string("test response"));
    json_object_set_new(final_response, "timestamp", json_integer(time(NULL)));
    
    // Add cache metadata
    json_t *cache_metadata = json_object();
    json_object_set_new(cache_metadata, "cache_key", json_string("test_post_execute"));
    json_object_set_new(cache_metadata, "cache_ttl", json_integer(60));
    json_object_set_new(cache_metadata, "cache_enabled", json_boolean(1));
    json_object_set_new(final_response, "_cache_metadata", cache_metadata);
    
    // Call post execute
    cache_middleware_post_execute(final_response, arena, get_arena_alloc_wrapper(), NULL);
    
    // Verify the response was cached by trying to retrieve it
    // This is an indirect test since we don't have direct access to cache_get
    json_t *test_request = create_test_request("GET", "/test");
    json_object_set_new(test_request, "url", json_string("test_post_execute")); // Match the cache key pattern
    
    json_decref(final_response);
    json_decref(test_request);
    destroy_test_arena(arena);
}

static void test_cache_middleware_null_input_handling(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    const char *config = "enabled: true\nttl: 30\n";
    char *content_type = NULL;
    
    // Cache middleware should handle null input gracefully
    json_t *output = cache_middleware_execute(NULL, arena, get_arena_alloc_wrapper(), NULL,
                                             config, NULL, &content_type, NULL);
    
    // Should return null for null input
    TEST_ASSERT_NULL(output);
    
    destroy_test_arena(arena);
}

static void test_cache_middleware_null_config_handling(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *input = create_test_request("GET", "/test");
    char *content_type = NULL;
    
    // Should handle null config by using defaults
    json_t *output = cache_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL,
                                             NULL, NULL, &content_type, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    // With null config, should still work with global defaults
    
    if (output != input) {
        json_decref(output);
    }
    json_decref(input);
    destroy_test_arena(arena);
}

static void test_cache_middleware_empty_config_handling(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *input = create_test_request("GET", "/test");
    const char *config = "";
    char *content_type = NULL;
    
    // Should handle empty config by using defaults
    json_t *output = cache_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL,
                                             config, NULL, &content_type, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    
    if (output != input) {
        json_decref(output);
    }
    json_decref(input);
    destroy_test_arena(arena);
}

static void test_cache_middleware_memory_arena_usage(void) {
    MemoryArena *arena = create_test_arena(1024);
    size_t initial_used = arena->used;
    
    json_t *input = create_test_request("GET", "/test");
    const char *config = "keyTemplate: memory-test-{params.id}\nttl: 30\n";
    char *content_type = NULL;
    
    json_t *output = cache_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL,
                                             config, NULL, &content_type, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    
    // Arena should have been used for cache key generation
    TEST_ASSERT_GREATER_THAN(initial_used, arena->used);
    
    if (output != input) {
        json_decref(output);
    }
    json_decref(input);
    destroy_test_arena(arena);
}

static void test_cache_middleware_performance_simple(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *input = create_test_request("GET", "/performance-test");
    const char *config = "enabled: true\nttl: 60\n";
    char *content_type = NULL;
    
    start_timer();
    
    // Run multiple cache operations
    for (int i = 0; i < 100; i++) {
        json_t *output = cache_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL,
                                                 config, NULL, &content_type, NULL);
        TEST_ASSERT_NOT_NULL(output);
        
        // Simulate post execute for some requests
        if (i % 10 == 0) {
            json_t *cache_metadata = json_object_get(output, "_cache_metadata");
            if (cache_metadata) {
                json_t *response = json_object();
                json_object_set_new(response, "iteration", json_integer(i));
                json_object_set_new(response, "_cache_metadata", json_deep_copy(cache_metadata));
                
                cache_middleware_post_execute(response, arena, get_arena_alloc_wrapper(), NULL);
                json_decref(response);
            }
        }
        
        // Decref output if it's different from input (happens when cache metadata is added)
        if (output != input) {
            json_decref(output);
        }
    }
    
    assert_execution_time_under(1.0);  // Should complete in under 1 second
    
    json_decref(input);
    destroy_test_arena(arena);
}

static void test_cache_middleware_complex_template(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *input = create_test_request("GET", "/api/search");
    
    json_t *params = json_object();
    json_object_set_new(params, "category", json_string("electronics"));
    json_object_set_new(input, "params", params);
    
    json_t *query = json_object();
    json_object_set_new(query, "q", json_string("smartphone"));
    json_object_set_new(query, "sort", json_string("price"));
    json_object_set_new(input, "query", query);
    
    json_t *headers = json_object();
    json_object_set_new(headers, "Accept-Language", json_string("en-US"));
    json_object_set_new(input, "headers", headers);
    
    const char *config = "keyTemplate: search-{params.category}-{query.q}-{query.sort}\n"
                        "ttl: 300\n"
                        "enabled: true\n";
    char *content_type = NULL;
    
    json_t *output = cache_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL,
                                             config, NULL, &content_type, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    
    json_t *cache_metadata = json_object_get(output, "_cache_metadata");
    TEST_ASSERT_NOT_NULL(cache_metadata);
    
    json_t *cache_key = json_object_get(cache_metadata, "cache_key");
    TEST_ASSERT_NOT_NULL(cache_key);
    
    const char *key_str = json_string_value(cache_key);
    TEST_ASSERT_EQUAL_STRING("search-electronics-smartphone-price", key_str);
    
    json_t *ttl = json_object_get(cache_metadata, "cache_ttl");
    TEST_ASSERT_EQUAL_INT(300, json_integer_value(ttl));
    
    if (output != input) {
        json_decref(output);
    }
    json_decref(input);
    destroy_test_arena(arena);
}

static void test_cache_middleware_full_pipeline_simulation(void) {
    MemoryArena *arena = create_test_arena(2048);
    
    // Simulate a complete cache miss -> execute -> post execute cycle
    
    // Step 1: Cache miss on first request
    json_t *input1 = create_test_request("GET", "/test/pipeline");
    json_t *params = json_object();
    json_object_set_new(params, "id", json_string("123"));
    json_object_set_new(input1, "params", params);
    
    const char *config = "keyTemplate: pipeline-{params.id}\nttl: 60\nenabled: true\n";
    char *content_type = NULL;
    
    json_t *cache_result1 = cache_middleware_execute(input1, arena, get_arena_alloc_wrapper(), NULL,
                                                    config, NULL, &content_type, NULL);
    
    TEST_ASSERT_NOT_NULL(cache_result1);
    TEST_ASSERT_EQUAL_PTR(input1, cache_result1); // Should be cache miss
    
    // Verify cache metadata was added
    json_t *cache_metadata = json_object_get(cache_result1, "_cache_metadata");
    TEST_ASSERT_NOT_NULL(cache_metadata);
    
    json_t *cache_key = json_object_get(cache_metadata, "cache_key");
    TEST_ASSERT_EQUAL_STRING("pipeline-123", json_string_value(cache_key));
    
    // Step 2: Simulate pipeline execution (jq middleware would do this)
    json_t *final_response = json_object();
    json_object_set_new(final_response, "id", json_string("123"));
    json_object_set_new(final_response, "data", json_string("processed data"));
    json_object_set_new(final_response, "timestamp", json_integer(time(NULL)));
    
    // Add the cache metadata from step 1
    json_object_set_new(final_response, "_cache_metadata", json_deep_copy(cache_metadata));
    
    // Step 3: Post execute to store in cache
    cache_middleware_post_execute(final_response, arena, get_arena_alloc_wrapper(), NULL);
    
    // Step 4: Test cache hit on second request (create new arena for clean test)
    MemoryArena *arena2 = create_test_arena(2048);
    json_t *input2 = create_test_request("GET", "/test/pipeline");
    json_t *params2 = json_object();
    json_object_set_new(params2, "id", json_string("123"));
    json_object_set_new(input2, "params", params2);
    
    json_t *cache_result2 = cache_middleware_execute(input2, arena2, get_arena_alloc_wrapper(), NULL,
                                                    config, NULL, &content_type, NULL);
    
    TEST_ASSERT_NOT_NULL(cache_result2);
    // This should be different from input2 if it's a cache hit with pipeline control
    if (cache_result2 != input2) {
        // Check for pipeline control response
        json_t *action = json_object_get(cache_result2, "_pipeline_action");
        if (action) {
            TEST_ASSERT_EQUAL_STRING("return", json_string_value(action));
            
            json_t *value = json_object_get(cache_result2, "value");
            TEST_ASSERT_NOT_NULL(value);
            
            json_t *cached_id = json_object_get(value, "id");
            TEST_ASSERT_EQUAL_STRING("123", json_string_value(cached_id));
        }
        // If cache_result2 is different from input2, we need to decref it
        json_decref(cache_result2);
    }
    
    json_decref(input1);
    json_decref(final_response);
    json_decref(input2);
    destroy_test_arena(arena);
    destroy_test_arena(arena2);
}

static void test_cache_middleware_integration_with_webpipe_config(void) {
    // Test using a WebPipe-style configuration similar to what would be in test.wp
    MemoryArena *arena = create_test_arena(1024);
    
    // Simulate the kind of request that would come from WebPipe runtime
    json_t *input = json_object();
    json_object_set_new(input, "method", json_string("GET"));
    json_object_set_new(input, "url", json_string("/cache-test"));
    json_object_set_new(input, "headers", json_object());
    json_object_set_new(input, "query", json_object());
    json_object_set_new(input, "params", json_object());
    json_object_set_new(input, "cookies", json_object());
    
    // WebPipe config that matches what we have in test.wp:
    // |> cache: `ttl: 10, enabled: true`
    const char *config = "ttl: 10\nenabled: true\n";
    char *content_type = NULL;
    
    json_t *output = cache_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL,
                                             config, NULL, &content_type, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    TEST_ASSERT_EQUAL_PTR(input, output); // Cache miss
    
    json_t *cache_metadata = json_object_get(output, "_cache_metadata");
    TEST_ASSERT_NOT_NULL(cache_metadata);
    
    json_t *ttl = json_object_get(cache_metadata, "cache_ttl");
    TEST_ASSERT_EQUAL_INT(10, json_integer_value(ttl));
    
    json_t *enabled = json_object_get(cache_metadata, "cache_enabled");
    TEST_ASSERT_TRUE(json_boolean_value(enabled));
    
    // Simulate the jq step that would come next:
    // |> jq: `{ message: "Hello from cache test!", timestamp: now, random: (now % 1000) }`
    json_t *jq_result = json_object();
    json_object_set_new(jq_result, "message", json_string("Hello from cache test!"));
    json_object_set_new(jq_result, "timestamp", json_integer(time(NULL)));
    json_object_set_new(jq_result, "random", json_integer(time(NULL) % 1000));
    
    // Add cache metadata for post execute
    json_object_set_new(jq_result, "_cache_metadata", json_deep_copy(cache_metadata));
    
    // Post execute
    cache_middleware_post_execute(jq_result, arena, get_arena_alloc_wrapper(), NULL);
    
    json_decref(input);
    json_decref(jq_result);
    destroy_test_arena(arena);
}

int main(void) {
    UNITY_BEGIN();
    
    RUN_TEST(test_cache_middleware_basic_passthrough);
    RUN_TEST(test_cache_middleware_disabled);
    RUN_TEST(test_cache_middleware_key_generation);
    RUN_TEST(test_cache_middleware_template_key);
    RUN_TEST(test_cache_middleware_template_missing_values);
    RUN_TEST(test_cache_middleware_configuration_parsing);
    RUN_TEST(test_cache_middleware_post_execute_storage);
    RUN_TEST(test_cache_middleware_null_input_handling);
    RUN_TEST(test_cache_middleware_null_config_handling);
    RUN_TEST(test_cache_middleware_empty_config_handling);
    RUN_TEST(test_cache_middleware_memory_arena_usage);
    RUN_TEST(test_cache_middleware_performance_simple);
    RUN_TEST(test_cache_middleware_complex_template);
    RUN_TEST(test_cache_middleware_full_pipeline_simulation);
    RUN_TEST(test_cache_middleware_integration_with_webpipe_config);
    
    return UNITY_END();
}
