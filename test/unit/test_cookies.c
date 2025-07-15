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

static void test_create_request_json_with_set_cookies(void) {
    MemoryArena *arena = arena_create(1024 * 1024);
    set_current_arena(arena);
    
    json_t *request = create_request_json(NULL, "/test", "GET", NULL);
    
    TEST_ASSERT_NOT_NULL(request);
    
    // Check that setCookies array is initialized
    json_t *set_cookies = json_object_get(request, "setCookies");
    TEST_ASSERT_NOT_NULL(set_cookies);
    TEST_ASSERT_TRUE(json_is_array(set_cookies));
    TEST_ASSERT_EQUAL(0, json_array_size(set_cookies));
    
    json_decref(request);
    set_current_arena(NULL);
    arena_free(arena);
}

static void test_set_cookies_array_manipulation(void) {
    MemoryArena *arena = arena_create(1024 * 1024);
    set_current_arena(arena);
    
    json_t *request = create_request_json(NULL, "/test", "GET", NULL);
    json_t *set_cookies = json_object_get(request, "setCookies");
    
    // Add some cookies
    json_array_append_new(set_cookies, json_string("sessionId=abc123; HttpOnly; Secure"));
    json_array_append_new(set_cookies, json_string("userId=john; Max-Age=3600"));
    json_array_append_new(set_cookies, json_string("theme=dark; Path=/"));
    
    TEST_ASSERT_EQUAL(3, json_array_size(set_cookies));
    
    // Verify cookie strings
    json_t *cookie1 = json_array_get(set_cookies, 0);
    json_t *cookie2 = json_array_get(set_cookies, 1);
    json_t *cookie3 = json_array_get(set_cookies, 2);
    
    TEST_ASSERT_STRING_EQUAL("sessionId=abc123; HttpOnly; Secure", json_string_value(cookie1));
    TEST_ASSERT_STRING_EQUAL("userId=john; Max-Age=3600", json_string_value(cookie2));
    TEST_ASSERT_STRING_EQUAL("theme=dark; Path=/", json_string_value(cookie3));
    
    json_decref(request);
    set_current_arena(NULL);
    arena_free(arena);
}

static void test_set_cookies_with_special_characters(void) {
    MemoryArena *arena = arena_create(1024 * 1024);
    set_current_arena(arena);
    
    json_t *request = create_request_json(NULL, "/test", "GET", NULL);
    json_t *set_cookies = json_object_get(request, "setCookies");
    
    // Add cookies with special characters
    json_array_append_new(set_cookies, json_string("token=abc%3D123; HttpOnly"));
    json_array_append_new(set_cookies, json_string("email=user%40example.com; Secure"));
    
    TEST_ASSERT_EQUAL(2, json_array_size(set_cookies));
    
    json_t *cookie1 = json_array_get(set_cookies, 0);
    json_t *cookie2 = json_array_get(set_cookies, 1);
    
    TEST_ASSERT_STRING_EQUAL("token=abc%3D123; HttpOnly", json_string_value(cookie1));
    TEST_ASSERT_STRING_EQUAL("email=user%40example.com; Secure", json_string_value(cookie2));
    
    json_decref(request);
    set_current_arena(NULL);
    arena_free(arena);
}

static void test_set_cookies_empty_values(void) {
    MemoryArena *arena = arena_create(1024 * 1024);
    set_current_arena(arena);
    
    json_t *request = create_request_json(NULL, "/test", "GET", NULL);
    json_t *set_cookies = json_object_get(request, "setCookies");
    
    // Test empty and null values
    json_array_append_new(set_cookies, json_string(""));
    json_array_append_new(set_cookies, json_string("validCookie=value"));
    
    TEST_ASSERT_EQUAL(2, json_array_size(set_cookies));
    
    json_t *cookie1 = json_array_get(set_cookies, 0);
    json_t *cookie2 = json_array_get(set_cookies, 1);
    
    TEST_ASSERT_STRING_EQUAL("", json_string_value(cookie1));
    TEST_ASSERT_STRING_EQUAL("validCookie=value", json_string_value(cookie2));
    
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
    RUN_TEST(test_create_request_json_with_set_cookies);
    RUN_TEST(test_set_cookies_array_manipulation);
    RUN_TEST(test_set_cookies_with_special_characters);
    RUN_TEST(test_set_cookies_empty_values);
    
    return UNITY_END();
}
