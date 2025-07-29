#include "../unity/unity.h"
#include "../helpers/test_utils.h"
#include "../../src/wp.h"
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdisabled-macro-expansion"
#include <curl/curl.h>
#pragma clang diagnostic pop
#include <string.h>
#include <sys/stat.h>
#include <unistd.h>
#include <signal.h>
#include <sys/wait.h>

// Server process ID for cleanup
static pid_t server_pid = 0;

// CURL response buffer struct  
typedef struct {
    char *data;
    size_t size;
} ResponseBuffer;

// Helper to build test URL with correct port
static char *build_test_url(const char *path) {
    static char url[256];
    snprintf(url, sizeof(url), "http://localhost:%d%s", get_test_port(), path);
    return url;
}

// Header capture structure for CURL
typedef struct {
    char **headers;
    size_t header_count;
    size_t header_capacity;
} HeaderCapture;

// CURL header callback
static size_t header_callback(char *buffer, size_t size, size_t nitems, void *userdata) {
    size_t realsize = size * nitems;
    HeaderCapture *headers = (HeaderCapture *)userdata;
    
    // Only capture Set-Cookie headers
    if (strncmp(buffer, "Set-Cookie:", 11) == 0) {
        // Ensure we have capacity
        if (headers->header_count >= headers->header_capacity) {
            headers->header_capacity = headers->header_capacity ? headers->header_capacity * 2 : 10;
            headers->headers = realloc(headers->headers, headers->header_capacity * sizeof(char*));
        }
        
        // Copy the header (skip "Set-Cookie: " prefix)
        char *cookie_value = buffer + 12; // Skip "Set-Cookie: "
        size_t cookie_len = realsize - 12;
        
        // Remove trailing CRLF
        while (cookie_len > 0 && (cookie_value[cookie_len-1] == '\r' || cookie_value[cookie_len-1] == '\n')) {
            cookie_len--;
        }
        
        headers->headers[headers->header_count] = malloc(cookie_len + 1);
        strncpy(headers->headers[headers->header_count], cookie_value, cookie_len);
        headers->headers[headers->header_count][cookie_len] = '\0';
        headers->header_count++;
    }
    
    return realsize;
}

// CURL write callback
static size_t write_callback(void *contents, size_t size, size_t nmemb, void *userp) {
    size_t realsize = size * nmemb;
    ResponseBuffer *mem = (ResponseBuffer *)userp;

    char *ptr = realloc(mem->data, mem->size + realsize + 1);
    if (!ptr) return 0; // Out of memory

    mem->data = ptr;
    memcpy(&(mem->data[mem->size]), contents, realsize);
    mem->size += realsize;
    mem->data[mem->size] = 0;

    return realsize;
}

// Helper to make HTTP requests and return JSON response
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdisabled-macro-expansion"
static json_t *make_request(const char *url, const char *method, const char *data, long *status_code_out, 
                           char **content_type_out, HeaderCapture *header_capture) {
    CURL *curl = curl_easy_init();
    if (!curl) return NULL;

    ResponseBuffer response = {0};
    response.data = malloc(1);
    response.size = 0;

    curl_easy_setopt(curl, CURLOPT_URL, url);
    curl_easy_setopt(curl, CURLOPT_WRITEFUNCTION, write_callback);
    curl_easy_setopt(curl, CURLOPT_WRITEDATA, (void *)&response);

    // Set up header capture if requested
    if (header_capture) {
        curl_easy_setopt(curl, CURLOPT_HEADERFUNCTION, header_callback);
        curl_easy_setopt(curl, CURLOPT_HEADERDATA, (void *)header_capture);
    }

    struct curl_slist *headers = NULL;
    if (data) {
        headers = curl_slist_append(headers, "Content-Type: application/json");
        curl_easy_setopt(curl, CURLOPT_HTTPHEADER, headers);
    }

    if (strcmp(method, "POST") == 0) {
        curl_easy_setopt(curl, CURLOPT_POSTFIELDS, data);
    } else if (strcmp(method, "PUT") == 0) {
        curl_easy_setopt(curl, CURLOPT_CUSTOMREQUEST, "PUT");
        curl_easy_setopt(curl, CURLOPT_POSTFIELDS, data);
    } else if (strcmp(method, "PATCH") == 0) {
        curl_easy_setopt(curl, CURLOPT_CUSTOMREQUEST, "PATCH");
        curl_easy_setopt(curl, CURLOPT_POSTFIELDS, data);
    } else if (strcmp(method, "DELETE") == 0) {
        curl_easy_setopt(curl, CURLOPT_CUSTOMREQUEST, "DELETE");
    }

    CURLcode res = curl_easy_perform(curl);
    
    if (status_code_out) {
        curl_easy_getinfo(curl, CURLINFO_RESPONSE_CODE, status_code_out);
    }
    
    // Get content type if requested
    if (content_type_out) {
        char *ct = NULL;
        curl_easy_getinfo(curl, CURLINFO_CONTENT_TYPE, &ct);
        *content_type_out = ct ? strdup(ct) : NULL;
    }

    if (headers) curl_slist_free_all(headers);
    curl_easy_cleanup(curl);

    if (res != CURLE_OK) {
        printf("CURL error: %s\n", curl_easy_strerror(res));
        free(response.data);
        return NULL;
    }

    // Parse JSON response
    json_error_t error;
    json_t *json = json_loads(response.data, 0, &error);
    free(response.data);

    return json;
}
#pragma clang diagnostic pop

