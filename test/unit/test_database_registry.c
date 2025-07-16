#include "../unity/unity.h"
#include "../../src/database_registry.h"
#include <jansson.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

// Mock arena allocator for testing
typedef struct {
    char data[1024];
    size_t used;
} MockArena;

static void* mock_arena_alloc(void* arena, size_t size) {
    MockArena* mock = (MockArena*)arena;
    if (mock->used + size > sizeof(mock->data)) {
        return NULL;
    }
    void* ptr = mock->data + mock->used;
    mock->used += size;
    return ptr;
}

// Mock execute_sql function for testing
static json_t* mock_execute_sql(const char* sql, json_t* params, void* arena, arena_alloc_func alloc_func) {
    (void)sql;
    (void)params;
    (void)arena;
    (void)alloc_func;
    
    json_t* result = json_object();
    json_t* rows = json_array();
    json_t* row = json_object();
    
    json_object_set_new(row, "id", json_integer(1));
    json_object_set_new(row, "name", json_string("test"));
    json_array_append_new(rows, row);
    
    json_object_set_new(result, "rows", rows);
    json_object_set_new(result, "rowCount", json_integer(1));
    
    return result;
}

// Test fixtures
void setUp(void) {
    // Initialize for each test
}

void tearDown(void) {
    // Cleanup after each test
    if (database_registry_is_initialized()) {
        database_registry_cleanup();
    }
}

// Test database registry initialization
static void test_database_registry_init(void) {
    TEST_ASSERT_FALSE(database_registry_is_initialized());
    
    int result = database_registry_init();
    TEST_ASSERT_EQUAL(0, result);
    TEST_ASSERT_TRUE(database_registry_is_initialized());
    
    // Test double initialization
    result = database_registry_init();
    TEST_ASSERT_EQUAL(0, result);
    TEST_ASSERT_TRUE(database_registry_is_initialized());
}

// Test database registry cleanup
static void test_database_registry_cleanup(void) {
    database_registry_init();
    TEST_ASSERT_TRUE(database_registry_is_initialized());
    
    database_registry_cleanup();
    TEST_ASSERT_FALSE(database_registry_is_initialized());
    
    // Test double cleanup
    database_registry_cleanup();
    TEST_ASSERT_FALSE(database_registry_is_initialized());
}

// Test provider registration
static void test_register_database_provider(void) {
    database_registry_init();
    
    // Test valid registration
    int result = register_database_provider("test_provider", NULL, mock_execute_sql);
    TEST_ASSERT_EQUAL(0, result);
    
    // Test duplicate registration (should update)
    result = register_database_provider("test_provider", NULL, mock_execute_sql);
    TEST_ASSERT_EQUAL(0, result);
    
    // Test invalid parameters
    result = register_database_provider(NULL, NULL, mock_execute_sql);
    TEST_ASSERT_EQUAL(-1, result);
    
    result = register_database_provider("test_provider", NULL, NULL);
    TEST_ASSERT_EQUAL(-1, result);
}

// Test provider access
static void test_get_database_provider(void) {
    database_registry_init();
    
    // Test getting non-existent provider
    DatabaseProvider* provider = get_database_provider("non_existent");
    TEST_ASSERT_NULL(provider);
    
    // Register a provider
    register_database_provider("test_provider", NULL, mock_execute_sql);
    
    // Test getting existing provider
    provider = get_database_provider("test_provider");
    TEST_ASSERT_NOT_NULL(provider);
    TEST_ASSERT_EQUAL_STRING("test_provider", provider->name);
    TEST_ASSERT_EQUAL_PTR(mock_execute_sql, provider->execute_sql);
}

// Test default provider
static void test_get_default_database_provider(void) {
    database_registry_init();
    
    // Test getting default provider when none registered
    DatabaseProvider* provider = get_default_database_provider();
    TEST_ASSERT_NULL(provider);
    
    // Register a provider
    register_database_provider("test_provider", NULL, mock_execute_sql);
    
    // Test getting default provider
    provider = get_default_database_provider();
    TEST_ASSERT_NOT_NULL(provider);
    TEST_ASSERT_EQUAL_STRING("test_provider", provider->name);
    
    // Register another provider
    register_database_provider("second_provider", NULL, mock_execute_sql);
    
    // Default should still be the first one
    provider = get_default_database_provider();
    TEST_ASSERT_NOT_NULL(provider);
    TEST_ASSERT_EQUAL_STRING("test_provider", provider->name);
}

// Test has_database_provider
static void test_has_database_provider(void) {
    database_registry_init();
    
    // Test when no providers registered
    TEST_ASSERT_FALSE(has_database_provider());
    
    // Register a provider
    register_database_provider("test_provider", NULL, mock_execute_sql);
    
    // Test when provider is registered
    TEST_ASSERT_TRUE(has_database_provider());
}

