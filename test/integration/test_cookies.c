#include "../unity/unity.h"
#include "../helpers/test_utils.h"
#include "../../src/wp.h"
#include <string.h>
#include <stdlib.h>
#include <unistd.h>
#include <sys/wait.h>
#include <signal.h>
#include <curl/curl.h>

// Server process ID for cleanup
static pid_t server_pid = 0;

// Helper struct for curl response
struct response_buffer {
    char *data;
    size_t size;
};

static size_t write_callback(void *contents, size_t size, size_t nmemb, void *userp) {
    size_t realsize = size * nmemb;
    struct response_buffer *mem = (struct response_buffer *)userp;
    char *ptr = realloc(mem->data, mem->size + realsize + 1);
    if(ptr == NULL)
        return 0;  // out of memory
    mem->data = ptr;
    memcpy(&(mem->data[mem->size]), contents, realsize);
    mem->size += realsize;
    mem->data[mem->size] = 0;
    return realsize;
}

// Start the WP server in background
static int startServer(void) {
    // Fork a child process to run the server
    server_pid = fork();
    
    if (server_pid == 0) {
        // Child process - run the server
        // Redirect stdout/stderr to suppress output during tests
        freopen("/dev/null", "w", stdout);
        freopen("/dev/null", "w", stderr);
        
        // Redirect stdin from /dev/null so getchar() doesn't block
        freopen("/dev/null", "r", stdin);
        
        // Use test port to avoid conflicts
        char port_arg[16];
        snprintf(port_arg, sizeof(port_arg), "%d", get_test_port());
        execl("./build/wp", "./build/wp", "test/fixtures/test_cookies.wp", "--test", "--port", port_arg, NULL);
        exit(1); // Should not reach here
    } else if (server_pid > 0) {
        // Parent process - wait a moment for server to start
        sleep(2);
        
        // Check if the process is still running
        int status;
        pid_t result = waitpid(server_pid, &status, WNOHANG);
        if (result == server_pid) {
            // Process has exited
            printf("Server process exited with status: %d\n", status);
            server_pid = 0;
            return -1;
        }
        
        return 0;
    } else {
        // Fork failed
        return -1;
    }
}

// Stop the WP server
static void stopServer(void) {
    if (server_pid > 0) {
        kill(server_pid, SIGTERM);
        int status;
        waitpid(server_pid, &status, 0);
        server_pid = 0;
    }
}

void setUp(void) {
    // Set up function called before each test
    // Make sure server is stopped
    stopServer();
    
    // Start server for each test
    if (startServer() != 0) {
        TEST_FAIL_MESSAGE("Failed to start WP server");
    }
}

void tearDown(void) {
    // Tear down function called after each test
    stopServer();
}

// Test fixture for cookie functionality
static void test_cookies_in_request_json(void) {
    MemoryArena *arena = arena_create(1024 * 1024);
    set_current_arena(arena);
    
    // Test that cookies are included in request JSON
    json_t *request = create_request_json(NULL, "/test", "GET", NULL);
    
    TEST_ASSERT_NOT_NULL(request);
    
    // Check that cookies field exists
    json_t *cookies = json_object_get(request, "cookies");
    TEST_ASSERT_NOT_NULL(cookies);
    TEST_ASSERT_TRUE(json_is_object(cookies));
    
    // Should be empty since we passed NULL connection
    TEST_ASSERT_EQUAL(0, json_object_size(cookies));
    
    json_decref(request);
    set_current_arena(NULL);
    arena_free(arena);
}

// Helper function to make HTTP request with cookies
static char *make_http_request_with_cookies(const char *url, const char *cookies) {
    CURL *curl = curl_easy_init();
    if (!curl) return NULL;

    struct curl_slist *headers = NULL;
    if (cookies && strlen(cookies) > 0) {
        char cookie_header[1024];
        snprintf(cookie_header, sizeof(cookie_header), "Cookie: %s", cookies);
        headers = curl_slist_append(headers, cookie_header);
    }

    struct response_buffer chunk = {0};
    curl_easy_setopt(curl, CURLOPT_URL, url);
    curl_easy_setopt(curl, CURLOPT_WRITEFUNCTION, write_callback);
    curl_easy_setopt(curl, CURLOPT_WRITEDATA, (void *)&chunk);
    curl_easy_setopt(curl, CURLOPT_HTTPHEADER, headers);
    curl_easy_setopt(curl, CURLOPT_TIMEOUT, 5L);

    CURLcode res = curl_easy_perform(curl);

    curl_slist_free_all(headers);
    curl_easy_cleanup(curl);

    if (res != CURLE_OK) {
        free(chunk.data);
        return NULL;
    }

    return chunk.data; // caller must free
}

