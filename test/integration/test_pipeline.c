#include "../unity/unity.h"
#include "../helpers/test_utils.h"
#include "../../src/wp.h"
#include <string.h>
#include <dlfcn.h>
#include <stdlib.h>

void setUp(void) {
    // Set up function called before each test - for now skip runtime setup
    // The pipeline tests will need to be modified to test at a different level
}

void tearDown(void) {
    // Tear down function called after each test
}

// Load the actual jq middleware for testing
static void *jq_middleware_handle = NULL;
static json_t *(*jq_middleware_execute)(json_t *, void *, arena_alloc_func, arena_free_func, const char *) = NULL;

static int load_jq_middleware_for_test(void) {
    if (jq_middleware_handle) return 0; // Already loaded
    
    jq_middleware_handle = dlopen("./middleware/jq.so", RTLD_LAZY);
    if (!jq_middleware_handle) {
        return -1;
    }
    
    void *middleware_func = dlsym(jq_middleware_handle, "middleware_execute");
    jq_middleware_execute = (json_t *(*)(json_t *, void *, arena_alloc_func, arena_free_func, const char *))
                        (uintptr_t)middleware_func;
    if (!middleware_func) {
        dlclose(jq_middleware_handle);
        jq_middleware_handle = NULL;
        return -1;
    }
    
    return 0;
}

static void test_pipeline_single_step(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    // Load jq middleware for this test
    if (load_jq_middleware_for_test() != 0) {
        TEST_FAIL_MESSAGE("Failed to load jq middleware");
    }
    
    json_t *input = create_test_request("GET", "/test");
    
    // Test single step execution by calling middleware directly
    const char *config = "{ \"message\": \"hello\" }";
    json_t *output = jq_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, config);
    
    TEST_ASSERT_NOT_NULL(output);
    
    // Check that the output has the expected structure
    json_t *message = json_object_get(output, "message");
    TEST_ASSERT_NOT_NULL(message);
    TEST_ASSERT_STRING_EQUAL("hello", json_string_value(message));
    
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
}

static void test_pipeline_multi_step(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    // Load jq middleware for this test
    if (load_jq_middleware_for_test() != 0) {
        TEST_FAIL_MESSAGE("Failed to load jq middleware");
    }
    
    json_t *input = create_test_request("GET", "/test");
    
    // Test multi-step execution by calling middlewares sequentially
    // Step 1: Create an object with id
    const char *config1 = "{ \"id\": \"123\" }";
    json_t *step1_output = jq_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, config1);
    
    TEST_ASSERT_NOT_NULL(step1_output);
    
    // Step 2: Transform the output from step 1
    const char *config2 = "{ \"user_id\": .id }";
    json_t *step2_output = jq_middleware_execute(step1_output, arena, get_arena_alloc_wrapper(), NULL, config2);
    
    TEST_ASSERT_NOT_NULL(step2_output);
    
    // Check that we have the transformed data
    json_t *user_id = json_object_get(step2_output, "user_id");
    TEST_ASSERT_NOT_NULL(user_id);
    TEST_ASSERT_STRING_EQUAL("123", json_string_value(user_id));
    
    json_decref(input);
    json_decref(step1_output);
    json_decref(step2_output);
    destroy_test_arena(arena);
}

int main(void) {
    UNITY_BEGIN();
    
    RUN_TEST(test_pipeline_single_step);
    RUN_TEST(test_pipeline_multi_step);
    
    return UNITY_END();
}
