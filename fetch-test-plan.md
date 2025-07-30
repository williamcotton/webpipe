# Fetch Middleware Test Plan - microhttpd Mock Server

## Analysis of E2E Test Pattern

From `test/system/test_e2e.c`, the pattern is:
1. **Global setup**: Start server once in `main()` before all tests
2. **Per-test setup**: `setUp()` prepares test data, no server restart
3. **Per-test teardown**: `tearDown()` cleans up test data
4. **Global teardown**: Stop server once in `main()` after all tests

## Fetch Test Structure

### File: `test/integration/test_fetch.c`

### 1. Mock HTTP Server with microhttpd

```c
#include "../unity/unity.h"
#include "../helpers/test_utils.h"
#include "../../src/wp.h"
#include <microhttpd.h>
#include <dlfcn.h>
#include <string.h>

#define TEST_HTTP_PORT 9082
static struct MHD_Daemon *mock_server = NULL;

// Mock server handler
static enum MHD_Result mock_http_handler(void *cls, struct MHD_Connection *connection,
                                       const char *url, const char *method,
                                       const char *version, const char *upload_data,
                                       size_t *upload_data_size, void **con_cls) {
    struct MHD_Response *response;
    enum MHD_Result ret;
    
    // Route: GET /api/test -> JSON success
    if (strcmp(url, "/api/test") == 0 && strcmp(method, "GET") == 0) {
        const char *json_response = "{\"message\": \"Hello World\", \"status\": \"success\"}";
        response = MHD_create_response_from_buffer(strlen(json_response),
                                                 (void*)json_response, MHD_RESPMEM_MUST_COPY);
        MHD_add_response_header(response, "Content-Type", "application/json");
        ret = MHD_queue_response(connection, MHD_HTTP_OK, response);
        MHD_destroy_response(response);
        return ret;
    }
    
    // Route: POST /api/echo -> Echo back request body
    if (strcmp(url, "/api/echo") == 0 && strcmp(method, "POST") == 0) {
        if (*upload_data_size != 0) {
            // First call - consume the upload data
            *upload_data_size = 0;
            return MHD_YES;
        }
        
        // Second call - return the echoed data
        const char *echo_response = "{\"echoed\": true, \"method\": \"POST\"}";
        response = MHD_create_response_from_buffer(strlen(echo_response),
                                                 (void*)echo_response, MHD_RESPMEM_MUST_COPY);
        MHD_add_response_header(response, "Content-Type", "application/json");
        ret = MHD_queue_response(connection, MHD_HTTP_OK, response);
        MHD_destroy_response(response);
        return ret;
    }
    
    // Route: GET /api/notfound -> 404 error
    if (strcmp(url, "/api/notfound") == 0) {
        const char *error_response = "{\"error\": \"Not Found\", \"code\": 404}";
        response = MHD_create_response_from_buffer(strlen(error_response),
                                                 (void*)error_response, MHD_RESPMEM_MUST_COPY);
        MHD_add_response_header(response, "Content-Type", "application/json");
        ret = MHD_queue_response(connection, MHD_HTTP_NOT_FOUND, response);
        MHD_destroy_response(response);
        return ret;
    }
    
    // Route: GET /api/servererror -> 500 error
    if (strcmp(url, "/api/servererror") == 0) {
        const char *error_response = "{\"error\": \"Internal Server Error\", \"code\": 500}";
        response = MHD_create_response_from_buffer(strlen(error_response),
                                                 (void*)error_response, MHD_RESPMEM_MUST_COPY);
        MHD_add_response_header(response, "Content-Type", "application/json");
        ret = MHD_queue_response(connection, MHD_HTTP_INTERNAL_SERVER_ERROR, response);
        MHD_destroy_response(response);
        return ret;
    }
    
    // Route: GET /api/slow -> Delayed response for timeout testing
    if (strcmp(url, "/api/slow") == 0) {
        sleep(3); // 3 second delay
        const char *slow_response = "{\"message\": \"Slow response\", \"delayed\": true}";
        response = MHD_create_response_from_buffer(strlen(slow_response),
                                                 (void*)slow_response, MHD_RESPMEM_MUST_COPY);
        MHD_add_response_header(response, "Content-Type", "application/json");
        ret = MHD_queue_response(connection, MHD_HTTP_OK, response);
        MHD_destroy_response(response);
        return ret;
    }
    
    // Route: GET /api/headers -> Return custom headers
    if (strcmp(url, "/api/headers") == 0) {
        const char *headers_response = "{\"message\": \"Headers test\"}";
        response = MHD_create_response_from_buffer(strlen(headers_response),
                                                 (void*)headers_response, MHD_RESPMEM_MUST_COPY);
        MHD_add_response_header(response, "Content-Type", "application/json");
        MHD_add_response_header(response, "X-Custom-Header", "test-value");
        MHD_add_response_header(response, "X-Server", "MockServer/1.0");
        ret = MHD_queue_response(connection, MHD_HTTP_OK, response);
        MHD_destroy_response(response);
        return ret;
    }
    
    // Route: GET /api/text -> Plain text response
    if (strcmp(url, "/api/text") == 0) {
        const char *text_response = "Hello, this is plain text!";
        response = MHD_create_response_from_buffer(strlen(text_response),
                                                 (void*)text_response, MHD_RESPMEM_MUST_COPY);
        MHD_add_response_header(response, "Content-Type", "text/plain");
        ret = MHD_queue_response(connection, MHD_HTTP_OK, response);
        MHD_destroy_response(response);
        return ret;
    }
    
    // Default: 404
    const char *not_found = "{\"error\": \"Endpoint not found\"}";
    response = MHD_create_response_from_buffer(strlen(not_found),
                                             (void*)not_found, MHD_RESPMEM_MUST_COPY);
    MHD_add_response_header(response, "Content-Type", "application/json");
    ret = MHD_queue_response(connection, MHD_HTTP_NOT_FOUND, response);
    MHD_destroy_response(response);
    return ret;
}

static void start_mock_server(void) {
    if (mock_server) return; // Already running
    
    mock_server = MHD_start_daemon(MHD_USE_THREAD_PER_CONNECTION,
                                  TEST_HTTP_PORT, NULL, NULL,
                                  &mock_http_handler, NULL,
                                  MHD_OPTION_END);
    if (!mock_server) {
        TEST_FAIL_MESSAGE("Failed to start mock HTTP server");
    }
    
    // Give server a moment to start
    usleep(100000); // 100ms
}

static void stop_mock_server(void) {
    if (mock_server) {
        MHD_stop_daemon(mock_server);
        mock_server = NULL;
    }
}
```

