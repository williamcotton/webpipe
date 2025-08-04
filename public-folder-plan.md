# Public Folder Implementation Plan

## Overview
Add support for serving static files from a `./public/` folder in the Web Pipe (wp) runtime. Files should be served directly without pipeline processing, with comprehensive security measures to prevent path traversal attacks.

## Current Architecture Analysis

### Request Flow (src/server.c)
1. `handle_request()` (line 1479) - Main HTTP request handler
2. Arena allocation (5MB per request) 
3. `find_and_process_route()` (line 974) - Route matching
4. `process_route()` (line 944) - Pipeline execution
5. 404 response for unmatched routes (lines 992-994)

### Integration Point
**Location**: `find_and_process_route()` function (lines 974-995)
- Add static file check BEFORE route matching loop
- Fallback to existing route matching if not a static file

## Security Requirements

### Path Traversal Prevention
Based on existing middleware validation pattern (lines 307-320):
- Block `..` sequences
- Block absolute paths (`/`, `\`)  
- Validate filename length
- Ensure files stay within `./public/` boundary

### File Access Controls
- Block hidden files (starting with `.`)
- Block sensitive extensions (`.wp`, `.so`, `.c`, etc.)
- Resolve symlinks and validate real paths
- File existence and read permission checks

### MIME Type Security
- Whitelist safe content types
- Default to `application/octet-stream` for unknown types
- Prevent script execution through content types

## Implementation Plan

### 1. Core Functions to Add

#### `validate_static_path(const char *url_path, char *safe_path, size_t path_size)`
```c
// Returns 0 on success, -1 on security violation
// Converts URL path to safe filesystem path in ./public/
// Performs all security validations
```

#### `get_mime_type(const char *file_path)`
```c
// Returns appropriate Content-Type header
// Based on file extension matching
// Safe defaults for unknown types
```

#### `serve_static_file(struct MHD_Connection *connection, const char *file_path, MemoryArena *arena)`
```c
// Reads file and sends HTTP response
// Uses existing response helper functions
// Proper error handling and cleanup
```

#### `try_serve_static_file(struct MHD_Connection *connection, const char *url, MemoryArena *arena)`
```c
// Main static file handler
// Returns MHD_YES if file served, MHD_NO if not found/invalid
// Integrates all validation and serving logic
```

### 2. Integration Changes

#### Modify `find_and_process_route()` (line 974)
```c
static enum MHD_Result find_and_process_route(struct MHD_Connection *connection,
                                             const char *url, const char *method,
                                             json_t *request, MemoryArena *arena) {
    // NEW: Try static file serving for GET requests
    if (strcmp(method, "GET") == 0) {
        enum MHD_Result static_result = try_serve_static_file(connection, url, arena);
        if (static_result == MHD_YES) {
            return MHD_YES; // File served successfully
        }
        // Fall through to route matching if not a static file
    }
    
    // Existing route matching logic...
    json_incref(request);
    for (int i = 0; i < runtime->program->data.program.statement_count; i++) {
        // ... existing code unchanged
    }
    
    // Existing 404 response
    return send_error_response(connection, 
                             "{\"error\": \"Not found\"}", 
                             MHD_HTTP_NOT_FOUND);
}
```

### 3. File Structure

```
./public/              # Static files directory
├── css/
│   └── styles.css
├── js/
│   └── app.js
├── images/
│   ├── favicon.ico
│   └── logo.png
├── fonts/
│   └── font.woff2
└── index.html
```

### 4. MIME Type Mapping

```c
typedef struct {
    const char *extension;
    const char *mime_type;
} MimeMapping;

static MimeMapping mime_types[] = {
    // Web assets
    {".html", "text/html"},
    {".css", "text/css"},
    {".js", "application/javascript"},
    {".json", "application/json"},
    
    // Images
    {".png", "image/png"},
    {".jpg", "image/jpeg"},
    {".jpeg", "image/jpeg"},
    {".gif", "image/gif"},
    {".svg", "image/svg+xml"},
    {".webp", "image/webp"},
    {".ico", "image/x-icon"},
    
    // Fonts
    {".woff", "font/woff"},
    {".woff2", "font/woff2"},
    {".ttf", "font/ttf"},
    {".otf", "font/otf"},
    
    // Documents
    {".pdf", "application/pdf"},
    {".txt", "text/plain"},
    {".xml", "application/xml"},
    
    // Default
    {NULL, "application/octet-stream"}
};
```

### 5. Security Validation Logic

#### Path Sanitization
```c
int validate_static_path(const char *url_path, char *safe_path, size_t path_size) {
    // Remove leading slash
    if (url_path[0] == '/') url_path++;
    
    // Check for empty path
    if (strlen(url_path) == 0) {
        snprintf(safe_path, path_size, "./public/index.html");
        return 0;
    }
    
    // Security checks
    if (strstr(url_path, "..") || 
        strstr(url_path, "\\") ||
        url_path[0] == '.' ||
        strlen(url_path) > 255) {
        return -1; // Security violation
    }
    
    // Build safe path
    snprintf(safe_path, path_size, "./public/%s", url_path);
    
    // Additional validation
    return validate_file_access(safe_path);
}
```

#### File Access Validation
```c
int validate_file_access(const char *file_path) {
    struct stat file_stat;
    
    // Check file exists and is readable
    if (stat(file_path, &file_stat) != 0) {
        return -1; // File not found
    }
    
    // Must be regular file (not directory or special file)
    if (!S_ISREG(file_stat.st_mode)) {
        return -1; // Not a regular file
    }
    
    // Resolve symlinks and validate real path is within public/
    char real_path[PATH_MAX];
    if (realpath(file_path, real_path) == NULL) {
        return -1; // Path resolution failed
    }
    
    // Ensure resolved path starts with ./public/
    char public_real[PATH_MAX];
    if (realpath("./public", public_real) == NULL) {
        return -1; // Public directory issue
    }
    
    if (strncmp(real_path, public_real, strlen(public_real)) != 0) {
        return -1; // Path outside public directory
    }
    
    return 0; // File is safe to serve
}
```

### 6. Error Handling

- File not found: Continue to route matching (may be dynamic route)
- Security violation: Log warning, return 403 Forbidden
- File read error: Return 500 Internal Server Error
- Memory allocation failure: Return 500 Internal Server Error

### 7. Performance Considerations

#### File Caching
- Initial implementation: No caching (simple)
- Future enhancement: Add ETag/Last-Modified headers
- Future enhancement: In-memory caching for small files

#### Memory Usage
- Use existing arena allocator for file content
- Stream large files instead of loading entirely into memory
- Set reasonable file size limits (e.g., 10MB max)

### 8. Configuration Options

Add to wp runtime configuration:
```c
typedef struct {
    char *public_folder;     // Default: "./public"
    bool enable_static;      // Default: true
    size_t max_file_size;    // Default: 10MB
    bool allow_hidden;       // Default: false
} StaticConfig;
```

### 9. Comprehensive Testing Strategy

The static file serving feature requires extensive testing across all three test categories used by the wp runtime: unit, integration, and system tests.

#### 9.1 Unit Tests (`test/unit/test_static.c`)

**Path Validation Tests:**
```c
void test_validate_static_path_basic(void) {
    char safe_path[256];
    
    // Valid paths
    TEST_ASSERT_EQUAL(0, validate_static_path("/style.css", safe_path, sizeof(safe_path)));
    TEST_ASSERT_STRING_EQUAL("./public/style.css", safe_path);
    
    // Root path -> index.html
    TEST_ASSERT_EQUAL(0, validate_static_path("/", safe_path, sizeof(safe_path)));
    TEST_ASSERT_STRING_EQUAL("./public/index.html", safe_path);
    
    // Subdirectory
    TEST_ASSERT_EQUAL(0, validate_static_path("/css/main.css", safe_path, sizeof(safe_path)));
    TEST_ASSERT_STRING_EQUAL("./public/css/main.css", safe_path);
}

void test_validate_static_path_security(void) {
    char safe_path[256];
    
    // Path traversal attempts
    TEST_ASSERT_EQUAL(-1, validate_static_path("/../etc/passwd", safe_path, sizeof(safe_path)));
    TEST_ASSERT_EQUAL(-1, validate_static_path("/../../secret.txt", safe_path, sizeof(safe_path)));
    TEST_ASSERT_EQUAL(-1, validate_static_path("/dir/../../../etc/hosts", safe_path, sizeof(safe_path)));
    
    // Hidden files
    TEST_ASSERT_EQUAL(-1, validate_static_path("/.env", safe_path, sizeof(safe_path)));
    TEST_ASSERT_EQUAL(-1, validate_static_path("/.git/config", safe_path, sizeof(safe_path)));
    
    // Sensitive extensions
    TEST_ASSERT_EQUAL(-1, validate_static_path("/config.wp", safe_path, sizeof(safe_path)));
    TEST_ASSERT_EQUAL(-1, validate_static_path("/middleware.so", safe_path, sizeof(safe_path)));
    
    // Overlong paths
    char long_path[300];
    memset(long_path, 'a', 299);
    long_path[0] = '/';
    long_path[299] = '\0';
    TEST_ASSERT_EQUAL(-1, validate_static_path(long_path, safe_path, sizeof(safe_path)));
}
```

**MIME Type Detection Tests:**
```c
void test_get_mime_type_web_assets(void) {
    TEST_ASSERT_STRING_EQUAL("text/html", get_mime_type("index.html"));
    TEST_ASSERT_STRING_EQUAL("text/css", get_mime_type("style.css"));
    TEST_ASSERT_STRING_EQUAL("application/javascript", get_mime_type("app.js"));
    TEST_ASSERT_STRING_EQUAL("application/json", get_mime_type("data.json"));
}

void test_get_mime_type_images(void) {
    TEST_ASSERT_STRING_EQUAL("image/png", get_mime_type("logo.png"));
    TEST_ASSERT_STRING_EQUAL("image/jpeg", get_mime_type("photo.jpg"));
    TEST_ASSERT_STRING_EQUAL("image/jpeg", get_mime_type("image.jpeg"));
    TEST_ASSERT_STRING_EQUAL("image/svg+xml", get_mime_type("icon.svg"));
}

void test_get_mime_type_fonts(void) {
    TEST_ASSERT_STRING_EQUAL("font/woff", get_mime_type("font.woff"));
    TEST_ASSERT_STRING_EQUAL("font/woff2", get_mime_type("font.woff2"));
    TEST_ASSERT_STRING_EQUAL("font/ttf", get_mime_type("font.ttf"));
}

void test_get_mime_type_unknown(void) {
    TEST_ASSERT_STRING_EQUAL("application/octet-stream", get_mime_type("unknown.xyz"));
    TEST_ASSERT_STRING_EQUAL("application/octet-stream", get_mime_type("noextension"));
}
```

**File Access Validation Tests:**
```c
void test_validate_file_access_basic(void) {
    // Create test file structure in setUp()
    TEST_ASSERT_EQUAL(0, validate_file_access("./test_public/test.txt"));
    TEST_ASSERT_EQUAL(-1, validate_file_access("./test_public/nonexistent.txt"));
}

void test_validate_file_access_symlink_security(void) {
    // Test symlink boundary validation
    // Create symlink pointing outside public directory
    TEST_ASSERT_EQUAL(-1, validate_file_access("./test_public/evil_symlink"));
    
    // Valid symlink within public directory
    TEST_ASSERT_EQUAL(0, validate_file_access("./test_public/valid_symlink"));
}
```

#### 9.2 Integration Tests (`test/integration/test_static.c`)

**File Serving Tests:**
```c
void test_serve_static_file_basic(void) {
    MemoryArena *arena = create_test_arena(1024 * 1024);
    struct test_http_response *response = create_test_response();
    
    // Create mock connection (using test utilities)
    struct MHD_Connection *connection = create_mock_connection();
    
    // Test CSS file serving
    enum MHD_Result result = serve_static_file(connection, "./test_public/style.css", arena);
    TEST_ASSERT_EQUAL(MHD_YES, result);
    
    // Verify response headers and content type
    verify_response_header(connection, "Content-Type", "text/css");
    
    destroy_test_response(response);
    destroy_test_arena(arena);
}

void test_serve_static_file_large_file(void) {
    MemoryArena *arena = create_test_arena(1024 * 1024);
    
    // Test with file larger than arena buffer
    enum MHD_Result result = serve_static_file(connection, "./test_public/large_image.png", arena);
    TEST_ASSERT_EQUAL(MHD_YES, result);
    verify_response_header(connection, "Content-Type", "image/png");
    
    destroy_test_arena(arena);
}
```

**Security Integration Tests:**
```c
void test_try_serve_static_file_security_blocking(void) {
    MemoryArena *arena = create_test_arena(1024 * 1024);
    struct MHD_Connection *connection = create_mock_connection();
    
    // Test path traversal blocking
    enum MHD_Result result = try_serve_static_file(connection, "/../etc/passwd", arena);
    TEST_ASSERT_EQUAL(MHD_NO, result); // Should be blocked, fall back to routing
    
    // Test hidden file blocking  
    result = try_serve_static_file(connection, "/.env", arena);
    TEST_ASSERT_EQUAL(MHD_NO, result);
    
    destroy_test_arena(arena);
}

void test_static_file_fallback_to_routing(void) {
    MemoryArena *arena = create_test_arena(1024 * 1024);
    struct MHD_Connection *connection = create_mock_connection();
    
    // Request for non-existent static file should fall back to routing
    enum MHD_Result result = try_serve_static_file(connection, "/api/users", arena);
    TEST_ASSERT_EQUAL(MHD_NO, result); // Not a static file
    
    destroy_test_arena(arena);
}
```

#### 9.3 System Tests (`test/system/test_static_e2e.c`)

**End-to-End HTTP Tests:**
```c
void test_static_file_http_requests(void) {
    // Use existing test runtime infrastructure
    int result = init_test_runtime("test_static.wp");
    TEST_ASSERT_EQUAL(0, result);
    
    struct test_http_response *response = create_test_response();
    
    // Test CSS file request
    result = simulate_http_request("GET", "/css/style.css", NULL, response);
    TEST_ASSERT_EQUAL(0, result);
    TEST_ASSERT_EQUAL(200, response->status_code);
    TEST_ASSERT_NOT_NULL(strstr(response->body, "body { margin: 0; }"));
    
    // Test image file request
    result = simulate_http_request("GET", "/images/logo.png", NULL, response);
    TEST_ASSERT_EQUAL(0, result);
    TEST_ASSERT_EQUAL(200, response->status_code);
    
    // Test root request -> index.html
    result = simulate_http_request("GET", "/", NULL, response);
    TEST_ASSERT_EQUAL(0, result);
    TEST_ASSERT_EQUAL(200, response->status_code);
    TEST_ASSERT_NOT_NULL(strstr(response->body, "<html>"));
    
    destroy_test_response(response);
    cleanup_test_runtime();
}

void test_static_file_security_e2e(void) {
    int result = init_test_runtime("test_static.wp");
    TEST_ASSERT_EQUAL(0, result);
    
    struct test_http_response *response = create_test_response();
    
    // Test path traversal returns 403
    result = simulate_http_request("GET", "/../etc/passwd", NULL, response);
    TEST_ASSERT_EQUAL(0, result);
    TEST_ASSERT_EQUAL(403, response->status_code);
    
    // Test hidden file returns 403
    result = simulate_http_request("GET", "/.env", NULL, response);
    TEST_ASSERT_EQUAL(0, result);
    TEST_ASSERT_EQUAL(403, response->status_code);
    
    destroy_test_response(response);
    cleanup_test_runtime();
}

void test_static_file_fallback_to_routes(void) {
    int result = init_test_runtime("test_static.wp");
    TEST_ASSERT_EQUAL(0, result);
    
    struct test_http_response *response = create_test_response();
    
    // Request that should fall back to route matching
    result = simulate_http_request("GET", "/api/users", NULL, response);
    TEST_ASSERT_EQUAL(0, result);
    // Should get route response, not 404
    
    destroy_test_response(response);
    cleanup_test_runtime();
}
```

**Performance and Memory Tests:**
```c
void test_static_file_performance(void) {
    int result = init_test_runtime("test_static.wp");
    TEST_ASSERT_EQUAL(0, result);
    
    struct test_http_response *response = create_test_response();
    
    start_timer();
    
    // Simulate concurrent requests
    for (int i = 0; i < 100; i++) {
        result = simulate_http_request("GET", "/css/style.css", NULL, response);
        TEST_ASSERT_EQUAL(0, result);
        TEST_ASSERT_EQUAL(200, response->status_code);
    }
    
    double elapsed = end_timer();
    assert_execution_time_under(5.0); // Should complete in under 5 seconds
    
    destroy_test_response(response);
    cleanup_test_runtime();
}

void test_static_file_memory_usage(void) {
    // Test that arena memory is properly managed and freed
    MemoryArena *arena = create_test_arena(1024 * 1024);
    size_t initial_used = arena->used;
    
    struct MHD_Connection *connection = create_mock_connection();
    serve_static_file(connection, "./test_public/style.css", arena);
    
    // Memory should be allocated
    TEST_ASSERT_GREATER_THAN(initial_used, arena->used);
    
    // After arena free, memory should be reclaimed (tested by arena utilities)
    destroy_test_arena(arena);
}
```

#### 9.4 Test File Structure

**Test Public Directory:**
```
test/fixtures/public/
├── index.html              # Root file test
├── css/
│   ├── style.css          # CSS serving test
│   └── empty.css          # Empty file test
├── js/
│   └── app.js             # JavaScript serving test
├── images/
│   ├── logo.png           # PNG image test
│   ├── favicon.ico        # ICO test
│   └── large_image.png    # Large file test (>1MB)
├── fonts/
│   └── font.woff2         # Font serving test
├── valid_symlink          # Symlink within public dir
├── evil_symlink -> ../../etc/passwd  # Security test symlink
└── .hidden_file           # Hidden file security test
```

**Test WP Configuration (`test/fixtures/test_static.wp`):**
```wp
GET /api/users
  |> jq: `{ users: ["alice", "bob"] }`

GET /fallback
  |> jq: `{ message: "Route matched, not static" }`
```

#### 9.5 Makefile Integration

**Add to Makefile:**
```makefile
# Static file testing
$(BUILD_DIR)/test_static: $(BUILD_DIR)/unity.o $(DOTENV_OBJ) $(TEST_DIR)/unit/test_static.c $(TEST_COMMON_SOURCES)
	$(CC) $(TEST_CFLAGS) -o $@ $(TEST_DIR)/unit/test_static.c $(TEST_COMMON_SOURCES) $(BUILD_DIR)/unity.o $(TEST_LDFLAGS)

$(BUILD_DIR)/test_static_integration: $(BUILD_DIR)/unity.o $(DOTENV_OBJ) $(TEST_DIR)/integration/test_static.c $(TEST_COMMON_SOURCES)
	$(CC) $(TEST_CFLAGS) -o $@ $(TEST_DIR)/integration/test_static.c $(TEST_COMMON_SOURCES) $(BUILD_DIR)/unity.o $(TEST_LDFLAGS)

$(BUILD_DIR)/test_static_e2e: $(BUILD_DIR)/unity.o $(DOTENV_OBJ) $(TEST_DIR)/system/test_static_e2e.c $(TEST_COMMON_SOURCES)
	$(CC) $(TEST_CFLAGS) -o $@ $(TEST_DIR)/system/test_static_e2e.c $(TEST_COMMON_SOURCES) $(BUILD_DIR)/unity.o $(TEST_LDFLAGS) -lcurl

# Update test lists
TEST_UNIT_BINS += $(BUILD_DIR)/test_static
TEST_INTEGRATION_BINS += $(BUILD_DIR)/test_static_integration  
TEST_SYSTEM_BINS += $(BUILD_DIR)/test_static_e2e
```

#### 9.6 Test Helper Extensions

**Add to `test_utils.h`:**
```c
// Static file testing utilities
void setup_test_public_directory(void);
void cleanup_test_public_directory(void);
void create_test_file(const char *path, const char *content);
void create_test_symlink(const char *link_path, const char *target_path);
struct MHD_Connection *create_mock_connection(void);
void verify_response_header(struct MHD_Connection *conn, const char *header, const char *expected_value);
```

#### 9.7 Continuous Integration Integration

**Test Execution Order:**
1. Unit tests for core functions (path validation, MIME detection)
2. Integration tests for file serving logic
3. System tests for end-to-end HTTP behavior
4. Memory leak detection with valgrind/leaks
5. Performance regression testing

**Coverage Requirements:**
- All security validation paths must be tested
- All MIME type mappings must be verified
- Error conditions (file not found, permissions, etc.) must be covered
- Memory arena usage patterns must be validated

#### 9.8 Test Data Management

**setUp/tearDown Pattern:**
```c
void setUp(void) {
    setup_test_public_directory();
    create_test_file("./test_fixtures/public/style.css", "body { margin: 0; }");
    create_test_file("./test_fixtures/public/index.html", "<html><body>Test</body></html>");
    // ... create other test files
}

void tearDown(void) {
    cleanup_test_public_directory();
}
```

This comprehensive testing strategy ensures that static file serving is thoroughly validated across all security, functionality, and performance requirements while integrating seamlessly with the existing wp testing infrastructure.

### 10. Backward Compatibility

- No changes to existing wp file syntax
- Route matching behavior unchanged
- Only GET requests checked for static files
- Other HTTP methods bypass static file serving

### 11. Implementation Order

1. **Phase 1**: Core functions (validate_path, get_mime_type, serve_file)
2. **Phase 2**: Integration with find_and_process_route
3. **Phase 3**: Security hardening and edge case handling
4. **Phase 4**: Testing and documentation
5. **Phase 5**: Performance optimizations (future)

### 12. File Location

All static file serving code should be added to `src/server.c` to maintain architectural consistency and leverage existing response handling infrastructure.

## Expected Behavior

### Successful Cases
- `GET /styles.css` → serves `./public/styles.css` with `text/css`
- `GET /images/logo.png` → serves `./public/images/logo.png` with `image/png`
- `GET /` → serves `./public/index.html` with `text/html`

### Security Blocked Cases
- `GET /../etc/passwd` → 403 Forbidden
- `GET /.env` → 403 Forbidden  
- `GET /config.wp` → 403 Forbidden

### Fallback Cases
- `GET /api/users` → Falls back to route matching (existing behavior)
- `POST /upload` → Bypasses static serving, uses route matching

This implementation will provide secure, efficient static file serving while maintaining the existing wp pipeline functionality and security standards.