#include "../unity/unity.h"
#include "../helpers/test_utils.h"
#include "../../src/wp.h"
#include <string.h>

void setUp(void) {
    // Set up function called before each test
}

void tearDown(void) {
    // Tear down function called after each test
}

void test_pipeline_single_step(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *input = create_test_request("GET", "/test");
    
    // Create a simple pipeline with one step
    PipelineStep *step = arena_alloc(arena, sizeof(PipelineStep));
    step->plugin = arena_strdup(arena, "jq");
    step->value = arena_strdup(arena, "{ message: \"hello\" }");
    step->is_variable = false;
    step->next = NULL;
    
    json_t *final_response;
    int response_code;
    
    int result = execute_pipeline_with_result(step, input, arena, &final_response, &response_code);
    
    TEST_ASSERT_EQUAL(0, result);
    TEST_ASSERT_NOT_NULL(final_response);
    TEST_ASSERT_EQUAL(200, response_code);
    
    json_decref(input);
    json_decref(final_response);
    destroy_test_arena(arena);
}

void test_pipeline_multi_step(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *input = create_test_request("GET", "/test");
    
    // Create multi-step pipeline
    PipelineStep *step1 = arena_alloc(arena, sizeof(PipelineStep));
    step1->plugin = arena_strdup(arena, "jq");
    step1->value = arena_strdup(arena, "{ id: \"123\" }");
    step1->is_variable = false;
    
    PipelineStep *step2 = arena_alloc(arena, sizeof(PipelineStep));
    step2->plugin = arena_strdup(arena, "lua");
    step2->value = arena_strdup(arena, "return { sqlParams = { request.id } }");
    step2->is_variable = false;
    step2->next = NULL;
    
    step1->next = step2;
    
    json_t *final_response;
    int response_code;
    
    int result = execute_pipeline_with_result(step1, input, arena, &final_response, &response_code);
    
    TEST_ASSERT_EQUAL(0, result);
    TEST_ASSERT_NOT_NULL(final_response);
    
    json_decref(input);
    json_decref(final_response);
    destroy_test_arena(arena);
}

int main(void) {
    UNITY_BEGIN();
    
    RUN_TEST(test_pipeline_single_step);
    RUN_TEST(test_pipeline_multi_step);
    
    return UNITY_END();
}