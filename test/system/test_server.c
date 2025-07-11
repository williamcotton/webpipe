#include "../unity/unity.h"
#include "../helpers/test_utils.h"
#include "../../src/wp.h"
#include <string.h>
#include <unistd.h>
#include <sys/wait.h>
#include <dlfcn.h>

void setUp(void) {
    // Set up function called before each test
}

void tearDown(void) {
    // Tear down function called after each test
}

static void test_server_startup_shutdown(void) {
    // Test server initialization using safe test runtime
    int result = init_test_runtime("test.wp");
    TEST_ASSERT_EQUAL(0, result);
    
    // Test server cleanup
    cleanup_test_runtime();
    
    // Should not crash
    TEST_ASSERT_TRUE(1);
}

static void test_server_route_matching(void) {
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

static void test_server_request_json_creation(void) {
    MemoryArena *arena = arena_create(1024 * 1024);  // Increased arena size
    set_current_arena(arena);
    
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
    
    // Clean up JSON object
    json_decref(request);
    
    set_current_arena(NULL);
    arena_free(arena);
}

static void test_server_middleware_loading(void) {
    // Test middleware loading by directly using dlopen/dlsym like the actual middleware loading
    void *handle = dlopen("./middleware/jq.so", RTLD_LAZY);
    TEST_ASSERT_NOT_NULL(handle);
    
    // Test that we can find the middleware_execute function
    json_t *(*execute)(json_t *, void *, arena_alloc_func, arena_free_func, const char *) = 
        (json_t *(*)(json_t *, void *, arena_alloc_func, arena_free_func, const char *))dlsym(handle, "middleware_execute");
    TEST_ASSERT_NOT_NULL(execute);
    
    // Test that the function works
    json_t *input = json_object();
    json_object_set_new(input, "test", json_string("value"));
    
    json_t *output = execute(input, NULL, NULL, NULL, ".");
    TEST_ASSERT_NOT_NULL(output);
    
    json_t *test_value = json_object_get(output, "test");
    TEST_ASSERT_NOT_NULL(test_value);
    TEST_ASSERT_STRING_EQUAL("value", json_string_value(test_value));
    
    // Clean up
    json_decref(input);
    json_decref(output);
    dlclose(handle);
    
    // Test loading non-existent middleware
    void *bad_handle = dlopen("./middleware/nonexistent.so", RTLD_LAZY);
    TEST_ASSERT_NULL(bad_handle);
}

static void test_server_memory_arena_per_request(void) {
    MemoryArena *arena = arena_create(1024 * 1024);  // Increased arena size
    
    // Set arena context for this thread before creating JSON
    set_current_arena(arena);
    
    size_t initial_used = arena->used;
    
    // Simulate request processing
    json_t *request = create_request_json(NULL, "/test", "GET", NULL, 0);
    TEST_ASSERT_NOT_NULL(request);
    
    // Process request (would normally go through full pipeline)
    char *test_string = arena_strdup(arena, "test allocation");
    TEST_ASSERT_NOT_NULL(test_string);
    
    // Arena should have been used
    TEST_ASSERT_GREATER_THAN(initial_used, arena->used);
    
    // Clean up JSON object
    json_decref(request);
    
    // Clear arena context before cleanup
    set_current_arena(NULL);
    arena_free(arena);
}

int main(void) {
    UNITY_BEGIN();
    
    RUN_TEST(test_server_startup_shutdown);
    RUN_TEST(test_server_route_matching);
    RUN_TEST(test_server_request_json_creation);
    RUN_TEST(test_server_middleware_loading);
    RUN_TEST(test_server_memory_arena_per_request);
    
    return UNITY_END();
}
