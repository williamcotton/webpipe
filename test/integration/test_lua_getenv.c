#include "../unity/unity.h"
#include "../helpers/test_utils.h"
#include "../../src/wp.h"
#include <stdlib.h>
#include <string.h>
#include <dlfcn.h>

// Load the actual lua middleware
static void *lua_middleware_handle = NULL;
static json_t *(*lua_middleware_execute)(json_t *, void *, arena_alloc_func, arena_free_func, const char *, json_t *, char **, json_t *) = NULL;

static int load_lua_middleware(void) {
    if (lua_middleware_handle) return 0; // Already loaded
    
    lua_middleware_handle = dlopen("./middleware/lua.so", RTLD_LAZY);
    if (!lua_middleware_handle) {
        fprintf(stderr, "Failed to load lua middleware: %s\n", dlerror());
        return -1;
    }
    
    void *middleware_func = dlsym(lua_middleware_handle, "middleware_execute");
    lua_middleware_execute = (json_t *(*)(json_t *, void *, arena_alloc_func, arena_free_func, const char *, json_t *, char **, json_t *))
                         (uintptr_t)middleware_func;
    if (!middleware_func) {
        fprintf(stderr, "Failed to find middleware_execute in lua middleware: %s\n", dlerror());
        dlclose(lua_middleware_handle);
        lua_middleware_handle = NULL;
        return -1;
    }
    
    return 0;
}

static void unload_lua_middleware(void) {
    if (lua_middleware_handle) {
        dlclose(lua_middleware_handle);
        lua_middleware_handle = NULL;
        lua_middleware_execute = NULL;
    }
}

static json_t *execute_lua_middleware(const char *lua_code, json_t *input) {
    MemoryArena *arena = arena_create(1024 * 1024);
    if (!arena) return NULL;
    
    char *content_type = NULL;
    json_t *result = lua_middleware_execute(input, arena, arena_alloc, arena_free, lua_code, NULL, &content_type, NULL);
    
    arena_free(arena);
    return result;
}

void setUp(void) {
    // Set up function called before each test
    if (load_lua_middleware() != 0) {
        TEST_FAIL_MESSAGE("Failed to load lua middleware");
    }
}

void tearDown(void) {
    // Tear down function called after each test
    unload_lua_middleware();
}

void test_lua_getenv_existing_variable(void) {
    // Set a test environment variable
    setenv("TEST_VAR", "test_value", 1);
    
    const char* lua_code = "return { value = getEnv('TEST_VAR') }";
    json_t* input = json_object();
    json_t* result = execute_lua_middleware(lua_code, input);
    
    TEST_ASSERT_NOT_NULL(result);
    TEST_ASSERT_TRUE(json_is_object(result));
    
    json_t* value = json_object_get(result, "value");
    TEST_ASSERT_NOT_NULL(value);
    TEST_ASSERT_TRUE(json_is_string(value));
    TEST_ASSERT_EQUAL_STRING("test_value", json_string_value(value));
    
    json_decref(input);
    json_decref(result);
    unsetenv("TEST_VAR");
}

void test_lua_getenv_nonexistent_variable(void) {
    const char* lua_code = "return { value = getEnv('NONEXISTENT_VAR') }";
    json_t* input = json_object();
    json_t* result = execute_lua_middleware(lua_code, input);
    
    TEST_ASSERT_NOT_NULL(result);
    TEST_ASSERT_TRUE(json_is_object(result));
    
    json_t* value = json_object_get(result, "value");
    // When Lua returns nil, it doesn't get serialized to JSON, so the key won't exist
    TEST_ASSERT_NULL(value);
    
    json_decref(input);
    json_decref(result);
}

void test_lua_getenv_with_default_value(void) {
    const char* lua_code = "return { value = getEnv('NONEXISTENT_VAR', 'default_value') }";
    json_t* input = json_object();
    json_t* result = execute_lua_middleware(lua_code, input);
    
    TEST_ASSERT_NOT_NULL(result);
    TEST_ASSERT_TRUE(json_is_object(result));
    
    json_t* value = json_object_get(result, "value");
    TEST_ASSERT_NOT_NULL(value);
    TEST_ASSERT_TRUE(json_is_string(value));
    TEST_ASSERT_EQUAL_STRING("default_value", json_string_value(value));
    
    json_decref(input);
    json_decref(result);
}

