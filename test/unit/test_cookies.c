#include "../unity/unity.h"
#include "../helpers/test_utils.h"
#include "../../src/wp.h"
#include <string.h>
#include <stdlib.h>

// Forward declaration of the parse_cookies function from server.c
json_t *parse_cookies(const char *cookie_header);

void setUp(void) {
    // Set up function called before each test
}

void tearDown(void) {
    // Tear down function called after each test
}

static void test_parse_cookies_null_header(void) {
    json_t *cookies = parse_cookies(NULL);
    
    TEST_ASSERT_NOT_NULL(cookies);
    TEST_ASSERT_TRUE(json_is_object(cookies));
    TEST_ASSERT_EQUAL(0, json_object_size(cookies));
    
    json_decref(cookies);
}

static void test_parse_cookies_empty_header(void) {
    json_t *cookies = parse_cookies("");
    
    TEST_ASSERT_NOT_NULL(cookies);
    TEST_ASSERT_TRUE(json_is_object(cookies));
    TEST_ASSERT_EQUAL(0, json_object_size(cookies));
    
    json_decref(cookies);
}

static void test_parse_cookies_single_cookie(void) {
    json_t *cookies = parse_cookies("sessionId=abc123");
    
    TEST_ASSERT_NOT_NULL(cookies);
    TEST_ASSERT_TRUE(json_is_object(cookies));
    TEST_ASSERT_EQUAL(1, json_object_size(cookies));
    
    json_t *session_id = json_object_get(cookies, "sessionId");
    TEST_ASSERT_NOT_NULL(session_id);
    TEST_ASSERT_STRING_EQUAL("abc123", json_string_value(session_id));
    
    json_decref(cookies);
}

static void test_parse_cookies_multiple_cookies(void) {
    json_t *cookies = parse_cookies("sessionId=abc123; user=john; theme=dark");
    
    TEST_ASSERT_NOT_NULL(cookies);
    TEST_ASSERT_TRUE(json_is_object(cookies));
    TEST_ASSERT_EQUAL(3, json_object_size(cookies));
    
    json_t *session_id = json_object_get(cookies, "sessionId");
    json_t *user = json_object_get(cookies, "user");
    json_t *theme = json_object_get(cookies, "theme");
    
    TEST_ASSERT_NOT_NULL(session_id);
    TEST_ASSERT_NOT_NULL(user);
    TEST_ASSERT_NOT_NULL(theme);
    
    TEST_ASSERT_STRING_EQUAL("abc123", json_string_value(session_id));
    TEST_ASSERT_STRING_EQUAL("john", json_string_value(user));
    TEST_ASSERT_STRING_EQUAL("dark", json_string_value(theme));
    
    json_decref(cookies);
}

static void test_parse_cookies_with_whitespace(void) {
    json_t *cookies = parse_cookies("  sessionId = abc123 ; user = john  ");
    
    TEST_ASSERT_NOT_NULL(cookies);
    TEST_ASSERT_TRUE(json_is_object(cookies));
    TEST_ASSERT_EQUAL(2, json_object_size(cookies));
    
    json_t *session_id = json_object_get(cookies, "sessionId");
    json_t *user = json_object_get(cookies, "user");
    
    TEST_ASSERT_NOT_NULL(session_id);
    TEST_ASSERT_NOT_NULL(user);
    
    TEST_ASSERT_STRING_EQUAL("abc123", json_string_value(session_id));
    TEST_ASSERT_STRING_EQUAL("john", json_string_value(user));
    
    json_decref(cookies);
}

static void test_parse_cookies_with_special_characters(void) {
    json_t *cookies = parse_cookies("token=abc%3D123; user=john%40example.com");
    
    TEST_ASSERT_NOT_NULL(cookies);
    TEST_ASSERT_TRUE(json_is_object(cookies));
    TEST_ASSERT_EQUAL(2, json_object_size(cookies));
    
    json_t *token = json_object_get(cookies, "token");
    json_t *user = json_object_get(cookies, "user");
    
    TEST_ASSERT_NOT_NULL(token);
    TEST_ASSERT_NOT_NULL(user);
    
    TEST_ASSERT_STRING_EQUAL("abc%3D123", json_string_value(token));
    TEST_ASSERT_STRING_EQUAL("john%40example.com", json_string_value(user));
    
    json_decref(cookies);
}

