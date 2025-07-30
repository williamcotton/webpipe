# Fetch Middleware Implementation Plan (C) - Simplified

## 1. File Structure and Dependencies

### Files to Create
- `src/middleware/fetch.c` - Main implementation
- Add build target to `Makefile`

### Dependencies
- **libcurl** - HTTP client library
- **jansson** - JSON library (already used)

### Headers to Include
```c
#include <jansson.h>
#include <curl/curl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdbool.h>
```

## 2. Data Structures (Simplified)

### HTTP Response Data (Arena-based)
```c
typedef struct {
    char *memory;
    size_t size;
    void *arena;
    arena_alloc_func alloc_func;
} HttpResponse;

typedef struct {
    json_t *headers;
    void *arena;
    arena_alloc_func alloc_func;
} HttpHeaders;
```

### Global Configuration
```c
// Global fetch configuration (set once at init)
static json_t *fetch_config = NULL;
static int fetch_init_failed = 0;

// Default values
#define DEFAULT_TIMEOUT 30
#define DEFAULT_CONNECT_TIMEOUT 10
#define DEFAULT_MAX_REDIRECTS 5
#define DEFAULT_USER_AGENT "WebPipe/1.0"
```

## 3. Configuration Block Support

### Configuration Getters (follow pg.c pattern)
```c
static const char *get_fetch_string_config(json_t *config, const char *key, const char *default_value);
static bool get_fetch_bool_config(json_t *config, const char *key, bool default_value);
static long get_fetch_long_config(json_t *config, const char *key, long default_value);
```

## 4. Core Functions (Simplified)

### CURL Handle Creation and Configuration
```c
static CURL *create_configured_curl(json_t *config, void *arena, arena_alloc_func alloc_func) {
    CURL *curl = curl_easy_init();
    if (!curl) return NULL;
    
    // Apply global configuration
    const char *user_agent = get_fetch_string_config(config, "userAgent", DEFAULT_USER_AGENT);
    long timeout = get_fetch_long_config(config, "timeout", DEFAULT_TIMEOUT);
    long connect_timeout = get_fetch_long_config(config, "connectTimeout", DEFAULT_CONNECT_TIMEOUT);
    bool follow_redirects = get_fetch_bool_config(config, "followRedirects", true);
    long max_redirects = get_fetch_long_config(config, "maxRedirects", DEFAULT_MAX_REDIRECTS);
    
    curl_easy_setopt(curl, CURLOPT_USERAGENT, user_agent);
    curl_easy_setopt(curl, CURLOPT_TIMEOUT, timeout);
    curl_easy_setopt(curl, CURLOPT_CONNECTTIMEOUT, connect_timeout);
    curl_easy_setopt(curl, CURLOPT_FOLLOWLOCATION, follow_redirects ? 1L : 0L);
    curl_easy_setopt(curl, CURLOPT_MAXREDIRS, max_redirects);
    
    // Security settings
    curl_easy_setopt(curl, CURLOPT_SSL_VERIFYPEER, 1L);
    curl_easy_setopt(curl, CURLOPT_SSL_VERIFYHOST, 2L);
    
    return curl;
}
```

