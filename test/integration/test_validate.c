#include "../unity/unity.h"
#include "../helpers/test_utils.h"
#include "../../src/wp.h"
#include <string.h>

// Load the actual validate middleware
#include <dlfcn.h>
static void *validate_middleware_handle = NULL;
static json_t *(*validate_middleware_execute)(json_t *, void *, arena_alloc_func, arena_free_func, const char *, char **, json_t *) = NULL;

static int load_validate_middleware(void) {
    if (validate_middleware_handle) return 0; // Already loaded
    
    validate_middleware_handle = dlopen("./middleware/validate.so", RTLD_LAZY);
    if (!validate_middleware_handle) {
        fprintf(stderr, "Failed to load validate middleware: %s\n", dlerror());
        return -1;
    }
    
    void *middleware_func = dlsym(validate_middleware_handle, "middleware_execute");
    validate_middleware_execute = (json_t *(*)(json_t *, void *, arena_alloc_func, arena_free_func, const char *, char **, json_t *))
                        (uintptr_t)middleware_func;
    if (!middleware_func) {
        fprintf(stderr, "Failed to find middleware_execute in validate middleware: %s\n", dlerror());
        dlclose(validate_middleware_handle);
        validate_middleware_handle = NULL;
        return -1;
    }
    
    return 0;
}

static void unload_validate_middleware(void) {
    if (validate_middleware_handle) {
        dlclose(validate_middleware_handle);
        validate_middleware_handle = NULL;
        validate_middleware_execute = NULL;
    }
}

void setUp(void) {
    // Set up function called before each test
    if (load_validate_middleware() != 0) {
        TEST_FAIL_MESSAGE("Failed to load validate middleware");
    }
}

void tearDown(void) {
    // Tear down function called after each test
    unload_validate_middleware();
}

// Basic field validation tests
static void test_validate_middleware_required_string(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *body = json_object();
    json_object_set_new(body, "name", json_string("John Doe"));
    json_t *input = create_test_request_with_body("POST", "/users", body);
    
    const char *config = "name: string";
    
    json_t *output = validate_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, config, NULL, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    
    // Should not have errors - validation passed
    json_t *errors = json_object_get(output, "errors");
    TEST_ASSERT_NULL(errors);
    
    json_decref(body);
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
}

static void test_validate_middleware_optional_string(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *body = json_object(); // Empty body, no name field
    json_t *input = create_test_request_with_body("POST", "/users", body);
    
    const char *config = "name?: string";
    
    json_t *output = validate_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, config, NULL, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    
    // Should not have errors - optional field can be missing
    json_t *errors = json_object_get(output, "errors");
    TEST_ASSERT_NULL(errors);
    
    json_decref(body);
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
}

static void test_validate_middleware_string_length_constraints(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *body = json_object();
    json_object_set_new(body, "name", json_string("John")); // Too short
    json_t *input = create_test_request_with_body("POST", "/users", body);
    
    const char *config = "name: string(10..100)";
    
    json_t *output = validate_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, config, NULL, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    
    // Should have errors array
    json_t *errors = json_object_get(output, "errors");
    TEST_ASSERT_NOT_NULL(errors);
    TEST_ASSERT_TRUE(json_is_array(errors));
    TEST_ASSERT_EQUAL(1, json_array_size(errors));
    
    json_t *error = json_array_get(errors, 0);
    json_t *type = json_object_get(error, "type");
    json_t *field = json_object_get(error, "field");
    json_t *rule = json_object_get(error, "rule");
    
    TEST_ASSERT_STRING_EQUAL("validationError", json_string_value(type));
    TEST_ASSERT_STRING_EQUAL("name", json_string_value(field));
    TEST_ASSERT_STRING_EQUAL("minLength", json_string_value(rule));
    
    json_decref(body);
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
}

static void test_validate_middleware_number_validation(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *body = json_object();
    json_object_set_new(body, "age", json_integer(25));
    json_t *input = create_test_request_with_body("POST", "/users", body);
    
    const char *config = "age: number";
    
    json_t *output = validate_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, config, NULL, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    
    // Should not have errors
    json_t *errors = json_object_get(output, "errors");
    TEST_ASSERT_NULL(errors);
    
    json_decref(body);
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
}

static void test_validate_middleware_number_range_constraints(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *body = json_object();
    json_object_set_new(body, "age", json_integer(15)); // Too young
    json_t *input = create_test_request_with_body("POST", "/users", body);
    
    const char *config = "age: number(18..120)";
    
    json_t *output = validate_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, config, NULL, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    
    // Should have errors
    json_t *errors = json_object_get(output, "errors");
    TEST_ASSERT_NOT_NULL(errors);
    TEST_ASSERT_TRUE(json_is_array(errors));
    TEST_ASSERT_EQUAL(1, json_array_size(errors));
    
    json_t *error = json_array_get(errors, 0);
    json_t *field = json_object_get(error, "field");
    json_t *rule = json_object_get(error, "rule");
    
    TEST_ASSERT_STRING_EQUAL("age", json_string_value(field));
    TEST_ASSERT_STRING_EQUAL("minimum", json_string_value(rule));
    
    json_decref(body);
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
}