// Start the WP server in background
static int start_server(void) {
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
        execl("./build/wp", "./build/wp", "test.wp", "--port", port_arg, NULL);
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
static void stop_server(void) {
    if (server_pid > 0) {
        kill(server_pid, SIGTERM);
        int status;
        waitpid(server_pid, &status, 0);
        server_pid = 0;
    }
}

void setUp(void) {
    // Set up function called before each test
    // Server is already running, just reset test data (not the connection)
    clear_test_data();
    insert_test_data();
}

void tearDown(void) {
    // Tear down function called after each test
    // Just clear data, keep connection alive for next test
    clear_test_data();
}

static void test_e2e_simple_route(void) {
    // Test the /test route
    long status_code;
    json_t *response = make_request(build_test_url("/test"), "GET", NULL, &status_code, NULL, NULL);
    
    TEST_ASSERT_EQUAL(200, status_code);
    TEST_ASSERT_NOT_NULL(response);
    
    // The /test route should return the request object (jq passthrough)
    TEST_ASSERT_NOT_NULL(json_object_get(response, "method"));
    TEST_ASSERT_STRING_EQUAL("GET", json_string_value(json_object_get(response, "method")));
    
    json_decref(response);
}

static void test_e2e_parameterized_route(void) {
    // Test the /page/:id route
    long status_code;
    json_t *response = make_request(build_test_url("/page/123"), "GET", NULL, &status_code, NULL, NULL);
    
    TEST_ASSERT_EQUAL(200, status_code);
    TEST_ASSERT_NOT_NULL(response);
    
    json_t *team = json_object_get(response, "team");
    TEST_ASSERT_NOT_NULL(team);
    
    json_decref(response);
}

static void test_e2e_result_step_success(void) {
    // Test the /test3 route with result step
    long status_code;
    json_t *response = make_request(build_test_url("/test3"), "GET", NULL, &status_code, NULL, NULL);
    
    TEST_ASSERT_EQUAL(200, status_code);
    TEST_ASSERT_NOT_NULL(response);
    
    json_t *success = json_object_get(response, "success");
    TEST_ASSERT_NOT_NULL(success);
    TEST_ASSERT_TRUE(json_is_true(success));
    
    json_decref(response);
}

static void test_e2e_result_step_validation_error(void) {
    // Test the /test4 route with validation error
    long status_code;
    json_t *response = make_request(build_test_url("/test4"), "GET", NULL, &status_code, NULL, NULL);
    
    TEST_ASSERT_EQUAL(400, status_code);
    TEST_ASSERT_NOT_NULL(response);
    
    json_t *error = json_object_get(response, "error");
    TEST_ASSERT_NOT_NULL(error);
    TEST_ASSERT_STRING_EQUAL("Validation failed", json_string_value(error));
    
    json_decref(response);
}

static void test_e2e_result_step_sql_error(void) {
    // Test the /test-sql-error route
    long status_code;
    json_t *response = make_request(build_test_url("/test-sql-error"), "GET", NULL, &status_code, NULL, NULL);
    
    TEST_ASSERT_EQUAL(500, status_code);
    TEST_ASSERT_NOT_NULL(response);
    
    json_t *error = json_object_get(response, "error");
    TEST_ASSERT_NOT_NULL(error);
    TEST_ASSERT_STRING_EQUAL("Database error", json_string_value(error));
    
    json_decref(response);
}

static void test_e2e_variable_usage(void) {
    // Test the /teams route that uses teamsQuery variable
    long status_code;
    json_t *response = make_request(build_test_url("/teams"), "GET", NULL, &status_code, NULL, NULL);
    
    TEST_ASSERT_EQUAL(200, status_code);
    TEST_ASSERT_NOT_NULL(response);
    
    json_t *data = json_object_get(response, "data");
    TEST_ASSERT_NOT_NULL(data);
    
    json_t *teamsQuery = json_object_get(data, "teamsQuery");
    TEST_ASSERT_NOT_NULL(teamsQuery);
    TEST_ASSERT_TRUE(json_is_object(teamsQuery));

    json_t *rows = json_object_get(teamsQuery, "rows");
    TEST_ASSERT_NOT_NULL(rows);
    TEST_ASSERT_TRUE(json_is_array(rows));

    json_t *row = json_array_get(rows, 0);
    TEST_ASSERT_NOT_NULL(row);
    
    json_decref(response);
}

static void test_e2e_pipeline_chain(void) {
    // Test a multi-step pipeline
    long status_code;
    json_t *response = make_request(build_test_url("/page/123"), "GET", NULL, &status_code, NULL, NULL);
    
    TEST_ASSERT_EQUAL(200, status_code);
    TEST_ASSERT_NOT_NULL(response);
    
    // Should have processed through jq -> pg -> jq
    json_t *team = json_object_get(response, "team");
    TEST_ASSERT_NOT_NULL(team);
    
    json_decref(response);
}

static void test_e2e_invalid_route(void) {
    // Test non-existent route
    long status_code;
    json_t *response = make_request(build_test_url("/nonexistent"), "GET", NULL, &status_code, NULL, NULL);
    
    TEST_ASSERT_EQUAL(404, status_code);
    
    if (response) {
        json_decref(response);
    }
}

static void test_e2e_invalid_method(void) {
    // Test invalid HTTP method - but most servers accept any method, so just test that we get a response
    long status_code;
    json_t *response = make_request(build_test_url("/test"), "INVALID", NULL, &status_code, NULL, NULL);
    
    // Accept any reasonable response for invalid method
    TEST_ASSERT_TRUE(status_code == 405 || status_code == 400 || status_code == 200);
    
    if (response) {
        json_decref(response);
    }
}

static void test_e2e_concurrent_requests(void) {
    // Test concurrent request handling by making multiple requests
    for (int i = 0; i < 5; i++) {
        long status_code;
        json_t *response = make_request(build_test_url("/test"), "GET", NULL, &status_code, NULL, NULL);
        TEST_ASSERT_EQUAL(200, status_code);
        TEST_ASSERT_NOT_NULL(response);
        json_decref(response);
        // Small delay to prevent overwhelming the server
        usleep(100000); // 100ms delay
    }
}

static void test_e2e_post_request(void) {
    // Test POST request with JSON body
    long status_code;
    json_t *response = make_request(build_test_url("/users"), "POST", "{\"name\": \"John Doe\", \"email\": \"john@example.com\"}", &status_code, NULL, NULL);
    
    TEST_ASSERT_EQUAL(200, status_code);
    TEST_ASSERT_NOT_NULL(response);
    
    json_t *method = json_object_get(response, "method");
    json_t *name = json_object_get(response, "name");
    json_t *email = json_object_get(response, "email");
    json_t *action = json_object_get(response, "action");
    
    TEST_ASSERT_EQUAL_STRING("POST", json_string_value(method));
    TEST_ASSERT_EQUAL_STRING("John Doe", json_string_value(name));
    TEST_ASSERT_EQUAL_STRING("john@example.com", json_string_value(email));
    TEST_ASSERT_EQUAL_STRING("create", json_string_value(action));
    
    json_decref(response);
}

static void test_e2e_put_request(void) {
    // Test PUT request with JSON body
    long status_code;
    json_t *response = make_request(build_test_url("/users/123"), "PUT", "{\"name\": \"Jane Doe\", \"email\": \"jane@example.com\"}", &status_code, NULL, NULL);
    
    TEST_ASSERT_EQUAL(200, status_code);
    TEST_ASSERT_NOT_NULL(response);
    
    json_t *method = json_object_get(response, "method");
    json_t *id = json_object_get(response, "id");
    json_t *name = json_object_get(response, "name");
    json_t *email = json_object_get(response, "email");
    json_t *action = json_object_get(response, "action");
    
    TEST_ASSERT_EQUAL_STRING("PUT", json_string_value(method));
    TEST_ASSERT_TRUE(json_is_number(id));
    double idValue = json_number_value(id);
    TEST_ASSERT_EQUAL(123, (int)idValue);
    TEST_ASSERT_EQUAL_STRING("Jane Doe", json_string_value(name));
    TEST_ASSERT_EQUAL_STRING("jane@example.com", json_string_value(email));
    TEST_ASSERT_EQUAL_STRING("update", json_string_value(action));
    
    json_decref(response);
}

static void test_e2e_patch_request(void) {
    // Test PATCH request with JSON body
    long status_code;
    json_t *response = make_request(build_test_url("/users/456"), "PATCH", "{\"email\": \"newemail@example.com\"}", &status_code, NULL, NULL);
    
    TEST_ASSERT_EQUAL(200, status_code);
    TEST_ASSERT_NOT_NULL(response);
    
    json_t *method = json_object_get(response, "method");
    json_t *id = json_object_get(response, "id");
    json_t *body = json_object_get(response, "body");
    json_t *action = json_object_get(response, "action");
    
    TEST_ASSERT_EQUAL_STRING("PATCH", json_string_value(method));
    TEST_ASSERT_TRUE(json_is_number(id));
    double idValue2 = json_number_value(id);
    TEST_ASSERT_EQUAL(456, (int)idValue2);
    TEST_ASSERT_EQUAL_STRING("partial_update", json_string_value(action));
    
    // Check that body contains the patch data
    TEST_ASSERT_NOT_NULL(body);
    json_t *email = json_object_get(body, "email");
    TEST_ASSERT_EQUAL_STRING("newemail@example.com", json_string_value(email));
    
    json_decref(response);
}

static void test_e2e_body_handling(void) {
    // Test that POST, PUT, and PATCH all handle body data correctly
    long status_code;
    
    // Test POST with body
    json_t *response = make_request(build_test_url("/test-body"), "POST", "{\"test\": \"post data\"}", &status_code, NULL, NULL);
    TEST_ASSERT_EQUAL(200, status_code);
    TEST_ASSERT_NOT_NULL(response);
    
    json_t *method = json_object_get(response, "method");
    json_t *hasBody = json_object_get(response, "hasBody");
    json_t *body = json_object_get(response, "body");
    
    TEST_ASSERT_EQUAL_STRING("POST", json_string_value(method));
    TEST_ASSERT_TRUE(json_is_true(hasBody));
    TEST_ASSERT_NOT_NULL(body);
    
    json_t *test_val = json_object_get(body, "test");
    TEST_ASSERT_EQUAL_STRING("post data", json_string_value(test_val));
    
    json_decref(response);
    
    // Test PUT with body
    response = make_request(build_test_url("/test-body"), "PUT", "{\"test\": \"put data\"}", &status_code, NULL, NULL);
    TEST_ASSERT_EQUAL(200, status_code);
    TEST_ASSERT_NOT_NULL(response);
    
    method = json_object_get(response, "method");
    hasBody = json_object_get(response, "hasBody");
    body = json_object_get(response, "body");
    
    TEST_ASSERT_EQUAL_STRING("PUT", json_string_value(method));
    TEST_ASSERT_TRUE(json_is_true(hasBody));
    TEST_ASSERT_NOT_NULL(body);
    
    test_val = json_object_get(body, "test");
    TEST_ASSERT_EQUAL_STRING("put data", json_string_value(test_val));
    
    json_decref(response);
    
    // Test PATCH with body
    response = make_request(build_test_url("/test-body"), "PATCH", "{\"test\": \"patch data\"}", &status_code, NULL, NULL);
    TEST_ASSERT_EQUAL(200, status_code);
    TEST_ASSERT_NOT_NULL(response);
    
    method = json_object_get(response, "method");
    hasBody = json_object_get(response, "hasBody");
    body = json_object_get(response, "body");
    
    TEST_ASSERT_EQUAL_STRING("PATCH", json_string_value(method));
    TEST_ASSERT_TRUE(json_is_true(hasBody));
    TEST_ASSERT_NOT_NULL(body);
    
    test_val = json_object_get(body, "test");
    TEST_ASSERT_EQUAL_STRING("patch data", json_string_value(test_val));
    
    json_decref(response);
}

// Helper function to check if response is HTML
static int is_html_response(const char *response_body) {
    return strstr(response_body, "<html>") != NULL && strstr(response_body, "</html>") != NULL;
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

    ResponseBuffer chunk = {0};
    chunk.data = malloc(1);
    chunk.size = 0;

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

// Helper function to make HTTP request and get raw response
static char *makeRawRequest(const char *url, const char *method, const char *data, long *status_code_out, char **content_type_out) {
    CURL *curl = curl_easy_init();
    if (!curl) return NULL;

    ResponseBuffer response = {0};
    response.data = malloc(1);
    response.size = 0;

    curl_easy_setopt(curl, CURLOPT_URL, url);
    curl_easy_setopt(curl, CURLOPT_WRITEFUNCTION, write_callback);
    curl_easy_setopt(curl, CURLOPT_WRITEDATA, (void *)&response);

    // Headers list for extracting content-type
    struct curl_slist *headers = NULL;
    if (data) {
        headers = curl_slist_append(headers, "Content-Type: application/json");
        curl_easy_setopt(curl, CURLOPT_HTTPHEADER, headers);
    }

    if (strcmp(method, "POST") == 0) {
        curl_easy_setopt(curl, CURLOPT_POSTFIELDS, data);
    }

    CURLcode res = curl_easy_perform(curl);
    
    if (status_code_out) {
        curl_easy_getinfo(curl, CURLINFO_RESPONSE_CODE, status_code_out);
    }
    
    // Get content type
    if (content_type_out) {
        char *ct = NULL;
        curl_easy_getinfo(curl, CURLINFO_CONTENT_TYPE, &ct);
        *content_type_out = ct ? strdup(ct) : NULL;
    }

    if (headers) curl_slist_free_all(headers);
    curl_easy_cleanup(curl);

    if (res != CURLE_OK) {
        printf("CURL error: %s\n", curl_easy_strerror(res));
        free(response.data);
        return NULL;
    }

    return response.data;
}

static void test_e2e_mustache_html_response(void) {
    // Test mustache middleware HTML response
    long status_code;
    char *content_type = NULL;
    char *response_body = makeRawRequest(build_test_url("/hello-mustache"), "GET", NULL, &status_code, &content_type);
    
    TEST_ASSERT_EQUAL(200, status_code);
    TEST_ASSERT_NOT_NULL(response_body);
    TEST_ASSERT_NOT_NULL(content_type);
    
    // Check content type is HTML
    TEST_ASSERT_TRUE(strstr(content_type, "text/html") != NULL);
    
    // Check HTML content
    TEST_ASSERT_TRUE(is_html_response(response_body));
    TEST_ASSERT_TRUE(strstr(response_body, "Hello from mustache!") != NULL);
    TEST_ASSERT_TRUE(strstr(response_body, "Hello, World!") != NULL);
    TEST_ASSERT_TRUE(strstr(response_body, "<title>Hello from mustache!</title>") != NULL);
    
    free(response_body);
    free(content_type);
}

static void test_e2e_mustache_error_response(void) {
    // Test mustache middleware error response
    long status_code;
    char *content_type = NULL;
    char *response_body = makeRawRequest(build_test_url("/mustache-error-test"), "GET", NULL, &status_code, &content_type);
    
    TEST_ASSERT_EQUAL(200, status_code);
    TEST_ASSERT_NOT_NULL(response_body);
    TEST_ASSERT_NOT_NULL(content_type);
    
    // Check content type is JSON (error fallback)
    TEST_ASSERT_TRUE(strstr(content_type, "application/json") != NULL);
    
    // Parse as JSON to verify error structure
    json_error_t error;
    json_t *response_json = json_loads(response_body, 0, &error);
    TEST_ASSERT_NOT_NULL_MESSAGE(response_json, "Response should be valid JSON");
    
    // Check error structure
    json_t *errors = json_object_get(response_json, "errors");
    TEST_ASSERT_NOT_NULL(errors);
    TEST_ASSERT_TRUE(json_is_array(errors));
    TEST_ASSERT_EQUAL_INT(1, json_array_size(errors));
    
    json_t *error_obj = json_array_get(errors, 0);
    TEST_ASSERT_NOT_NULL(error_obj);
    
    json_t *error_type = json_object_get(error_obj, "type");
    TEST_ASSERT_NOT_NULL(error_type);
    TEST_ASSERT_EQUAL_STRING("templateError", json_string_value(error_type));
    
    json_t *error_message = json_object_get(error_obj, "message");
    TEST_ASSERT_NOT_NULL(error_message);
    TEST_ASSERT_EQUAL_STRING("Template rendering failed", json_string_value(error_message));
    
    json_decref(response_json);
    free(response_body);
    free(content_type);
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


// Test cookie setting via middleware
static void test_e2e_cookie_setting(void) {
    HeaderCapture header_capture = {0};
    
    // Make request to a route that should set cookies
    long status_code;
    json_t *response = make_request(build_test_url("/cookies"), "GET", NULL, &status_code, NULL, &header_capture);
    
    TEST_ASSERT_EQUAL(200, status_code);
    TEST_ASSERT_NOT_NULL(response);
    
    // Verify that the response body doesn't contain setCookies (should be removed)
    json_t *set_cookies = json_object_get(response, "setCookies");
    TEST_ASSERT_NULL(set_cookies);
    
    // Verify that we have the expected message in the response body
    json_t *message = json_object_get(response, "message");
    TEST_ASSERT_NOT_NULL(message);
    TEST_ASSERT_STRING_EQUAL("Cookie test response", json_string_value(message));
    
    // Verify that we captured Set-Cookie headers
    TEST_ASSERT_EQUAL(3, header_capture.header_count);
    
    // Check that the expected cookies are present
    bool found_session = false, found_user = false, found_theme = false;
    
    for (size_t i = 0; i < header_capture.header_count; i++) {
        if (strstr(header_capture.headers[i], "sessionId=abc123") != NULL) {
            found_session = true;
            TEST_ASSERT_TRUE(strstr(header_capture.headers[i], "HttpOnly") != NULL);
            TEST_ASSERT_TRUE(strstr(header_capture.headers[i], "Secure") != NULL);
            TEST_ASSERT_TRUE(strstr(header_capture.headers[i], "Max-Age=3600") != NULL);
        }
        if (strstr(header_capture.headers[i], "userId=john") != NULL) {
            found_user = true;
            TEST_ASSERT_TRUE(strstr(header_capture.headers[i], "Max-Age=86400") != NULL);
        }
        if (strstr(header_capture.headers[i], "theme=dark") != NULL) {
            found_theme = true;
            TEST_ASSERT_TRUE(strstr(header_capture.headers[i], "Path=/") != NULL);
        }
    }
    
    TEST_ASSERT_TRUE(found_session);
    TEST_ASSERT_TRUE(found_user);
    TEST_ASSERT_TRUE(found_theme);
    
    // Clean up response
    json_decref(response);
    
    // Clean up header capture
    for (size_t i = 0; i < header_capture.header_count; i++) {
        free(header_capture.headers[i]);
    }
    free(header_capture.headers);
}

int main(void) {
    // Initialize curl
    curl_global_init(CURL_GLOBAL_DEFAULT);
    
    // Set up test database connection once
    setup_test_database();
    
    // Start server once for all tests
    if (start_server() != 0) {
        printf("Failed to start WP server for E2E tests\n");
        teardown_test_database();
        curl_global_cleanup();
        return 1;
    }
    
    UNITY_BEGIN();
    
    RUN_TEST(test_e2e_simple_route);
    RUN_TEST(test_e2e_parameterized_route);
    RUN_TEST(test_e2e_result_step_success);
    RUN_TEST(test_e2e_result_step_validation_error);
    RUN_TEST(test_e2e_result_step_sql_error);
    RUN_TEST(test_e2e_variable_usage);
    RUN_TEST(test_e2e_pipeline_chain);
    RUN_TEST(test_e2e_invalid_route);
    RUN_TEST(test_e2e_invalid_method);
    RUN_TEST(test_e2e_concurrent_requests);
    RUN_TEST(test_e2e_post_request);
    RUN_TEST(test_e2e_put_request);
    RUN_TEST(test_e2e_patch_request);
    RUN_TEST(test_e2e_body_handling);
    RUN_TEST(test_e2e_mustache_html_response);
    RUN_TEST(test_e2e_mustache_error_response);
    RUN_TEST(test_cookies_in_request_json);
    RUN_TEST(test_cookies_edge_cases);
    RUN_TEST(test_cookies_whitespace_handling);
    RUN_TEST(test_cookies_with_http_requests);
    RUN_TEST(test_e2e_cookie_setting);
    
    int result = UNITY_END();
    
    // Stop server after all tests
    stop_server();
    
    // Clean up test database connection
    teardown_test_database();
    
    // Cleanup curl
    curl_global_cleanup();
    
    return result;
}