### Memory Callbacks (Arena-based)
```c
static size_t WriteMemoryCallback(void *contents, size_t size, size_t nmemb, HttpResponse *response) {
    size_t realsize = size * nmemb;
    
    // Use arena allocator to grow the response buffer
    char *new_memory = response->alloc_func(response->arena, response->size + realsize + 1);
    if (!new_memory) {
        return 0; // Out of memory
    }
    
    // Copy existing data if we had any
    if (response->memory && response->size > 0) {
        memcpy(new_memory, response->memory, response->size);
    }
    
    // Copy new data
    memcpy(new_memory + response->size, contents, realsize);
    response->size += realsize;
    new_memory[response->size] = '\0'; // Null terminate
    
    response->memory = new_memory;
    return realsize;
}

static size_t HeaderCallback(char *buffer, size_t size, size_t nitems, HttpHeaders *headers) {
    size_t realsize = size * nitems;
    
    // Parse header line: "Header-Name: Value\r\n"
    char *colon = memchr(buffer, ':', realsize);
    if (!colon) return realsize; // Skip non-header lines
    
    // Extract header name (arena allocated)
    size_t name_len = colon - buffer;
    char *name = headers->alloc_func(headers->arena, name_len + 1);
    if (!name) return 0;
    memcpy(name, buffer, name_len);
    name[name_len] = '\0';
    
    // Extract header value (arena allocated)
    char *value_start = colon + 1;
    while (value_start < buffer + realsize && (*value_start == ' ' || *value_start == '\t')) {
        value_start++; // Skip whitespace
    }
    
    char *value_end = buffer + realsize - 1;
    while (value_end > value_start && (*value_end == '\r' || *value_end == '\n' || *value_end == ' ')) {
        value_end--; // Trim whitespace/newlines
    }
    
    size_t value_len = value_end - value_start + 1;
    char *value = headers->alloc_func(headers->arena, value_len + 1);
    if (!value) return 0;
    memcpy(value, value_start, value_len);
    value[value_len] = '\0';
    
    // Add to headers JSON object
    json_object_set_new(headers->headers, name, json_string(value));
    
    return realsize;
}
```

### HTTP Request Execution (Simplified)
```c
static json_t *execute_http_request(const char *url, json_t *request_params, 
                                  void *arena, arena_alloc_func alloc_func, json_t *config) {
    CURL *curl = create_configured_curl(config, arena, alloc_func);
    if (!curl) {
        return create_fetch_error("networkError", "Failed to initialize HTTP client", url, 0);
    }
    
    // Set URL
    curl_easy_setopt(curl, CURLOPT_URL, url);
    
    // Initialize response structures
    HttpResponse response = {0};
    response.arena = arena;
    response.alloc_func = alloc_func;
    
    HttpHeaders headers_data = {0};
    headers_data.arena = arena;
    headers_data.alloc_func = alloc_func;
    headers_data.headers = json_object();
    
    // Set callbacks
    curl_easy_setopt(curl, CURLOPT_WRITEFUNCTION, WriteMemoryCallback);
    curl_easy_setopt(curl, CURLOPT_WRITEDATA, &response);
    curl_easy_setopt(curl, CURLOPT_HEADERFUNCTION, HeaderCallback);
    curl_easy_setopt(curl, CURLOPT_HEADERDATA, &headers_data);
    
    // Configure HTTP method and body
    configure_http_method(curl, request_params, arena, alloc_func);
    
    // Configure headers
    configure_http_headers(curl, request_params, arena, alloc_func);
    
    // Execute request
    CURLcode curl_code = curl_easy_perform(curl);
    
    // Get response code
    long response_code = 0;
    curl_easy_getinfo(curl, CURLINFO_RESPONSE_CODE, &response_code);
    
    // Cleanup curl handle
    curl_easy_cleanup(curl);
    
    // Handle curl errors
    if (curl_code != CURLE_OK) {
        return handle_curl_error(curl_code, url);
    }
    
    // Handle HTTP errors (4xx, 5xx)
    if (response_code >= 400) {
        char error_msg[256];
        snprintf(error_msg, sizeof(error_msg), "HTTP %ld error", response_code);
        return create_fetch_error("httpError", error_msg, url, response_code);
    }
    
    // Create successful response
    return create_fetch_response(&response, &headers_data, response_code, arena, alloc_func);
}
```

## 5. Middleware Interface Functions

