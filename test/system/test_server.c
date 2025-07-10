#include "../unity/unity.h"
#include "../helpers/test_utils.h"
#include "../../src/wp.h"
#include <string.h>
#include <unistd.h>
#include <sys/wait.h>

void setUp(void) {
    // Set up function called before each test
}

void tearDown(void) {
    // Tear down function called after each test
}

void test_server_startup_shutdown(void) {
    // Test server initialization using safe test runtime
    int result = init_test_runtime("test.wp");
    TEST_ASSERT_EQUAL(0, result);
    
    // Test server cleanup
    cleanup_test_runtime();
    
    // Should not crash
    TEST_ASSERT_TRUE(1);
}

void test_server_route_matching(void) {
    json_t *params = json_object();
    
    // Test exact match
    bool match = match_route("/test", "/test", params);
    TEST_ASSERT_TRUE(match);
    
    // Test parameter match
    match = match_route("/page/:id", "/page/123", params);
    TEST_ASSERT_TRUE(match);
    
    json_t *id = json_object_get(params, "id");
    TEST_ASSERT_NOT_NULL(id);
    TEST_ASSERT_TRUE(json_is_integer(id));
    TEST_ASSERT_EQUAL(123, json_integer_value(id));
    
    // Test no match
    match = match_route("/test", "/different", params);
    TEST_ASSERT_FALSE(match);
    
    json_decref(params);
}

void test_server_request_json_creation(void) {
    json_t *request = create_request_json(NULL, "/test", "GET", NULL, 0);
    
    TEST_ASSERT_NOT_NULL(request);
    
    json_t *method = json_object_get(request, "method");
    json_t *url = json_object_get(request, "url");
    json_t *params = json_object_get(request, "params");
    json_t *query = json_object_get(request, "query");
    json_t *headers = json_object_get(request, "headers");
    json_t *body = json_object_get(request, "body");
    
    TEST_ASSERT_NOT_NULL(method);
    TEST_ASSERT_NOT_NULL(url);
    TEST_ASSERT_NOT_NULL(params);
    TEST_ASSERT_NOT_NULL(query);
    TEST_ASSERT_NOT_NULL(headers);
    TEST_ASSERT_NOT_NULL(body);
    
    TEST_ASSERT_STRING_EQUAL("GET", json_string_value(method));
    TEST_ASSERT_STRING_EQUAL("/test", json_string_value(url));
    
    json_decref(request);
}

void test_server_plugin_loading(void) {
    // Initialize the actual runtime for plugin loading
    int result = wp_runtime_init("test.wp");
    TEST_ASSERT_EQUAL(0, result);
    
    // Test loading existing plugins
    result = load_plugin("jq");
    TEST_ASSERT_EQUAL(0, result);
    
    Plugin *plugin = find_plugin("jq");
    TEST_ASSERT_NOT_NULL(plugin);
    TEST_ASSERT_STRING_EQUAL("jq", plugin->name);
    
    // Test loading non-existent plugin
    result = load_plugin("nonexistent");
    TEST_ASSERT_NOT_EQUAL(0, result);
    
    plugin = find_plugin("nonexistent");
    TEST_ASSERT_NULL(plugin);
    
    // Clean up
    wp_runtime_cleanup();
}

void test_server_memory_arena_per_request(void) {
    MemoryArena *arena = arena_create(1024);
    
    // Simulate request processing
    json_t *request = create_request_json(NULL, "/test", "GET", NULL, 0);
    
    size_t initial_used = arena->used;
    
    // Process request (would normally go through full pipeline)
    char *test_string = arena_strdup(arena, "test allocation");
    TEST_ASSERT_NOT_NULL(test_string);
    
    // Arena should have been used
    TEST_ASSERT_GREATER_THAN(initial_used, arena->used);
    
    json_decref(request);
    arena_free(arena);
}

int main(void) {
    UNITY_BEGIN();
    
    RUN_TEST(test_server_startup_shutdown);
    RUN_TEST(test_server_route_matching);
    RUN_TEST(test_server_request_json_creation);
    RUN_TEST(test_server_plugin_loading);
    RUN_TEST(test_server_memory_arena_per_request);
    
    return UNITY_END();
}