// Test cookie parsing with actual HTTP requests
static void test_cookies_with_http_requests(void) {
    // Test 1: Request without cookies
    char url[256];
    snprintf(url, sizeof(url), "http://localhost:%d/cookies", get_test_port());
    char *response1 = make_http_request_with_cookies(url, NULL);
    TEST_ASSERT_NOT_NULL(response1);
    
    // Parse response to check if cookies field exists and is empty
    json_error_t error;
    json_t *json_response1 = json_loads(response1, 0, &error);
    if (json_response1) {
        json_t *cookies = json_object_get(json_response1, "cookies");
        TEST_ASSERT_NOT_NULL(cookies);
        TEST_ASSERT_TRUE(json_is_object(cookies));
        TEST_ASSERT_EQUAL(0, json_object_size(cookies));
        json_decref(json_response1);
    }
    free(response1);
    
    // Test 2: Request with single cookie
    char *response2 = make_http_request_with_cookies(url, "sessionId=abc123");
    TEST_ASSERT_NOT_NULL(response2);
    
    json_t *json_response2 = json_loads(response2, 0, &error);
    if (json_response2) {
        json_t *cookies = json_object_get(json_response2, "cookies");
        TEST_ASSERT_NOT_NULL(cookies);
        TEST_ASSERT_TRUE(json_is_object(cookies));
        TEST_ASSERT_EQUAL(1, json_object_size(cookies));
        
        json_t *session_id = json_object_get(cookies, "sessionId");
        TEST_ASSERT_NOT_NULL(session_id);
        TEST_ASSERT_STRING_EQUAL("abc123", json_string_value(session_id));
        
        json_decref(json_response2);
    }
    free(response2);
    
    // Test 3: Request with multiple cookies
    char *response3 = make_http_request_with_cookies(url, "sessionId=abc123; user=john; theme=dark");
    TEST_ASSERT_NOT_NULL(response3);
    
    json_t *json_response3 = json_loads(response3, 0, &error);
    if (json_response3) {
        json_t *cookies = json_object_get(json_response3, "cookies");
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
        
        json_decref(json_response3);
    }
    free(response3);
    
    // Test 4: Request with cookies containing special characters
    char *response4 = make_http_request_with_cookies(url, "token=abc%3D123; user=john%40example.com");
    TEST_ASSERT_NOT_NULL(response4);
    
    json_t *json_response4 = json_loads(response4, 0, &error);
    if (json_response4) {
        json_t *cookies = json_object_get(json_response4, "cookies");
        TEST_ASSERT_NOT_NULL(cookies);
        TEST_ASSERT_TRUE(json_is_object(cookies));
        TEST_ASSERT_EQUAL(2, json_object_size(cookies));
        
        json_t *token = json_object_get(cookies, "token");
        json_t *user = json_object_get(cookies, "user");
        
        TEST_ASSERT_NOT_NULL(token);
        TEST_ASSERT_NOT_NULL(user);
        
        TEST_ASSERT_STRING_EQUAL("abc%3D123", json_string_value(token));
        TEST_ASSERT_STRING_EQUAL("john%40example.com", json_string_value(user));
        
        json_decref(json_response4);
    }
    free(response4);
}

// Test cookie parsing edge cases
static void test_cookies_edge_cases(void) {
    MemoryArena *arena = arena_create(1024 * 1024);
    set_current_arena(arena);
    
    // Test malformed cookies
    json_t *cookies1 = parse_cookies("sessionId=abc123; invalid; user=john; =value; name=");
    TEST_ASSERT_NOT_NULL(cookies1);
    TEST_ASSERT_EQUAL(2, json_object_size(cookies1)); // Only valid cookies should be parsed
    
    json_t *session_id = json_object_get(cookies1, "sessionId");
    json_t *user = json_object_get(cookies1, "user");
    
    TEST_ASSERT_NOT_NULL(session_id);
    TEST_ASSERT_NOT_NULL(user);
    TEST_ASSERT_STRING_EQUAL("abc123", json_string_value(session_id));
    TEST_ASSERT_STRING_EQUAL("john", json_string_value(user));
    
    json_decref(cookies1);
    
    // Test empty values
    json_t *cookies2 = parse_cookies("sessionId=; user=john; empty=");
    TEST_ASSERT_NOT_NULL(cookies2);
    TEST_ASSERT_EQUAL(1, json_object_size(cookies2)); // Only cookies with non-empty values
    
    json_t *user2 = json_object_get(cookies2, "user");
    TEST_ASSERT_NOT_NULL(user2);
    TEST_ASSERT_STRING_EQUAL("john", json_string_value(user2));
    
    json_decref(cookies2);
    
    set_current_arena(NULL);
    arena_free(arena);
}

// Test cookies with whitespace handling
static void test_cookies_whitespace_handling(void) {
    MemoryArena *arena = arena_create(1024 * 1024);
    set_current_arena(arena);
    
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
    set_current_arena(NULL);
    arena_free(arena);
}

int main(void) {
    UNITY_BEGIN();
    
    RUN_TEST(test_cookies_in_request_json);
    RUN_TEST(test_cookies_edge_cases);
    RUN_TEST(test_cookies_whitespace_handling);
    
    // Only run HTTP tests if curl is available
    if (system("which curl > /dev/null 2>&1") == 0) {
        RUN_TEST(test_cookies_with_http_requests);
    } else {
        printf("Skipping HTTP cookie tests - curl not available\n");
    }
    
    return UNITY_END();
} 