### Standard Interface (following pg.c pattern)
```c
int middleware_init(json_t *config) {
    // Store global configuration
    fetch_config = config;
    
    // Initialize libcurl globally (not per-request)
    CURLcode init_result = curl_global_init(CURL_GLOBAL_DEFAULT);
    if (init_result != CURLE_OK) {
        fprintf(stderr, "fetch: Failed to initialize libcurl: %s\n", curl_easy_strerror(init_result));
        fetch_init_failed = 1;
        return 1; // failure
    }
    
    printf("fetch: HTTP client initialized\n");
    return 0; // success
}

json_t *middleware_execute(json_t *input, void *arena, arena_alloc_func alloc_func, 
                          arena_free_func free_func, const char *url_template, 
                          json_t *middleware_config, char **contentType, json_t *variables) {
    (void)free_func; // Not used - arena is freed all at once
    (void)contentType; // fetch middleware produces JSON output
    (void)variables; // Not used for now
    
    if (fetch_init_failed) {
        json_t *response = json_deep_copy(input);
        json_t *error = create_fetch_error("networkError", "HTTP client not initialized", url_template, 0);
        json_object_set_new(response, "errors", json_object_get(error, "errors"));
        return response;
    }
    
    // Determine URL (fetchUrl overrides url_template)
    const char *url = url_template;
    json_t *fetch_url = json_object_get(input, "fetchUrl");
    if (fetch_url && json_is_string(fetch_url)) {
        url = json_string_value(fetch_url);
    }
    
    if (!url || strlen(url) == 0) {
        json_t *response = json_deep_copy(input);
        json_t *error = create_fetch_error("networkError", "No URL specified", "", 0);
        json_object_set_new(response, "errors", json_object_get(error, "errors"));
        return response;
    }
    
    // Security check
    if (!is_url_allowed(url, middleware_config)) {
        json_t *response = json_deep_copy(input);
        json_t *error = create_fetch_error("networkError", "URL not allowed", url, 0);
        json_object_set_new(response, "errors", json_object_get(error, "errors"));
        return response;
    }
    
    // Execute HTTP request
    json_t *http_result = execute_http_request(url, input, arena, alloc_func, middleware_config);
    
    // Check for errors
    if (json_object_get(http_result, "errors")) {
        json_t *response = json_deep_copy(input);
        json_object_set_new(response, "errors", json_deep_copy(json_object_get(http_result, "errors")));
        return response;
    }
    
    // Create response by copying input
    json_t *response = json_deep_copy(input);
    
    // Handle result naming (same as pg.c)
    json_t *result_name = json_object_get(input, "resultName");
    if (result_name && json_is_string(result_name)) {
        // Named result: store under data.resultName
        const char *name = json_string_value(result_name);
        json_t *data_obj = json_object_get(response, "data");
        if (!data_obj || !json_is_object(data_obj)) {
            data_obj = json_object();
            json_object_set_new(response, "data", data_obj);
        }
        json_object_set_new(data_obj, name, http_result);
    } else {
        // Unnamed result: store as { data: { response: ..., status: ..., headers: ... } }
        json_object_set_new(response, "data", http_result);
    }
    
    return response;
}

__attribute__((destructor)) void middleware_destructor(void) {
    curl_global_cleanup();
    fetch_init_failed = 0;
}
```

## 6. Helper Functions

### URL and Query Parameter Building
```c
static char *build_url_with_query(const char *base_url, json_t *query_params, 
                                 void *arena, arena_alloc_func alloc_func) {
    if (!query_params || !json_is_object(query_params) || json_object_size(query_params) == 0) {
        // No query params, return copy of base URL
        size_t url_len = strlen(base_url);
        char *result = alloc_func(arena, url_len + 1);
        if (result) {
            strcpy(result, base_url);
        }
        return result;
    }
    
    // Calculate needed size and build query string
    size_t total_size = strlen(base_url) + 1; // +1 for '?'
    
    // Build query string using arena allocation
    // ... implementation details for URL encoding and concatenation
}
```

### HTTP Method and Body Configuration
```c
static void configure_http_method(CURL *curl, json_t *input, void *arena, arena_alloc_func alloc_func) {
    json_t *method_json = json_object_get(input, "fetchMethod");
    const char *method = method_json && json_is_string(method_json) ? 
                        json_string_value(method_json) : "GET";
    
    json_t *body_json = json_object_get(input, "fetchBody");
    char *body_data = NULL;
    
    if (body_json) {
        // Convert JSON body to string using arena
        body_data = json_dumps_arena(body_json, arena, alloc_func);
    }
    
    if (strcmp(method, "GET") == 0) {
        curl_easy_setopt(curl, CURLOPT_HTTPGET, 1L);
    } else if (strcmp(method, "POST") == 0) {
        curl_easy_setopt(curl, CURLOPT_POST, 1L);
        if (body_data) curl_easy_setopt(curl, CURLOPT_POSTFIELDS, body_data);
    } else if (strcmp(method, "PUT") == 0) {
        curl_easy_setopt(curl, CURLOPT_CUSTOMREQUEST, "PUT");
        if (body_data) curl_easy_setopt(curl, CURLOPT_POSTFIELDS, body_data);
    } else if (strcmp(method, "DELETE") == 0) {
        curl_easy_setopt(curl, CURLOPT_CUSTOMREQUEST, "DELETE");
    } else if (strcmp(method, "PATCH") == 0) {
        curl_easy_setopt(curl, CURLOPT_CUSTOMREQUEST, "PATCH");
        if (body_data) curl_easy_setopt(curl, CURLOPT_POSTFIELDS, body_data);
    }
}
```

