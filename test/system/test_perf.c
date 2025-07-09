#include "../unity/unity.h"
#include "../helpers/test_utils.h"
#include "../../src/wp.h"
#include <string.h>
#include <time.h>

void setUp(void) {
    // Set up function called before each test
}

void tearDown(void) {
    // Tear down function called after each test
}

void test_perf_arena_allocation(void) {
    const int iterations = 10000;
    
    start_timer();
    
    for (int i = 0; i < iterations; i++) {
        MemoryArena *arena = arena_create(1024);
        
        // Allocate various sizes
        for (int j = 0; j < 100; j++) {
            void *ptr = arena_alloc(arena, 10);
            TEST_ASSERT_NOT_NULL(ptr);
        }
        
        arena_free(arena);
    }
    
    assert_execution_time_under(5.0);  // Should complete in under 5 seconds
}

void test_perf_lexer_tokenization(void) {
    const char *source = "GET /test\n  |> jq: `{ message: \"hello\" }`\n\nPOST /users\n  |> jq: `{ user: .body }`";
    const int iterations = 1000;
    
    start_timer();
    
    for (int i = 0; i < iterations; i++) {
        int token_count;
        Token *tokens = tokenize_test_string(source, &token_count);
        TEST_ASSERT_NOT_NULL(tokens);
        free_test_tokens(tokens, token_count);
    }
    
    assert_execution_time_under(2.0);  // Should complete in under 2 seconds
}

void test_perf_parser_parsing(void) {
    const char *source = "GET /test\n  |> jq: `{ message: \"hello\" }`\n\nPOST /users\n  |> jq: `{ user: .body }`";
    const int iterations = 1000;
    
    start_timer();
    
    for (int i = 0; i < iterations; i++) {
        ASTNode *ast = parse_test_string(source);
        TEST_ASSERT_NOT_NULL(ast);
        free_test_ast(ast);
    }
    
    assert_execution_time_under(3.0);  // Should complete in under 3 seconds
}

void test_perf_json_operations(void) {
    const int iterations = 10000;
    
    start_timer();
    
    for (int i = 0; i < iterations; i++) {
        json_t *obj = json_object();
        json_object_set_new(obj, "message", json_string("hello"));
        json_object_set_new(obj, "count", json_integer(i));
        json_object_set_new(obj, "active", json_true());
        
        json_t *array = json_array();
        for (int j = 0; j < 10; j++) {
            json_array_append_new(array, json_integer(j));
        }
        json_object_set_new(obj, "numbers", array);
        
        char *json_str = json_dumps(obj, JSON_COMPACT);
        TEST_ASSERT_NOT_NULL(json_str);
        
        free(json_str);
        json_decref(obj);
    }
    
    assert_execution_time_under(2.0);  // Should complete in under 2 seconds
}

void test_perf_plugin_execution(void) {
    const int iterations = 1000;
    MemoryArena *arena = create_test_arena(1024 * 1024);  // 1MB arena
    
    Plugin *plugin = create_mock_plugin("test", mock_plugin_passthrough);
    
    start_timer();
    
    for (int i = 0; i < iterations; i++) {
        json_t *input = create_test_request("GET", "/test");
        json_t *output = plugin->execute(input, arena, get_arena_alloc_wrapper(), NULL, "test config");
        
        TEST_ASSERT_NOT_NULL(output);
        
        json_decref(input);
        json_decref(output);
    }
    
    assert_execution_time_under(1.0);  // Should complete in under 1 second
    
    destroy_mock_plugin(plugin);
    destroy_test_arena(arena);
}

void test_perf_memory_usage(void) {
    const int iterations = 1000;
    
    start_timer();
    
    for (int i = 0; i < iterations; i++) {
        MemoryArena *arena = create_test_arena(1024);
        
        // Simulate request processing
        json_t *request = create_test_request("GET", "/test");
        char *test_string = arena_strdup(arena, "test allocation");
        
        TEST_ASSERT_NOT_NULL(test_string);
        
        json_decref(request);
        destroy_test_arena(arena);
    }
    
    assert_execution_time_under(2.0);  // Should complete in under 2 seconds
}

void test_perf_concurrent_arena_access(void) {
    const int iterations = 1000;
    MemoryArena *arena = create_test_arena(1024 * 1024);  // 1MB arena
    
    start_timer();
    
    // Simulate concurrent access patterns
    for (int i = 0; i < iterations; i++) {
        void *ptr1 = arena_alloc(arena, 100);
        void *ptr2 = arena_alloc(arena, 200);
        void *ptr3 = arena_alloc(arena, 50);
        
        TEST_ASSERT_NOT_NULL(ptr1);
        TEST_ASSERT_NOT_NULL(ptr2);
        TEST_ASSERT_NOT_NULL(ptr3);
        
        // Verify no overlap
        TEST_ASSERT_NOT_EQUAL(ptr1, ptr2);
        TEST_ASSERT_NOT_EQUAL(ptr2, ptr3);
        TEST_ASSERT_NOT_EQUAL(ptr1, ptr3);
    }
    
    assert_execution_time_under(0.5);  // Should complete in under 0.5 seconds
    
    destroy_test_arena(arena);
}

void test_perf_string_operations(void) {
    const int iterations = 10000;
    MemoryArena *arena = create_test_arena(1024 * 1024);  // 1MB arena
    
    start_timer();
    
    for (int i = 0; i < iterations; i++) {
        char *str1 = arena_strdup(arena, "Hello, World!");
        char *str2 = arena_strndup(arena, "Test string", 4);
        
        TEST_ASSERT_NOT_NULL(str1);
        TEST_ASSERT_NOT_NULL(str2);
        TEST_ASSERT_STRING_EQUAL("Hello, World!", str1);
        TEST_ASSERT_STRING_EQUAL("Test", str2);
    }
    
    assert_execution_time_under(1.0);  // Should complete in under 1 second
    
    destroy_test_arena(arena);
}

void test_perf_pipeline_execution(void) {
    const int iterations = 100;
    
    start_timer();
    
    for (int i = 0; i < iterations; i++) {
        MemoryArena *arena = create_test_arena(1024);
        json_t *input = create_test_request("GET", "/test");
        
        // Create a simple pipeline
        PipelineStep *step = arena_alloc(arena, sizeof(PipelineStep));
        step->plugin = arena_strdup(arena, "jq");
        step->value = arena_strdup(arena, "{ message: \"performance test\" }");
        step->is_variable = false;
        step->next = NULL;
        
        json_t *final_response;
        int response_code;
        
        int result = execute_pipeline_with_result(step, input, arena, &final_response, &response_code);
        
        TEST_ASSERT_EQUAL(0, result);
        TEST_ASSERT_NOT_NULL(final_response);
        
        json_decref(input);
        json_decref(final_response);
        destroy_test_arena(arena);
    }
    
    assert_execution_time_under(2.0);  // Should complete in under 2 seconds
}

int main(void) {
    UNITY_BEGIN();
    
    RUN_TEST(test_perf_arena_allocation);
    RUN_TEST(test_perf_lexer_tokenization);
    RUN_TEST(test_perf_parser_parsing);
    RUN_TEST(test_perf_json_operations);
    RUN_TEST(test_perf_plugin_execution);
    RUN_TEST(test_perf_memory_usage);
    RUN_TEST(test_perf_concurrent_arena_access);
    RUN_TEST(test_perf_string_operations);
    RUN_TEST(test_perf_pipeline_execution);
    
    return UNITY_END();
}