void test_lua_getenv_existing_over_default(void) {
    // Set a test environment variable
    setenv("TEST_VAR2", "actual_value", 1);
    
    const char* lua_code = "return { value = getEnv('TEST_VAR2', 'default_value') }";
    json_t* input = json_object();
    json_t* result = execute_lua_middleware(lua_code, input);
    
    TEST_ASSERT_NOT_NULL(result);
    TEST_ASSERT_TRUE(json_is_object(result));
    
    json_t* value = json_object_get(result, "value");
    TEST_ASSERT_NOT_NULL(value);
    TEST_ASSERT_TRUE(json_is_string(value));
    TEST_ASSERT_EQUAL_STRING("actual_value", json_string_value(value));
    
    json_decref(input);
    json_decref(result);
    unsetenv("TEST_VAR2");
}

void test_lua_getenv_empty_string_value(void) {
    // Set a test environment variable to empty string
    setenv("TEST_EMPTY", "", 1);
    
    const char* lua_code = "return { value = getEnv('TEST_EMPTY', 'default_value') }";
    json_t* input = json_object();
    json_t* result = execute_lua_middleware(lua_code, input);
    
    TEST_ASSERT_NOT_NULL(result);
    TEST_ASSERT_TRUE(json_is_object(result));
    
    json_t* value = json_object_get(result, "value");
    TEST_ASSERT_NOT_NULL(value);
    TEST_ASSERT_TRUE(json_is_string(value));
    TEST_ASSERT_EQUAL_STRING("", json_string_value(value));  // Empty string should be returned, not default
    
    json_decref(input);
    json_decref(result);
    unsetenv("TEST_EMPTY");
}

void test_lua_getenv_no_arguments(void) {
    const char* lua_code = "return { value = getEnv() }";
    json_t* input = json_object();
    json_t* result = execute_lua_middleware(lua_code, input);
    
    TEST_ASSERT_NOT_NULL(result);
    TEST_ASSERT_TRUE(json_is_object(result));
    
    json_t* value = json_object_get(result, "value");
    // When Lua returns nil, it doesn't get serialized to JSON, so the key won't exist
    TEST_ASSERT_NULL(value);
    
    json_decref(input);
    json_decref(result);
}

void test_lua_getenv_non_string_argument(void) {
    const char* lua_code = "return { value = getEnv(123) }";
    json_t* input = json_object();
    json_t* result = execute_lua_middleware(lua_code, input);
    
    TEST_ASSERT_NOT_NULL(result);
    TEST_ASSERT_TRUE(json_is_object(result));
    
    json_t* value = json_object_get(result, "value");
    // When Lua returns nil, it doesn't get serialized to JSON, so the key won't exist
    TEST_ASSERT_NULL(value);
    
    json_decref(input);
    json_decref(result);
}

void test_lua_getenv_multiple_calls(void) {
    // Set test environment variables
    setenv("VAR1", "value1", 1);
    setenv("VAR2", "value2", 1);
    
    const char* lua_code = "return { "
                          "var1 = getEnv('VAR1'), "
                          "var2 = getEnv('VAR2'), "
                          "var3 = getEnv('VAR3', 'default3') "
                          "}";
    json_t* input = json_object();
    json_t* result = execute_lua_middleware(lua_code, input);
    
    TEST_ASSERT_NOT_NULL(result);
    TEST_ASSERT_TRUE(json_is_object(result));
    
    json_t* var1 = json_object_get(result, "var1");
    TEST_ASSERT_NOT_NULL(var1);
    TEST_ASSERT_TRUE(json_is_string(var1));
    TEST_ASSERT_EQUAL_STRING("value1", json_string_value(var1));
    
    json_t* var2 = json_object_get(result, "var2");
    TEST_ASSERT_NOT_NULL(var2);
    TEST_ASSERT_TRUE(json_is_string(var2));
    TEST_ASSERT_EQUAL_STRING("value2", json_string_value(var2));
    
    json_t* var3 = json_object_get(result, "var3");
    TEST_ASSERT_NOT_NULL(var3);
    TEST_ASSERT_TRUE(json_is_string(var3));
    TEST_ASSERT_EQUAL_STRING("default3", json_string_value(var3));
    
    json_decref(input);
    json_decref(result);
    unsetenv("VAR1");
    unsetenv("VAR2");
}

int main(void) {
    UNITY_BEGIN();
    
    RUN_TEST(test_lua_getenv_existing_variable);
    RUN_TEST(test_lua_getenv_nonexistent_variable);
    RUN_TEST(test_lua_getenv_with_default_value);
    RUN_TEST(test_lua_getenv_existing_over_default);
    RUN_TEST(test_lua_getenv_empty_string_value);
    RUN_TEST(test_lua_getenv_no_arguments);
    RUN_TEST(test_lua_getenv_non_string_argument);
    RUN_TEST(test_lua_getenv_multiple_calls);
    
    return UNITY_END();
}