### 2. Fetch Middleware Loading (following pg.c pattern)

```c
// Load the actual fetch middleware
static void *fetch_middleware_handle = NULL;
static json_t *(*fetch_middleware_execute)(json_t *, void *, arena_alloc_func, arena_free_func, const char *, json_t *, char **, json_t *) = NULL;

static int load_fetch_middleware(void) {
    if (fetch_middleware_handle) return 0; // Already loaded
    
    fetch_middleware_handle = dlopen("./middleware/fetch.so", RTLD_LAZY);
    if (!fetch_middleware_handle) {
        fprintf(stderr, "Failed to load fetch middleware: %s\n", dlerror());
        return -1;
    }
    
    void *middleware_func = dlsym(fetch_middleware_handle, "middleware_execute");
    fetch_middleware_execute = (json_t *(*)(json_t *, void *, arena_alloc_func, arena_free_func, const char *, json_t *, char **, json_t *))
                            (uintptr_t)middleware_func;
    if (!middleware_func) {
        fprintf(stderr, "Failed to find middleware_execute in fetch middleware: %s\n", dlerror());
        dlclose(fetch_middleware_handle);
        fetch_middleware_handle = NULL;
        return -1;
    }
    
    return 0;
}

static void unload_fetch_middleware(void) {
    if (fetch_middleware_handle) {
        dlclose(fetch_middleware_handle);
        fetch_middleware_handle = NULL;
        fetch_middleware_execute = NULL;
    }
}
```

### 3. Test Setup/Teardown

```c
void setUp(void) {
    // Per-test setup - middleware already loaded, server already running
    // Just prepare any test-specific data
}

void tearDown(void) {
    // Per-test cleanup
    // Server keeps running, middleware stays loaded
}
```

### 4. Helper Functions

