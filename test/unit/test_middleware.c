#include "../unity/unity.h"
#include "../helpers/test_utils.h"
#include "../../src/wp.h"
#include <string.h>
#include <dlfcn.h>

void setUp(void) {
    // Set up function called before each test
}

void tearDown(void) {
    // Tear down function called after each test
}

static void test_middleware_interface_basic(void) {
    Middleware *middleware = create_mock_middleware("test", mock_middleware_passthrough);
    
    TEST_ASSERT_NOT_NULL(middleware);
    TEST_ASSERT_STRING_EQUAL("test", middleware->name);
    TEST_ASSERT_NOT_NULL(middleware->execute);
    TEST_ASSERT_NULL(middleware->handle);
    
    destroy_mock_middleware(middleware);
}

static void test_middleware_execute_passthrough(void) {
    Middleware *middleware = create_mock_middleware("test", mock_middleware_passthrough);
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *input = json_object();
    json_object_set_new(input, "test", json_string("value"));
    
    char *content_type = NULL;
    json_t *output = middleware->execute(input, arena, get_arena_alloc_wrapper(), NULL, "test config", &content_type, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    TEST_ASSERT_JSON_EQUAL(input, output);
    
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
    destroy_mock_middleware(middleware);
}

static void test_middleware_execute_with_arena(void) {
    Middleware *middleware = create_mock_middleware("test", mock_middleware_passthrough);
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *input = json_object();
    json_object_set_new(input, "message", json_string("hello"));
    
    // Test that arena is passed correctly
    char *content_type = NULL;
    json_t *output = middleware->execute(input, arena, get_arena_alloc_wrapper(), NULL, NULL, &content_type, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
    destroy_mock_middleware(middleware);
}

static void test_middleware_execute_error_handling(void) {
    Middleware *middleware = create_mock_middleware("test", mock_middleware_error);
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *input = json_object();
    json_object_set_new(input, "test", json_string("value"));
    
    char *content_type = NULL;
    json_t *output = middleware->execute(input, arena, get_arena_alloc_wrapper(), NULL, "test config", &content_type, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    
    // Check that error structure is present
    json_t *errors = json_object_get(output, "errors");
    TEST_ASSERT_NOT_NULL(errors);
    TEST_ASSERT_TRUE(json_is_array(errors));
    TEST_ASSERT_EQUAL(1, json_array_size(errors));
    
    json_t *error = json_array_get(errors, 0);
    TEST_ASSERT_NOT_NULL(error);
    
    json_t *type = json_object_get(error, "type");
    TEST_ASSERT_NOT_NULL(type);
    TEST_ASSERT_STRING_EQUAL("mockError", json_string_value(type));
    
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
    destroy_mock_middleware(middleware);
}

static void test_middleware_execute_with_config(void) {
    Middleware *middleware = create_mock_middleware("test", mock_middleware_passthrough);
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *input = json_object();
    const char *config = "test configuration string";
    
    char *content_type = NULL;
    json_t *output = middleware->execute(input, arena, get_arena_alloc_wrapper(), NULL, config, &content_type, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
    destroy_mock_middleware(middleware);
}

static void test_middleware_execute_null_input(void) {
    Middleware *middleware = create_mock_middleware("test", mock_middleware_passthrough);
    MemoryArena *arena = create_test_arena(1024);
    
    char *content_type = NULL;
    json_t *output = middleware->execute(NULL, arena, get_arena_alloc_wrapper(), NULL, NULL, &content_type, NULL);
    
    // Should handle null input gracefully
    TEST_ASSERT_NULL(output);
    
    destroy_test_arena(arena);
    destroy_mock_middleware(middleware);
}

static void test_middleware_execute_null_arena(void) {
    Middleware *middleware = create_mock_middleware("test", mock_middleware_passthrough);
    
    json_t *input = json_object();
    json_object_set_new(input, "test", json_string("value"));
    
    char *content_type = NULL;
    json_t *output = middleware->execute(input, NULL, get_arena_alloc_wrapper(), NULL, NULL, &content_type, NULL);
    
    // Should handle null arena gracefully
    TEST_ASSERT_NOT_NULL(output);
    
    json_decref(input);
    json_decref(output);
    destroy_mock_middleware(middleware);
}

static void test_middleware_load_function(void) {
    // Test middleware loading functionality
    // This would test the actual dynamic library loading
    
    // For now, we'll test the interface
    int result = load_middleware("nonexistent");
    TEST_ASSERT_NOT_EQUAL(0, result);  // Should fail for non-existent middleware
}

static void test_middleware_find_function(void) {
    // Test middleware finding functionality
    
    Middleware *middleware = find_middleware("nonexistent");
    TEST_ASSERT_NULL(middleware);  // Should return NULL for non-existent middleware
}

static void test_middleware_arena_alloc_function(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    void *ptr = arena_alloc(arena, 100);
    TEST_ASSERT_NOT_NULL(ptr);
    TEST_ASSERT_EQUAL(100, arena->used);
    
    destroy_test_arena(arena);
}

static void test_middleware_memory_management(void) {
    Middleware *middleware = create_mock_middleware("test", mock_middleware_passthrough);
    MemoryArena *arena = create_test_arena(1024);
    
    // Test multiple allocations within middleware context
    for (int i = 0; i < 10; i++) {
        json_t *input = json_object();
        json_object_set_new(input, "iteration", json_integer(i));
        
        char *content_type = NULL;
        json_t *output = middleware->execute(input, arena, get_arena_alloc_wrapper(), NULL, NULL, &content_type, NULL);
        
        TEST_ASSERT_NOT_NULL(output);
        
        json_decref(input);
        json_decref(output);
    }
    
    destroy_test_arena(arena);
    destroy_mock_middleware(middleware);
}

static void test_middleware_concurrent_execution(void) {
    Middleware *middleware = create_mock_middleware("test", mock_middleware_passthrough);
    MemoryArena *arena1 = create_test_arena(1024);
    MemoryArena *arena2 = create_test_arena(1024);
    
    json_t *input1 = json_object();
    json_object_set_new(input1, "thread", json_string("1"));
    
    json_t *input2 = json_object();
    json_object_set_new(input2, "thread", json_string("2"));
    
    // Simulate concurrent execution
    char *content_type1 = NULL;
    char *content_type2 = NULL;
    json_t *output1 = middleware->execute(input1, arena1, get_arena_alloc_wrapper(), NULL, "config1", &content_type1, NULL);
    json_t *output2 = middleware->execute(input2, arena2, get_arena_alloc_wrapper(), NULL, "config2", &content_type2, NULL);
    
    TEST_ASSERT_NOT_NULL(output1);
    TEST_ASSERT_NOT_NULL(output2);
    
    json_decref(input1);
    json_decref(input2);
    json_decref(output1);
    json_decref(output2);
    destroy_test_arena(arena1);
    destroy_test_arena(arena2);
    destroy_mock_middleware(middleware);
}

static void test_middleware_json_preservation(void) {
    Middleware *middleware = create_mock_middleware("test", mock_middleware_passthrough);
    MemoryArena *arena = create_test_arena(1024);
    
    // Test various JSON types
    json_t *inputs[] = {
        json_object(),
        json_array(),
        json_string("test"),
        json_integer(42),
        json_real(3.14),
        json_true(),
        json_false(),
        json_null()
    };
    int num_inputs = sizeof(inputs) / sizeof(inputs[0]);
    
    for (int i = 0; i < num_inputs; i++) {
        char *content_type = NULL;
        json_t *output = middleware->execute(inputs[i], arena, get_arena_alloc_wrapper(), NULL, NULL, &content_type, NULL);
        
        TEST_ASSERT_NOT_NULL(output);
        TEST_ASSERT_JSON_EQUAL(inputs[i], output);
        
        json_decref(output);
    }
    
    // Clean up inputs
    for (int i = 0; i < num_inputs; i++) {
        json_decref(inputs[i]);
    }
    
    destroy_test_arena(arena);
    destroy_mock_middleware(middleware);
}

static void test_middleware_complex_json_handling(void) {
    Middleware *middleware = create_mock_middleware("test", mock_middleware_passthrough);
    MemoryArena *arena = create_test_arena(1024);
    
    // Create complex JSON structure
    json_t *input = json_object();
    json_t *request = json_object();
    json_t *params = json_object();
    json_t *headers = json_object();
    json_t *body = json_object();
    
    json_object_set_new(params, "id", json_string("123"));
    json_object_set_new(headers, "Content-Type", json_string("application/json"));
    json_object_set_new(body, "name", json_string("John Doe"));
    json_object_set_new(body, "email", json_string("john@example.com"));
    
    json_object_set_new(request, "method", json_string("POST"));
    json_object_set_new(request, "url", json_string("/users"));
    json_object_set_new(request, "params", params);
    json_object_set_new(request, "headers", headers);
    json_object_set_new(request, "body", body);
    
    json_object_set_new(input, "request", request);
    
    char *content_type = NULL;
    json_t *output = middleware->execute(input, arena, get_arena_alloc_wrapper(), NULL, NULL, &content_type, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    TEST_ASSERT_JSON_EQUAL(input, output);
    
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
    destroy_mock_middleware(middleware);
}

static void test_middleware_error_propagation(void) {
    Middleware *middleware = create_mock_middleware("test", mock_middleware_error);
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *input = json_object();
    json_object_set_new(input, "test", json_string("value"));
    
    char *content_type = NULL;
    json_t *output = middleware->execute(input, arena, get_arena_alloc_wrapper(), NULL, NULL, &content_type, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    
    // Verify error structure
    json_t *errors = json_object_get(output, "errors");
    TEST_ASSERT_NOT_NULL(errors);
    TEST_ASSERT_TRUE(json_is_array(errors));
    
    json_t *error = json_array_get(errors, 0);
    TEST_ASSERT_NOT_NULL(error);
    
    json_t *type = json_object_get(error, "type");
    json_t *message = json_object_get(error, "message");
    
    TEST_ASSERT_NOT_NULL(type);
    TEST_ASSERT_NOT_NULL(message);
    TEST_ASSERT_STRING_EQUAL("mockError", json_string_value(type));
    TEST_ASSERT_STRING_EQUAL("Mock middleware error", json_string_value(message));
    
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
    destroy_mock_middleware(middleware);
}

static void test_middleware_name_collision_handling(void) {
    Middleware *middleware1 = create_mock_middleware("test", mock_middleware_passthrough);
    Middleware *middleware2 = create_mock_middleware("test", mock_middleware_error);
    
    // Both middlewares have the same name
    TEST_ASSERT_STRING_EQUAL("test", middleware1->name);
    TEST_ASSERT_STRING_EQUAL("test", middleware2->name);
    
    // But different execution functions
    TEST_ASSERT_NOT_EQUAL(middleware1->execute, middleware2->execute);
    
    destroy_mock_middleware(middleware1);
    destroy_mock_middleware(middleware2);
}

static void test_middleware_registry_operations(void) {
    // Test middleware registry operations
    // This would test the actual middleware registration system
    
    // For now, test the interface
    Middleware *middleware = find_middleware("jq");
    // Middleware may or may not exist depending on system state
    
    middleware = find_middleware("nonexistent_middleware_name");
    TEST_ASSERT_NULL(middleware);
}

static void test_middleware_content_type_handling(void) {
    // Test that content type parameter is properly passed
    Middleware *middleware = create_mock_middleware("test", mock_middleware_passthrough);
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *input = json_object();
    json_object_set_new(input, "test", json_string("value"));
    
    char *content_type = NULL;
    json_t *output = middleware->execute(input, arena, get_arena_alloc_wrapper(), NULL, "test config", &content_type, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    // Mock middleware doesn't set content type, so it should remain NULL
    TEST_ASSERT_NULL(content_type);
    
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
    destroy_mock_middleware(middleware);
}

int main(void) {
    UNITY_BEGIN();
    
    RUN_TEST(test_middleware_interface_basic);
    RUN_TEST(test_middleware_execute_passthrough);
    RUN_TEST(test_middleware_execute_with_arena);
    RUN_TEST(test_middleware_execute_error_handling);
    RUN_TEST(test_middleware_execute_with_config);
    RUN_TEST(test_middleware_execute_null_input);
    RUN_TEST(test_middleware_execute_null_arena);
    RUN_TEST(test_middleware_load_function);
    RUN_TEST(test_middleware_find_function);
    RUN_TEST(test_middleware_arena_alloc_function);
    RUN_TEST(test_middleware_memory_management);
    RUN_TEST(test_middleware_concurrent_execution);
    RUN_TEST(test_middleware_json_preservation);
    RUN_TEST(test_middleware_complex_json_handling);
    RUN_TEST(test_middleware_error_propagation);
    RUN_TEST(test_middleware_name_collision_handling);
    RUN_TEST(test_middleware_registry_operations);
    RUN_TEST(test_middleware_content_type_handling);
    
    return UNITY_END();
}