static void test_parse_cookies_malformed_cookies(void) {
    json_t *cookies = parse_cookies("sessionId=abc123; invalid; user=john; =value; name=");
    
    TEST_ASSERT_NOT_NULL(cookies);
    TEST_ASSERT_TRUE(json_is_object(cookies));
    TEST_ASSERT_EQUAL(2, json_object_size(cookies)); // Only valid cookies should be parsed
    
    json_t *session_id = json_object_get(cookies, "sessionId");
    json_t *user = json_object_get(cookies, "user");
    
    TEST_ASSERT_NOT_NULL(session_id);
    TEST_ASSERT_NOT_NULL(user);
    
    TEST_ASSERT_STRING_EQUAL("abc123", json_string_value(session_id));
    TEST_ASSERT_STRING_EQUAL("john", json_string_value(user));
    
    json_decref(cookies);
}

static void test_parse_cookies_empty_values(void) {
    json_t *cookies = parse_cookies("sessionId=; user=john; empty=");
    
    TEST_ASSERT_NOT_NULL(cookies);
    TEST_ASSERT_TRUE(json_is_object(cookies));
    TEST_ASSERT_EQUAL(1, json_object_size(cookies)); // Only cookies with non-empty values
    
    json_t *user = json_object_get(cookies, "user");
    TEST_ASSERT_NOT_NULL(user);
    TEST_ASSERT_STRING_EQUAL("john", json_string_value(user));
    
    json_decref(cookies);
}

static void test_parse_cookies_complex_scenario(void) {
    json_t *cookies = parse_cookies("  sessionId=abc123; user=john%40example.com; theme=dark; lang=en-US; lastVisit=2024-01-15  ");
    
    TEST_ASSERT_NOT_NULL(cookies);
    TEST_ASSERT_TRUE(json_is_object(cookies));
    TEST_ASSERT_EQUAL(5, json_object_size(cookies));
    
    json_t *session_id = json_object_get(cookies, "sessionId");
    json_t *user = json_object_get(cookies, "user");
    json_t *theme = json_object_get(cookies, "theme");
    json_t *lang = json_object_get(cookies, "lang");
    json_t *last_visit = json_object_get(cookies, "lastVisit");
    
    TEST_ASSERT_NOT_NULL(session_id);
    TEST_ASSERT_NOT_NULL(user);
    TEST_ASSERT_NOT_NULL(theme);
    TEST_ASSERT_NOT_NULL(lang);
    TEST_ASSERT_NOT_NULL(last_visit);
    
    TEST_ASSERT_STRING_EQUAL("abc123", json_string_value(session_id));
    TEST_ASSERT_STRING_EQUAL("john%40example.com", json_string_value(user));
    TEST_ASSERT_STRING_EQUAL("dark", json_string_value(theme));
    TEST_ASSERT_STRING_EQUAL("en-US", json_string_value(lang));
    TEST_ASSERT_STRING_EQUAL("2024-01-15", json_string_value(last_visit));
    
    json_decref(cookies);
}

static void test_create_request_json_with_cookies(void) {
    MemoryArena *arena = arena_create(1024 * 1024);
    set_current_arena(arena);
    
    // Create a mock connection with cookies
    // Note: This is a simplified test since we can't easily mock MHD_Connection
    // In a real scenario, this would be tested with actual HTTP requests
    
    json_t *request = create_request_json(NULL, "/test", "GET", NULL);
    
    TEST_ASSERT_NOT_NULL(request);
    
    json_t *cookies = json_object_get(request, "cookies");
    TEST_ASSERT_NOT_NULL(cookies);
    TEST_ASSERT_TRUE(json_is_object(cookies));
    
    // Should be empty since we passed NULL connection
    TEST_ASSERT_EQUAL(0, json_object_size(cookies));
    
    json_decref(request);
    set_current_arena(NULL);
    arena_free(arena);
}

int main(void) {
    UNITY_BEGIN();
    
    RUN_TEST(test_parse_cookies_null_header);
    RUN_TEST(test_parse_cookies_empty_header);
    RUN_TEST(test_parse_cookies_single_cookie);
    RUN_TEST(test_parse_cookies_multiple_cookies);
    RUN_TEST(test_parse_cookies_with_whitespace);
    RUN_TEST(test_parse_cookies_with_special_characters);
    RUN_TEST(test_parse_cookies_malformed_cookies);
    RUN_TEST(test_parse_cookies_empty_values);
    RUN_TEST(test_parse_cookies_complex_scenario);
    RUN_TEST(test_create_request_json_with_cookies);
    
    return UNITY_END();
}
