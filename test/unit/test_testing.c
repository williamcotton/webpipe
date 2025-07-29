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

// Test JSON comparison functionality using string dumps (workaround for json_equal issues)
static void test_json_comparison_via_dumps(void) {
    json_t *obj1 = json_loads("{\"id\":1,\"name\":\"test\"}", 0, NULL);
    json_t *obj2 = json_loads("{\"id\":1,\"name\":\"test\"}", 0, NULL);
    json_t *obj3 = json_loads("{\"id\":2,\"name\":\"different\"}", 0, NULL);
    
    // Compare JSON objects by dumping to strings (this is what the testing framework does internally)
    char *str1 = json_dumps(obj1, JSON_SORT_KEYS | JSON_COMPACT);
    char *str2 = json_dumps(obj2, JSON_SORT_KEYS | JSON_COMPACT);
    char *str3 = json_dumps(obj3, JSON_SORT_KEYS | JSON_COMPACT);
    
    TEST_ASSERT_NOT_NULL(str1);
    TEST_ASSERT_NOT_NULL(str2);
    TEST_ASSERT_NOT_NULL(str3);
    
    // Test that identical JSON objects produce identical string representations
    TEST_ASSERT_EQUAL_STRING(str1, str2);
    
    // Test that different JSON objects produce different string representations  
    TEST_ASSERT_FALSE(strcmp(str1, str3) == 0);
    
    free(str1);
    free(str2);
    free(str3);
    json_decref(obj1);
    json_decref(obj2);
    json_decref(obj3);
}

// Test basic JSON operations that the testing framework relies on
static void test_json_operations(void) {
    // Test creating and manipulating JSON objects (used by testing framework)
    json_t *obj = json_object();
    TEST_ASSERT_NOT_NULL(obj);
    
    json_object_set_new(obj, "test", json_string("value"));
    json_t *value = json_object_get(obj, "test");
    TEST_ASSERT_NOT_NULL(value);
    TEST_ASSERT_EQUAL_STRING("value", json_string_value(value));
    
    // Test array operations (used for error arrays)
    json_t *arr = json_array();
    TEST_ASSERT_NOT_NULL(arr);
    
    json_array_append_new(arr, json_string("item1"));
    json_array_append_new(arr, json_string("item2"));
    
    TEST_ASSERT_EQUAL_size_t(2, json_array_size(arr));
    
    json_t *first = json_array_get(arr, 0);
    TEST_ASSERT_EQUAL_STRING("item1", json_string_value(first));
    
    json_decref(obj);
    json_decref(arr);
}

// Test that we can load and parse JSON strings (used throughout testing)
static void test_json_parsing(void) {
    const char *json_str = "{\"success\":true,\"data\":[1,2,3],\"message\":\"hello\"}";
    
    json_t *parsed = json_loads(json_str, 0, NULL);
    TEST_ASSERT_NOT_NULL(parsed);
    TEST_ASSERT_TRUE(json_is_object(parsed));
    
    // Test boolean access
    json_t *success = json_object_get(parsed, "success");
    TEST_ASSERT_TRUE(json_is_boolean(success));
    TEST_ASSERT_TRUE(json_boolean_value(success));
    
    // Test array access
    json_t *data = json_object_get(parsed, "data");
    TEST_ASSERT_TRUE(json_is_array(data));
    TEST_ASSERT_EQUAL_size_t(3, json_array_size(data));
    
    // Test string access
    json_t *message = json_object_get(parsed, "message");
    TEST_ASSERT_TRUE(json_is_string(message));
    TEST_ASSERT_EQUAL_STRING("hello", json_string_value(message));
    
    json_decref(parsed);
}

// Test runner
int main(void) {
    UNITY_BEGIN();
    
    // Basic functionality tests
    RUN_TEST(test_json_comparison_via_dumps);
    RUN_TEST(test_json_operations);
    RUN_TEST(test_json_parsing);
    
    return UNITY_END();
}
