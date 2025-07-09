#include "../unity/unity.h"
#include "../helpers/test_utils.h"
#include "../../src/wp.h"
#include <string.h>

// Mock pg plugin function for testing
static json_t *pg_plugin_execute(json_t *input, void *arena, arena_alloc_func alloc_func, arena_free_func free_func, const char *config) {
    // For testing, just return a mock response
    (void)arena; (void)alloc_func; (void)free_func; (void)config;
    return json_incref(input);
}

void setUp(void) {
    setup_test_database();
}

void tearDown(void) {
    teardown_test_database();
}

void test_pg_plugin_simple_select(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *input = json_object();
    json_t *sqlParams = json_array();
    json_object_set_new(input, "sqlParams", sqlParams);
    
    const char *config = "SELECT 1 as test_value";
    
    json_t *output = pg_plugin_execute(input, arena, get_arena_alloc_wrapper(), NULL, config);
    
    TEST_ASSERT_NOT_NULL(output);
    
    json_t *data = json_object_get(output, "data");
    TEST_ASSERT_NOT_NULL(data);
    
    json_t *rows = json_object_get(data, "rows");
    TEST_ASSERT_NOT_NULL(rows);
    TEST_ASSERT_TRUE(json_is_array(rows));
    
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
}

void test_pg_plugin_parameterized_query(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *input = json_object();
    json_t *sqlParams = json_array();
    json_array_append_new(sqlParams, json_string("123"));
    json_object_set_new(input, "sqlParams", sqlParams);
    
    const char *config = "SELECT * FROM teams WHERE id = $1";
    
    json_t *output = pg_plugin_execute(input, arena, get_arena_alloc_wrapper(), NULL, config);
    
    TEST_ASSERT_NOT_NULL(output);
    
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
}

void test_pg_plugin_sql_error_handling(void) {
    MemoryArena *arena = create_test_arena(1024);
    
    json_t *input = json_object();
    json_t *sqlParams = json_array();
    json_object_set_new(input, "sqlParams", sqlParams);
    
    const char *config = "SELECT * FROM nonexistent_table";
    
    json_t *output = pg_plugin_execute(input, arena, get_arena_alloc_wrapper(), NULL, config);
    
    TEST_ASSERT_NOT_NULL(output);
    
    json_t *errors = json_object_get(output, "errors");
    if (errors) {
        TEST_ASSERT_TRUE(json_is_array(errors));
        TEST_ASSERT_GREATER_THAN(0, json_array_size(errors));
    }
    
    json_decref(input);
    json_decref(output);
    destroy_test_arena(arena);
}

int main(void) {
    UNITY_BEGIN();
    
    RUN_TEST(test_pg_plugin_simple_select);
    RUN_TEST(test_pg_plugin_parameterized_query);
    RUN_TEST(test_pg_plugin_sql_error_handling);
    
    return UNITY_END();
}