static void test_validate_middleware_email_validation(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *body = json_object();
    json_object_set_new(body, "email", json_string("invalid-email"));
    json_t *input = create_test_request_with_body("POST", "/users", body);
    
    const char *config = "email: email";
    
    json_t *output = validate_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, config, NULL, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    
    json_t *errors = json_object_get(output, "errors");
    TEST_ASSERT_NOT_NULL(errors);
    TEST_ASSERT_EQUAL(1, json_array_size(errors));
    
    json_t *error = json_array_get(errors, 0);
    json_t *field = json_object_get(error, "field");
    TEST_ASSERT_STRING_EQUAL("email", json_string_value(field));
    
    json_decref(body);
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
}

static void test_validate_middleware_valid_email(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *body = json_object();
    json_object_set_new(body, "email", json_string("user@example.com"));
    json_t *input = create_test_request_with_body("POST", "/users", body);
    
    const char *config = "email: email";
    
    json_t *output = validate_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, config, NULL, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    
    // Should not have errors
    json_t *errors = json_object_get(output, "errors");
    TEST_ASSERT_NULL(errors);
    
    json_decref(body);
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
}

static void test_validate_middleware_boolean_validation(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *body = json_object();
    json_object_set_new(body, "active", json_true());
    json_t *input = create_test_request_with_body("POST", "/users", body);
    
    const char *config = "active: boolean";
    
    json_t *output = validate_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, config, NULL, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    
    // Should not have errors
    json_t *errors = json_object_get(output, "errors");
    TEST_ASSERT_NULL(errors);
    
    json_decref(body);
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
}

// DSL parsing tests
static void test_validate_middleware_simple_dsl(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *body = json_object();
    json_object_set_new(body, "name", json_string("John"));
    json_object_set_new(body, "age", json_integer(25));
    json_t *input = create_test_request_with_body("POST", "/users", body);
    
    const char *config = "name: string\nage: number";
    
    json_t *output = validate_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, config, NULL, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    
    // Should not have errors
    json_t *errors = json_object_get(output, "errors");
    TEST_ASSERT_NULL(errors);
    
    json_decref(body);
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
}

static void test_validate_middleware_complex_dsl(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *body = json_object();
    json_object_set_new(body, "name", json_string("John Doe Smith"));
    json_object_set_new(body, "email", json_string("john@example.com"));
    json_object_set_new(body, "age", json_integer(25));
    json_t *input = create_test_request_with_body("POST", "/users", body);
    
    const char *config = "name: string(10..100)\nemail: email\nage: number(18..120)\nteam_id?: number";
    
    json_t *output = validate_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, config, NULL, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    
    // Should not have errors
    json_t *errors = json_object_get(output, "errors");
    TEST_ASSERT_NULL(errors);
    
    json_decref(body);
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
}

static void test_validate_middleware_multiline_dsl(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *body = json_object();
    json_object_set_new(body, "name", json_string("John"));
    json_t *input = create_test_request_with_body("POST", "/users", body);
    
    const char *config = "\n  name: string  \n\n  email?: email  \n";
    
    json_t *output = validate_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, config, NULL, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    
    // Should not have errors
    json_t *errors = json_object_get(output, "errors");
    TEST_ASSERT_NULL(errors);
    
    json_decref(body);
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
}

// Error generation tests
static void test_validate_middleware_missing_required_field(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *body = json_object(); // Empty body
    json_t *input = create_test_request_with_body("POST", "/users", body);
    
    const char *config = "name: string\nemail: email";
    
    json_t *output = validate_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, config, NULL, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    
    json_t *errors = json_object_get(output, "errors");
    TEST_ASSERT_NOT_NULL(errors);
    TEST_ASSERT_TRUE(json_is_array(errors));
    TEST_ASSERT_EQUAL(2, json_array_size(errors)); // Both name and email missing
    
    json_decref(body);
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
}

static void test_validate_middleware_invalid_type(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *body = json_object();
    json_object_set_new(body, "name", json_integer(123)); // Wrong type
    json_t *input = create_test_request_with_body("POST", "/users", body);
    
    const char *config = "name: string";
    
    json_t *output = validate_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, config, NULL, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    
    json_t *errors = json_object_get(output, "errors");
    TEST_ASSERT_NOT_NULL(errors);
    TEST_ASSERT_EQUAL(1, json_array_size(errors));
    
    json_t *error = json_array_get(errors, 0);
    json_t *rule = json_object_get(error, "rule");
    TEST_ASSERT_STRING_EQUAL("type", json_string_value(rule));
    
    json_decref(body);
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
}