## 7. Error Handling (Simplified)

### Error Creation Functions
```c
static json_t *create_fetch_error(const char *type, const char *message, const char *url, long status_code) {
    json_t *error_obj = json_object();
    json_t *errors_array = json_array();
    json_t *error_detail = json_object();
    
    json_object_set_new(error_detail, "type", json_string(type));
    json_object_set_new(error_detail, "message", json_string(message));
    if (url && strlen(url) > 0) {
        json_object_set_new(error_detail, "url", json_string(url));
    }
    if (status_code > 0) {
        json_object_set_new(error_detail, "status", json_integer(status_code));
    }
    
    json_array_append_new(errors_array, error_detail);
    json_object_set_new(error_obj, "errors", errors_array);
    
    return error_obj;
}

static json_t *handle_curl_error(CURLcode curl_code, const char *url) {
    switch (curl_code) {
        case CURLE_OPERATION_TIMEDOUT:
        case CURLE_TIMEOUT:
            return create_fetch_error("timeoutError", "Request timed out", url, 0);
        case CURLE_COULDNT_CONNECT:
        case CURLE_COULDNT_RESOLVE_HOST:
        case CURLE_COULDNT_RESOLVE_PROXY:
            return create_fetch_error("networkError", "Connection failed", url, 0);
        case CURLE_SSL_CONNECT_ERROR:
        case CURLE_SSL_CERTPROBLEM:
            return create_fetch_error("networkError", "SSL/TLS error", url, 0);
        default:
            return create_fetch_error("networkError", curl_easy_strerror(curl_code), url, 0);
    }
}
```

## 8. Security Features (Simplified)

### Domain Filtering
```c
static bool is_url_allowed(const char *url, json_t *config) {
    // Simple domain checking without complex parsing
    json_t *allowed_domains = json_object_get(config, "allowedDomains");
    json_t *blocked_domains = json_object_get(config, "blockedDomains");
    
    if (!allowed_domains && !blocked_domains) {
        return true; // No restrictions
    }
    
    // Extract domain from URL and check against lists
    // Simple implementation using strstr() for basic matching
    // ... implementation details
    
    return true; // placeholder
}
```

## 9. Makefile Integration

### Add to middleware target
```makefile
middleware: $(BUILD_DIR) $(BUILD_DIR)/jq.so $(BUILD_DIR)/lua.so $(BUILD_DIR)/pg.so $(BUILD_DIR)/mustache.so $(BUILD_DIR)/validate.so $(BUILD_DIR)/auth.so $(BUILD_DIR)/cache.so $(BUILD_DIR)/log.so $(BUILD_DIR)/fetch.so

$(BUILD_DIR)/fetch.so: $(MIDDLEWARE_DIR)/fetch.c
	$(CC) $(CFLAGS) -shared -fPIC -o $@ $< -ljansson -lcurl
```

## 10. Key Advantages of Simplified Approach

1. **No Connection Pooling Complexity** - Each request gets a fresh CURL handle
2. **Arena Memory Management** - All allocations use the per-request arena
3. **Simple Cleanup** - CURL handle cleaned up immediately after use
4. **Thread Safety** - No shared state between requests
5. **Easy to Debug** - Linear execution flow
6. **Follows Existing Patterns** - Same structure as pg.c but simpler

This approach prioritizes simplicity and correctness over maximum performance, which is appropriate for most web application use cases.