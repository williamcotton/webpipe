#include "../unity/unity.h"
#include "../helpers/test_utils.h"
#include "../../src/wp.h"
#include <string.h>

// Load the actual pg middleware
#include <dlfcn.h>
static void *pg_middleware_handle = NULL;
static json_t *(*pg_middleware_execute)(json_t *, void *, arena_alloc_func, arena_free_func, const char *, json_t *, char **, json_t *) = NULL;

static int load_pg_middleware(void) {
    if (pg_middleware_handle) return 0; // Already loaded
    
    pg_middleware_handle = dlopen("./middleware/pg.so", RTLD_LAZY);
    if (!pg_middleware_handle) {
        fprintf(stderr, "Failed to load pg middleware: %s\n", dlerror());
        return -1;
    }
    
    void *middleware_func = dlsym(pg_middleware_handle, "middleware_execute");
    pg_middleware_execute = (json_t *(*)(json_t *, void *, arena_alloc_func, arena_free_func, const char *, json_t *, char **, json_t *))
                        (uintptr_t)middleware_func;
    if (!middleware_func) {
        fprintf(stderr, "Failed to find middleware_execute in pg middleware: %s\n", dlerror());
        dlclose(pg_middleware_handle);
        pg_middleware_handle = NULL;
        return -1;
    }
    
    return 0;
}

static void unload_pg_middleware(void) {
    if (pg_middleware_handle) {
        dlclose(pg_middleware_handle);
        pg_middleware_handle = NULL;
        pg_middleware_execute = NULL;
    }
}

void setUp(void) {
    setup_test_database();
    if (load_pg_middleware() != 0) {
        TEST_FAIL_MESSAGE("Failed to load pg middleware");
    }
}

void tearDown(void) {
    teardown_test_database();
    unload_pg_middleware();
}

static void test_pg_middleware_simple_select(void) {
    MemoryArena *arena = create_test_arena(1024 * 1024);
    
    // Set arena context for JSON allocation
    set_current_arena(arena);
    
    // Set up arena-based jansson allocators to match runtime behavior
    json_set_alloc_funcs(jansson_arena_malloc, jansson_arena_free);
    
    json_t *input = json_object();
    json_t *sqlParams = json_array();
    json_object_set_new(input, "sqlParams", sqlParams);
    
    const char *config = "SELECT 1 as test_value";
    
    // Create a simple middleware config for testing
    json_t *middleware_config = json_object();
    json_object_set_new(middleware_config, "host", json_string("localhost"));
    json_object_set_new(middleware_config, "port", json_string("5432"));
    json_object_set_new(middleware_config, "database", json_string("express-test"));
    json_object_set_new(middleware_config, "user", json_string("postgres"));
    json_object_set_new(middleware_config, "password", json_string("postgres"));
    json_object_set_new(middleware_config, "ssl", json_boolean(false));
    
    char *contentType = NULL;
    json_t *variables = json_object();
    
    json_t *output = pg_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, config, middleware_config, &contentType, variables);
    
    TEST_ASSERT_NOT_NULL(output);
    
    json_t *data = json_object_get(output, "data");
    TEST_ASSERT_NOT_NULL(data);
    
    json_t *rows = json_object_get(data, "rows");
    TEST_ASSERT_NOT_NULL(rows);
    TEST_ASSERT_TRUE(json_is_array(rows));
    
    // Don't call json_decref on arena-allocated objects - they're freed with the arena
    
    // Restore default jansson allocators
    json_set_alloc_funcs(malloc, free);
    
    // Clear arena context before cleanup
    set_current_arena(NULL);
    destroy_test_arena(arena);
}

static void test_pg_middleware_parameterized_query(void) {
    MemoryArena *arena = create_test_arena(1024 * 1024);
    
    // Set arena context for JSON allocation
    set_current_arena(arena);
    
    // Set up arena-based jansson allocators to match runtime behavior
    json_set_alloc_funcs(jansson_arena_malloc, jansson_arena_free);
    
    json_t *input = json_object();
    json_t *sqlParams = json_array();
    json_array_append_new(sqlParams, json_string("123"));
    json_object_set_new(input, "sqlParams", sqlParams);
    
    const char *config = "SELECT * FROM teams WHERE id = $1";
    
    // Create a simple middleware config for testing
    json_t *middleware_config = json_object();
    json_object_set_new(middleware_config, "host", json_string("localhost"));
    json_object_set_new(middleware_config, "port", json_string("5432"));
    json_object_set_new(middleware_config, "database", json_string("express-test"));
    json_object_set_new(middleware_config, "user", json_string("postgres"));
    json_object_set_new(middleware_config, "password", json_string("postgres"));
    json_object_set_new(middleware_config, "ssl", json_boolean(false));
    
    char *contentType = NULL;
    json_t *variables = json_object();
    
    json_t *output = pg_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, config, middleware_config, &contentType, variables);
    
    TEST_ASSERT_NOT_NULL(output);
    
    // Don't call json_decref on arena-allocated objects - they're freed with the arena
    
    // Restore default jansson allocators
    json_set_alloc_funcs(malloc, free);
    
    // Clear arena context before cleanup
    set_current_arena(NULL);
    destroy_test_arena(arena);
}

static void test_pg_middleware_sql_error_handling(void) {
    MemoryArena *arena = create_test_arena(1024 * 1024);
    
    // Set arena context for JSON allocation
    set_current_arena(arena);
    
    // Set up arena-based jansson allocators to match runtime behavior
    json_set_alloc_funcs(jansson_arena_malloc, jansson_arena_free);
    
    json_t *input = json_object();
    json_t *sqlParams = json_array();
    json_object_set_new(input, "sqlParams", sqlParams);
    
    const char *config = "SELECT * FROM nonexistent_table";
    
    // Create a simple middleware config for testing
    json_t *middleware_config = json_object();
    json_object_set_new(middleware_config, "host", json_string("localhost"));
    json_object_set_new(middleware_config, "port", json_string("5432"));
    json_object_set_new(middleware_config, "database", json_string("express-test"));
    json_object_set_new(middleware_config, "user", json_string("postgres"));
    json_object_set_new(middleware_config, "password", json_string("postgres"));
    json_object_set_new(middleware_config, "ssl", json_boolean(false));
    
    char *contentType = NULL;
    json_t *variables = json_object();
    
    json_t *output = pg_middleware_execute(input, arena, get_arena_alloc_wrapper(), NULL, config, middleware_config, &contentType, variables);
    
    TEST_ASSERT_NOT_NULL(output);
    
    json_t *errors = json_object_get(output, "errors");
    if (errors) {
        TEST_ASSERT_TRUE(json_is_array(errors));
        TEST_ASSERT_GREATER_THAN(0, json_array_size(errors));
    }
    
    // Don't call json_decref on arena-allocated objects - they're freed with the arena
    
    // Restore default jansson allocators
    json_set_alloc_funcs(malloc, free);
    
    // Clear arena context before cleanup
    set_current_arena(NULL);
    destroy_test_arena(arena);
}



int main(void) {
    UNITY_BEGIN();
    
    RUN_TEST(test_pg_middleware_simple_select);
    RUN_TEST(test_pg_middleware_parameterized_query);
    RUN_TEST(test_pg_middleware_sql_error_handling);
    
    return UNITY_END();
}