static void test_validate_middleware_multiple_errors(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *body = json_object();
    json_object_set_new(body, "name", json_string("Jo")); // Too short
    json_object_set_new(body, "age", json_integer(15)); // Too young
    json_object_set_new(body, "email", json_string("invalid")); // Invalid email
    json_t *input = create_test_request_with_body("POST", "/users", body);
    
    const char *config = "name: string(5..100)\nage: number(18..120)\nemail: email";
    
    json_t *output = validate_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, config, NULL, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    
    json_t *errors = json_object_get(output, "errors");
    TEST_ASSERT_NOT_NULL(errors);
    TEST_ASSERT_TRUE(json_is_array(errors));
    TEST_ASSERT_EQUAL(3, json_array_size(errors)); // All three fields have errors
    
    json_decref(body);
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
}

// Edge cases and error handling
static void test_validate_middleware_null_input(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    const char *config = "name: string";
    
    json_t *output = validate_middleware_execute(NULL, arena, get_arena_alloc_wrapper(), NULL, config, NULL, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    
    json_t *errors = json_object_get(output, "errors");
    TEST_ASSERT_NOT_NULL(errors);
    TEST_ASSERT_EQUAL(1, json_array_size(errors));

    json_decref(output);
    destroy_test_arena(arena);
}

static void test_validate_middleware_null_config(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *input = create_test_request("GET", "/test");
    
    json_t *output = validate_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, NULL, NULL, NULL);
    
    // Should handle null config gracefully by returning input
    TEST_ASSERT_NOT_NULL(output);
    TEST_ASSERT_JSON_EQUAL(input, output);
    
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
}

static void test_validate_middleware_empty_config(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *input = create_test_request("GET", "/test");
    const char *config = "";
    
    json_t *output = validate_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, config, NULL, NULL);
    
    // Should handle empty config gracefully by returning input
    TEST_ASSERT_NOT_NULL(output);
    TEST_ASSERT_JSON_EQUAL(input, output);
    
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
}

static void test_validate_middleware_missing_body(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *input = create_test_request("POST", "/users"); // Has empty body object
    
    const char *config = "name: string";
    
    json_t *output = validate_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, config, NULL, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    
    json_t *errors = json_object_get(output, "errors");
    TEST_ASSERT_NOT_NULL(errors);
    TEST_ASSERT_EQUAL(1, json_array_size(errors));
    
    json_t *error = json_array_get(errors, 0);
    json_t *field = json_object_get(error, "field");
    TEST_ASSERT_STRING_EQUAL("name", json_string_value(field)); // Should be "name", not "body"
    
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
}

// Performance and memory tests
static void test_validate_middleware_memory_arena_usage(void) {
    MemoryArena *arena = create_test_arena(1024);
    size_t initial_used = arena->used;
    
    json_t *body = json_object();
    json_object_set_new(body, "name", json_string("John"));
    json_t *input = create_test_request_with_body("POST", "/users", body);
    
    const char *config = "name: string";
    
    json_t *output = validate_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, config, NULL, NULL);
    
    TEST_ASSERT_NOT_NULL(output);
    
    // Arena should have been used for allocations
    TEST_ASSERT_GREATER_THAN(initial_used, arena->used);
    
    json_decref(body);
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
}

static void test_validate_middleware_performance_simple(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *body = json_object();
    json_object_set_new(body, "name", json_string("John"));
    json_t *input = create_test_request_with_body("POST", "/users", body);
    
    const char *config = "name: string";
    
    start_timer();
    
    for (int i = 0; i < 100; i++) {
        json_t *output = validate_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, config, NULL, NULL);
        TEST_ASSERT_NOT_NULL(output);
        json_decref(output);
    }
    
    assert_execution_time_under(1.0); // Should complete in under 1 second
    
    json_decref(body);
    json_decref(input);
    destroy_test_arena(arena);
}

int main(void) {
    UNITY_BEGIN();
    
    RUN_TEST(test_validate_middleware_required_string);
    RUN_TEST(test_validate_middleware_optional_string);
    RUN_TEST(test_validate_middleware_string_length_constraints);
    RUN_TEST(test_validate_middleware_number_validation);
    RUN_TEST(test_validate_middleware_number_range_constraints);
    RUN_TEST(test_validate_middleware_email_validation);
    RUN_TEST(test_validate_middleware_valid_email);
    RUN_TEST(test_validate_middleware_boolean_validation);
    RUN_TEST(test_validate_middleware_simple_dsl);
    RUN_TEST(test_validate_middleware_complex_dsl);
    RUN_TEST(test_validate_middleware_multiline_dsl);
    RUN_TEST(test_validate_middleware_missing_required_field);
    RUN_TEST(test_validate_middleware_invalid_type);
    RUN_TEST(test_validate_middleware_multiple_errors);
    RUN_TEST(test_validate_middleware_null_input);
    RUN_TEST(test_validate_middleware_null_config);
    RUN_TEST(test_validate_middleware_empty_config);
    RUN_TEST(test_validate_middleware_missing_body);
    RUN_TEST(test_validate_middleware_memory_arena_usage);
    RUN_TEST(test_validate_middleware_performance_simple);
    
    return UNITY_END();
}
