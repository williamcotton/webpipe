#include "../unity/unity.h"
#include "../helpers/test_utils.h"
#include "../../src/wp.h"
#include <string.h>

// Load the actual pg plugin
#include <dlfcn.h>
static void *pg_plugin_handle = NULL;
static json_t *(*pg_plugin_execute)(json_t *, void *, arena_alloc_func, arena_free_func, const char *) = NULL;

static int load_pg_plugin(void) {
    if (pg_plugin_handle) return 0; // Already loaded
    
    pg_plugin_handle = dlopen("./plugins/pg.so", RTLD_LAZY);
    if (!pg_plugin_handle) {
        fprintf(stderr, "Failed to load pg plugin: %s\n", dlerror());
        return -1;
    }
    
    void *plugin_func = dlsym(pg_plugin_handle, "plugin_execute");
    pg_plugin_execute = (json_t *(*)(json_t *, void *, arena_alloc_func, arena_free_func, const char *))
                        (uintptr_t)plugin_func;
    if (!plugin_func) {
        fprintf(stderr, "Failed to find plugin_execute in pg plugin: %s\n", dlerror());
        dlclose(pg_plugin_handle);
        pg_plugin_handle = NULL;
        return -1;
    }
    
    return 0;
}

static void unload_pg_plugin(void) {
    if (pg_plugin_handle) {
        dlclose(pg_plugin_handle);
        pg_plugin_handle = NULL;
        pg_plugin_execute = NULL;
    }
}

void setUp(void) {
    setup_test_database();
    if (load_pg_plugin() != 0) {
        TEST_FAIL_MESSAGE("Failed to load pg plugin");
    }
}

void tearDown(void) {
    teardown_test_database();
    unload_pg_plugin();
}

static void test_pg_plugin_simple_select(void) {
    MemoryArena *arena = create_test_arena(1024 * 1024);
    
    // Set arena context for JSON allocation
    set_current_arena(arena);
    
    // Set up arena-based jansson allocators to match runtime behavior
    json_set_alloc_funcs(jansson_arena_malloc, jansson_arena_free);
    
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
    
    // Don't call json_decref on arena-allocated objects - they're freed with the arena
    
    // Restore default jansson allocators
    json_set_alloc_funcs(malloc, free);
    
    // Clear arena context before cleanup
    set_current_arena(NULL);
    destroy_test_arena(arena);
}

static void test_pg_plugin_parameterized_query(void) {
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
    
    json_t *output = pg_plugin_execute(input, arena, get_arena_alloc_wrapper(), NULL, config);
    
    TEST_ASSERT_NOT_NULL(output);
    
    // Don't call json_decref on arena-allocated objects - they're freed with the arena
    
    // Restore default jansson allocators
    json_set_alloc_funcs(malloc, free);
    
    // Clear arena context before cleanup
    set_current_arena(NULL);
    destroy_test_arena(arena);
}

static void test_pg_plugin_sql_error_handling(void) {
    MemoryArena *arena = create_test_arena(1024 * 1024);
    
    // Set arena context for JSON allocation
    set_current_arena(arena);
    
    // Set up arena-based jansson allocators to match runtime behavior
    json_set_alloc_funcs(jansson_arena_malloc, jansson_arena_free);
    
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
    
    // Don't call json_decref on arena-allocated objects - they're freed with the arena
    
    // Restore default jansson allocators
    json_set_alloc_funcs(malloc, free);
    
    // Clear arena context before cleanup
    set_current_arena(NULL);
    destroy_test_arena(arena);
}



int main(void) {
    UNITY_BEGIN();
    
    RUN_TEST(test_pg_plugin_simple_select);
    RUN_TEST(test_pg_plugin_parameterized_query);
    RUN_TEST(test_pg_plugin_sql_error_handling);
    
    return UNITY_END();
}