// Test execute_sql function
static void test_execute_sql(void) {
    database_registry_init();
    
    MockArena arena = {0};
    
    // Test execute_sql with no provider registered
    json_t* result = execute_sql("SELECT * FROM test", NULL, &arena, mock_arena_alloc);
    TEST_ASSERT_NOT_NULL(result);
    
    json_t* errors = json_object_get(result, "errors");
    TEST_ASSERT_NOT_NULL(errors);
    TEST_ASSERT_TRUE(json_is_array(errors));
    TEST_ASSERT_EQUAL_INT(1, json_array_size(errors));
    
    json_t* error = json_array_get(errors, 0);
    TEST_ASSERT_NOT_NULL(error);
    
    json_t* type = json_object_get(error, "type");
    TEST_ASSERT_NOT_NULL(type);
    TEST_ASSERT_EQUAL_STRING("databaseError", json_string_value(type));
    
    json_decref(result);
    
    // Register a provider
    register_database_provider("test_provider", NULL, mock_execute_sql);
    
    // Test execute_sql with provider registered
    result = execute_sql("SELECT * FROM test", NULL, &arena, mock_arena_alloc);
    TEST_ASSERT_NOT_NULL(result);
    
    json_t* rows = json_object_get(result, "rows");
    TEST_ASSERT_NOT_NULL(rows);
    TEST_ASSERT_TRUE(json_is_array(rows));
    TEST_ASSERT_EQUAL_INT(1, json_array_size(rows));
    
    json_t* row = json_array_get(rows, 0);
    TEST_ASSERT_NOT_NULL(row);
    
    json_t* id = json_object_get(row, "id");
    TEST_ASSERT_NOT_NULL(id);
    TEST_ASSERT_EQUAL_INT(1, json_integer_value(id));
    
    json_decref(result);
}

// Test provider unregistration
static void test_unregister_database_provider(void) {
    database_registry_init();
    
    // Test unregistering non-existent provider
    int result = unregister_database_provider("non_existent");
    TEST_ASSERT_EQUAL(-1, result);
    
    // Register a provider
    register_database_provider("test_provider", NULL, mock_execute_sql);
    TEST_ASSERT_TRUE(has_database_provider());
    
    // Test unregistering existing provider
    result = unregister_database_provider("test_provider");
    TEST_ASSERT_EQUAL(0, result);
    TEST_ASSERT_FALSE(has_database_provider());
}

// Test provider listing
static void test_list_database_provider_names(void) {
    database_registry_init();
    
    int count = 0;
    
    // Test listing when no providers registered
    char** names = list_database_provider_names(&count);
    TEST_ASSERT_NULL(names);
    TEST_ASSERT_EQUAL(0, count);
    
    // Register providers
    register_database_provider("provider1", NULL, mock_execute_sql);
    register_database_provider("provider2", NULL, mock_execute_sql);
    
    // Test listing providers
    names = list_database_provider_names(&count);
    TEST_ASSERT_NOT_NULL(names);
    TEST_ASSERT_EQUAL(2, count);
    
    bool found_provider1 = false;
    bool found_provider2 = false;
    
    for (int i = 0; i < count; i++) {
        if (strcmp(names[i], "provider1") == 0) {
            found_provider1 = true;
        } else if (strcmp(names[i], "provider2") == 0) {
            found_provider2 = true;
        }
    }
    
    TEST_ASSERT_TRUE(found_provider1);
    TEST_ASSERT_TRUE(found_provider2);
    
    // Cleanup
    for (int i = 0; i < count; i++) {
        free(names[i]);
    }
    free(names);
}

// Test registry statistics
static void test_get_database_registry_stats(void) {
    database_registry_init();
    
    // Test stats with no providers
    DatabaseRegistryStats* stats = get_database_registry_stats();
    TEST_ASSERT_NOT_NULL(stats);
    TEST_ASSERT_EQUAL(0, stats->total_providers);
    TEST_ASSERT_EQUAL(0, stats->available_providers);
    TEST_ASSERT_NULL(stats->provider_names);
    
    free_database_registry_stats(stats);
    
    // Register providers
    register_database_provider("provider1", NULL, mock_execute_sql);
    register_database_provider("provider2", NULL, mock_execute_sql);
    
    // Test stats with providers
    stats = get_database_registry_stats();
    TEST_ASSERT_NOT_NULL(stats);
    TEST_ASSERT_EQUAL(2, stats->total_providers);
    TEST_ASSERT_EQUAL(2, stats->available_providers);
    TEST_ASSERT_NOT_NULL(stats->provider_names);
    
    free_database_registry_stats(stats);
    
    database_registry_cleanup();
}

// Main test runner
int main(void) {
    UNITY_BEGIN();
    
    RUN_TEST(test_database_registry_init);
    RUN_TEST(test_database_registry_cleanup);
    RUN_TEST(test_register_database_provider);
    RUN_TEST(test_get_database_provider);
    RUN_TEST(test_get_default_database_provider);
    RUN_TEST(test_has_database_provider);
    RUN_TEST(test_execute_sql);
    RUN_TEST(test_unregister_database_provider);
    RUN_TEST(test_list_database_provider_names);
    RUN_TEST(test_get_database_registry_stats);
    
    return UNITY_END();
}