```c
// Helper to build mock server URLs
static char *build_mock_url(const char *path) {
    static char url[256];
    snprintf(url, sizeof(url), "http://localhost:%d%s", TEST_HTTP_PORT, path);
    return url;
}

// Helper to create middleware config
static json_t *create_test_middleware_config(void) {
    json_t *config = json_object();
    json_object_set_new(config, "timeout", json_integer(5));
    json_object_set_new(config, "userAgent", json_string("WebPipe-Test/1.0"));
    json_object_set_new(config, "followRedirects", json_boolean(true));
    json_object_set_new(config, "maxRedirects", json_integer(3));
    return config;
}
```

### 5. Core Test Functions

```c
static void test_fetch_simple_get(void) {
    MemoryArena *arena = create_test_arena(1024 * 1024);
    set_current_arena(arena);
    json_set_alloc_funcs(jansson_arena_malloc, jansson_arena_free);
    
    json_t *input = json_object();
    json_object_set_new(input, "method", json_string("GET"));
    json_object_set_new(input, "path", json_string("/test"));
    
    const char *url_template = build_mock_url("/api/test");
    json_t *middleware_config = create_test_middleware_config();
    
    char *contentType = NULL;
    json_t *variables = json_object();
    
    json_t *output = fetch_middleware_execute(input, arena, get_arena_alloc_wrapper(), 
                                            NULL, url_template, middleware_config, 
                                            &contentType, variables);
    
    TEST_ASSERT_NOT_NULL(output);
    
    // Should preserve original fields
    json_t *method = json_object_get(output, "method");
    json_t *path = json_object_get(output, "path");
    TEST_ASSERT_STRING_EQUAL("GET", json_string_value(method));
    TEST_ASSERT_STRING_EQUAL("/test", json_string_value(path));
    
    // Should have data with response structure
    json_t *data = json_object_get(output, "data");
    TEST_ASSERT_NOT_NULL(data);
    
    json_t *response = json_object_get(data, "response");
    json_t *status = json_object_get(data, "status");
    json_t *headers = json_object_get(data, "headers");
    
    TEST_ASSERT_NOT_NULL(response);
    TEST_ASSERT_NOT_NULL(status);
    TEST_ASSERT_NOT_NULL(headers);
    TEST_ASSERT_EQUAL_INT(200, json_integer_value(status));
    
    // Check response content
    json_t *message = json_object_get(response, "message");
    TEST_ASSERT_STRING_EQUAL("Hello World", json_string_value(message));
    
    json_set_alloc_funcs(malloc, free);
    set_current_arena(NULL);
    destroy_test_arena(arena);
}

static void test_fetch_url_override(void) {
    MemoryArena *arena = create_test_arena(1024 * 1024);
    set_current_arena(arena);
    json_set_alloc_funcs(jansson_arena_malloc, jansson_arena_free);
    
    json_t *input = json_object();
    json_object_set_new(input, "method", json_string("GET"));
    json_object_set_new(input, "fetchUrl", json_string(build_mock_url("/api/test")));
    
    // URL template should be ignored
    const char *url_template = "http://ignored.com/ignored";
    json_t *middleware_config = create_test_middleware_config();
    
    char *contentType = NULL;
    json_t *variables = json_object();
    
    json_t *output = fetch_middleware_execute(input, arena, get_arena_alloc_wrapper(), 
                                            NULL, url_template, middleware_config, 
                                            &contentType, variables);
    
    TEST_ASSERT_NOT_NULL(output);
    
    // Should have successful response from overridden URL
    json_t *data = json_object_get(output, "data");
    json_t *status = json_object_get(data, "status");
    TEST_ASSERT_EQUAL_INT(200, json_integer_value(status));
    
    json_set_alloc_funcs(malloc, free);
    set_current_arena(NULL);
    destroy_test_arena(arena);
}

static void test_fetch_post_with_body(void) {
    MemoryArena *arena = create_test_arena(1024 * 1024);
    set_current_arena(arena);
    json_set_alloc_funcs(jansson_arena_malloc, jansson_arena_free);
    
    json_t *input = json_object();
    json_object_set_new(input, "fetchMethod", json_string("POST"));
    
    json_t *body = json_object();
    json_object_set_new(body, "name", json_string("John"));
    json_object_set_new(body, "email", json_string("john@example.com"));
    json_object_set_new(input, "fetchBody", body);
    
    json_t *headers = json_object();
    json_object_set_new(headers, "Content-Type", json_string("application/json"));
    json_object_set_new(input, "fetchHeaders", headers);
    
    const char *url_template = build_mock_url("/api/echo");
    json_t *middleware_config = create_test_middleware_config();
    
    char *contentType = NULL;
    json_t *variables = json_object();
    
    json_t *output = fetch_middleware_execute(input, arena, get_arena_alloc_wrapper(), 
                                            NULL, url_template, middleware_config, 
                                            &contentType, variables);
    
    TEST_ASSERT_NOT_NULL(output);
    
    json_t *data = json_object_get(output, "data");
    json_t *response = json_object_get(data, "response");
    json_t *status = json_object_get(data, "status");
    
    TEST_ASSERT_EQUAL_INT(200, json_integer_value(status));
    
    json_t *echoed = json_object_get(response, "echoed");
    TEST_ASSERT_TRUE(json_is_true(echoed));
    
    json_set_alloc_funcs(malloc, free);
    set_current_arena(NULL);
    destroy_test_arena(arena);
}

static void test_fetch_http_error_404(void) {
    MemoryArena *arena = create_test_arena(1024 * 1024);
    set_current_arena(arena);
    json_set_alloc_funcs(jansson_arena_malloc, jansson_arena_free);
    
    json_t *input = json_object();
    const char *url_template = build_mock_url("/api/notfound");
    json_t *middleware_config = create_test_middleware_config();
    
    char *contentType = NULL;
    json_t *variables = json_object();
    
    json_t *output = fetch_middleware_execute(input, arena, get_arena_alloc_wrapper(), 
                                            NULL, url_template, middleware_config, 
                                            &contentType, variables);
    
    TEST_ASSERT_NOT_NULL(output);
    
    // Should have errors array for HTTP 404
    json_t *errors = json_object_get(output, "errors");
    TEST_ASSERT_NOT_NULL(errors);
    TEST_ASSERT_TRUE(json_is_array(errors));
    
    json_t *first_error = json_array_get(errors, 0);
    json_t *error_type = json_object_get(first_error, "type");
    json_t *status = json_object_get(first_error, "status");
    
    TEST_ASSERT_STRING_EQUAL("httpError", json_string_value(error_type));
    TEST_ASSERT_EQUAL_INT(404, json_integer_value(status));
    
    json_set_alloc_funcs(malloc, free);
    set_current_arena(NULL);
    destroy_test_arena(arena);
}

static void test_fetch_with_resultName(void) {
    MemoryArena *arena = create_test_arena(1024 * 1024);
    set_current_arena(arena);
    json_set_alloc_funcs(jansson_arena_malloc, jansson_arena_free);
    
    json_t *input = json_object();
    json_object_set_new(input, "method", json_string("GET"));
    json_object_set_new(input, "resultName", json_string("apiCall"));
    
    const char *url_template = build_mock_url("/api/test");
    json_t *middleware_config = create_test_middleware_config();
    
    char *contentType = NULL;
    json_t *variables = json_object();
    
    json_t *output = fetch_middleware_execute(input, arena, get_arena_alloc_wrapper(), 
                                            NULL, url_template, middleware_config, 
                                            &contentType, variables);
    
    TEST_ASSERT_NOT_NULL(output);
    
    // Should preserve original fields
    json_t *method = json_object_get(output, "method");
    TEST_ASSERT_STRING_EQUAL("GET", json_string_value(method));
    
    // Should have data.apiCall structure
    json_t *data = json_object_get(output, "data");
    json_t *apiCall = json_object_get(data, "apiCall");
    TEST_ASSERT_NOT_NULL(apiCall);
    
    json_t *response = json_object_get(apiCall, "response");
    json_t *status = json_object_get(apiCall, "status");
    json_t *headers = json_object_get(apiCall, "headers");
    
    TEST_ASSERT_NOT_NULL(response);
    TEST_ASSERT_NOT_NULL(status);
    TEST_ASSERT_NOT_NULL(headers);
    TEST_ASSERT_EQUAL_INT(200, json_integer_value(status));
    
    json_set_alloc_funcs(malloc, free);
    set_current_arena(NULL);
    destroy_test_arena(arena);
}

static void test_fetch_timeout_error(void) {
    MemoryArena *arena = create_test_arena(1024 * 1024);
    set_current_arena(arena);
    json_set_alloc_funcs(jansson_arena_malloc, jansson_arena_free);
    
    json_t *input = json_object();
    json_object_set_new(input, "fetchTimeout", json_integer(1)); // 1 second timeout
    
    const char *url_template = build_mock_url("/api/slow"); // 3 second delay
    json_t *middleware_config = create_test_middleware_config();
    
    char *contentType = NULL;
    json_t *variables = json_object();
    
    json_t *output = fetch_middleware_execute(input, arena, get_arena_alloc_wrapper(), 
                                            NULL, url_template, middleware_config, 
                                            &contentType, variables);
    
    TEST_ASSERT_NOT_NULL(output);
    
    // Should have timeout error
    json_t *errors = json_object_get(output, "errors");
    TEST_ASSERT_NOT_NULL(errors);
    
    json_t *first_error = json_array_get(errors, 0);
    json_t *error_type = json_object_get(first_error, "type");
    
    TEST_ASSERT_STRING_EQUAL("timeoutError", json_string_value(error_type));
    
    json_set_alloc_funcs(malloc, free);
    set_current_arena(NULL);
    destroy_test_arena(arena);
}

// Additional tests for headers, text responses, etc...
```

### 6. Main Function (following e2e pattern)

```c
int main(void) {
    // Initialize curl globally
    curl_global_init(CURL_GLOBAL_DEFAULT);
    
    // Start mock HTTP server once for all tests
    start_mock_server();
    
    // Load fetch middleware once
    if (load_fetch_middleware() != 0) {
        printf("Failed to load fetch middleware for tests\n");
        stop_mock_server();
        curl_global_cleanup();
        return 1;
    }
    
    UNITY_BEGIN();
    
    // Basic functionality tests
    RUN_TEST(test_fetch_simple_get);
    RUN_TEST(test_fetch_url_override);
    RUN_TEST(test_fetch_post_with_body);
    
    // Response processing tests
    RUN_TEST(test_fetch_json_response);
    RUN_TEST(test_fetch_text_response);
    RUN_TEST(test_fetch_with_headers);
    
    // Result naming tests
    RUN_TEST(test_fetch_with_resultName);
    RUN_TEST(test_fetch_without_naming);
    
    // Error handling tests
    RUN_TEST(test_fetch_http_error_404);
    RUN_TEST(test_fetch_http_error_500);
    RUN_TEST(test_fetch_timeout_error);
    RUN_TEST(test_fetch_network_error);
    
    // Configuration tests
    RUN_TEST(test_fetch_with_config);
    RUN_TEST(test_fetch_different_methods);
    
    int result = UNITY_END();
    
    // Clean up - stop server and unload middleware
    unload_fetch_middleware();
    stop_mock_server();
    curl_global_cleanup();
    
    return result;
}
```

### 7. Makefile Integration

```makefile
$(BUILD_DIR)/test_fetch: $(BUILD_DIR)/unity.o $(DOTENV_OBJ) $(TEST_DIR)/integration/test_fetch.c $(TEST_COMMON_SOURCES)
	$(CC) $(TEST_CFLAGS) -o $@ $(TEST_DIR)/integration/test_fetch.c $(TEST_COMMON_SOURCES) $(BUILD_DIR)/unity.o $(TEST_LDFLAGS) -lcurl -lmicrohttpd
```

## Key Benefits of This Approach

1. **Real HTTP Testing**: Uses actual HTTP protocol with microhttpd server
2. **Follows Established Pattern**: Same structure as existing e2e tests  
3. **Single Setup/Teardown**: Server runs once for all tests
4. **Arena Memory Management**: Follows existing memory patterns
5. **Comprehensive Coverage**: Tests all HTTP methods, errors, timeouts
6. **Easy to Extend**: Add new mock endpoints by extending the handler

This approach gives us realistic HTTP testing while maintaining the simplicity and patterns established in the existing test suite.