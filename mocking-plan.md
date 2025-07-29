# Mocking System Implementation Plan

## Overview
This document provides detailed implementation guidelines for integrating a comprehensive testing and mocking system into the Web Pipe runtime. The system will be middleware-agnostic and tightly coupled with the runtime for seamless test execution.

## Current Runtime Architecture Analysis

### Key Entry Points
- **Runtime Init**: `wp_runtime_init()` (server.c:1471) - Main initialization
- **Pipeline Execution**: `execute_pipeline_with_result()` (server.c:920) - Core pipeline runner
- **Middleware Execution**: `mw->execute()` (server.c:1010) - Perfect mock injection point
- **Middleware Loading**: `collect_middleware_names_from_ast()` (server.c:1310) - Already middleware-agnostic

### Current Runtime State Structure
```c
typedef struct {
    struct MHD_Daemon *daemon;
    ASTNode *program;
    Middleware *middleware;
    int middleware_count;
    json_t *variables;
    ParseContext *parse_ctx;
    ConfigBlock *config_blocks;
    int config_count;
} WPRuntime;
```

## Mock System Architecture

### 1. Test Context Structure
```c
typedef struct {
    hash_table_t *middleware_mocks;    // middleware_name -> mock_data
    hash_table_t *variable_mocks;      // middleware.variable -> mock_data  
    test_results_t *results;           // Test execution results
    arena_t *test_arena;              // Memory arena for test execution
    bool is_test_mode;                // Runtime test mode flag
} test_context_t;

// Thread-local test context
static _Thread_local test_context_t *current_test_context = NULL;
```

### 2. Mock Entry Structure
```c
typedef struct {
    char *middleware_name;     // e.g., "pg", "jq", "lua"
    char *variable_name;       // e.g., "teamsQuery" (optional)
    json_t *mock_data;         // Generic JSON response
    bool is_active;           // Enable/disable mock
    mock_type_t type;         // MIDDLEWARE_MOCK or VARIABLE_MOCK
} mock_entry_t;

typedef enum {
    MIDDLEWARE_MOCK,    // Mock entire middleware: "with mock pg returning {...}"
    VARIABLE_MOCK       // Mock specific variable: "with mock pg.teamsQuery returning {...}"
} mock_type_t;
```

### 3. Test Execution Phases

#### Phase 1: Test Discovery and AST Extension
**Location**: Extend `wp_runtime_init()` (server.c:1471)

```c
// Add after line 1538 (process_config_blocks)
if (has_test_blocks(runtime->program)) {
    if (is_test_mode_enabled()) {
        printf("Test mode detected, executing test suite...\n");
        int test_result = execute_test_suite(runtime->program);
        return test_result; // Skip HTTP server startup
    }
}
```

#### Phase 2: Mock Injection Point
**Location**: Modify `execute_pipeline_with_result()` (server.c:1010)

```c
// BEFORE: mw->execute(middleware_input, arena, ...)
json_t *result;

// Check for active mocks in test context
if (current_test_context && is_mock_active(current_test_context, mw->name, variable_name)) {
    result = get_mock_result(current_test_context, mw->name, variable_name);
    printf("Mock intercepted: %s%s%s\n", mw->name, 
           variable_name ? "." : "", variable_name ? variable_name : "");
} else {
    // Normal middleware execution
    result = mw->execute(middleware_input, arena,
                        arena_alloc_wrapper, arena_free_wrapper,
                        conf, mw_cfg, content_type, runtime->variables);
}
```

## Detailed Implementation Strategy

### ✅ 1. Lexer Extensions (lexer.c) - ALREADY IMPLEMENTED
The lexer has been extended with all required testing tokens:
- **Lines 124-156**: All testing keywords are implemented (`describe`, `it`, `with`, `mock`, `returning`, `when`, `executing`, `variable`, `calling`, `input`, `then`, `output`, `equals`, `status`, `is`, `and`)
- **Token Types**: Available in `wp.h` with proper enum values

### ✅ 2. Parser Extensions (parser.c) - ALREADY IMPLEMENTED  
The parser infrastructure is complete:

#### Existing AST Node Types (wp.h:97-102)
```c
AST_DESCRIBE_BLOCK,    // ✅ Implemented
AST_IT_BLOCK,          // ✅ Implemented  
AST_MOCK_CONFIG,       // ✅ Implemented
AST_TEST_EXECUTION,    // ✅ Implemented
AST_TEST_ASSERTION,    // ✅ Implemented
```

#### Existing AST Data Structures (wp.h:188-210)
```c
// ✅ Already defined
struct {
  char *description;
  ASTNode **mock_configs;  // Array of mock configurations
  int mock_count;
  ASTNode **tests;         // Array of it blocks  
  int test_count;
} describe_block;

struct {
  char *description;
  ASTNode *execution;      // Test execution config
  ASTNode **assertions;    // Array of assertions
  int assertion_count;
} it_block;

struct {
  char *middleware_name;   // e.g., "pg"
  char *variable_name;     // e.g., "teamsQuery" (optional)
  char *return_value;      // JSON string for mock response
} mock_config;
```

#### Existing Parsing Functions (parser.c:1087-1160)
```c
// ✅ All core parsing functions implemented
ASTNode *parser_parse_describe_block(Parser *parser);  // Lines 1087-1148
ASTNode *parser_parse_it_block(Parser *parser);        // Lines 1150+
ASTNode *parser_parse_mock_config(Parser *parser);     // Already implemented
ASTNode *parser_parse_test_execution(Parser *parser);  // Declared in wp.h
ASTNode *parser_parse_test_assertion(Parser *parser);  // Declared in wp.h
```

### 3. Runtime Integration Points

#### Test Mode Detection
```c
// Add to wp.h
extern bool is_test_mode_enabled(void);
extern void set_test_mode(bool enabled);

// Global test mode state
static bool global_test_mode = false;

bool is_test_mode_enabled(void) {
    return global_test_mode;
}

void set_test_mode(bool enabled) {
    global_test_mode = enabled;
}
```

#### Test Context Management
```c
// Thread-local test context management
void set_test_context(test_context_t *ctx) {
    current_test_context = ctx;
}

test_context_t *get_test_context(void) {
    return current_test_context;
}

test_context_t *create_test_context(MemoryArena *arena) {
    test_context_t *ctx = arena_alloc(arena, sizeof(test_context_t));
    ctx->middleware_mocks = create_hash_table(arena, 32);
    ctx->variable_mocks = create_hash_table(arena, 64);
    ctx->results = arena_alloc(arena, sizeof(test_results_t));
    ctx->test_arena = arena;
    ctx->is_test_mode = true;
    return ctx;
}
```

#### Mock Registry Implementation
```c
// Generic mock system - completely middleware agnostic
void register_mock(test_context_t *ctx, const char *middleware_name, 
                   const char *variable_name, json_t *mock_data) {
    char key[256];
    
    if (variable_name) {
        // Variable-specific mock: "pg.teamsQuery"
        snprintf(key, sizeof(key), "%s.%s", middleware_name, variable_name);
        hash_table_set(ctx->variable_mocks, key, mock_data);
    } else {
        // Middleware-wide mock: "pg"
        hash_table_set(ctx->middleware_mocks, middleware_name, mock_data);
    }
}

bool is_mock_active(test_context_t *ctx, const char *middleware_name, 
                    const char *variable_name) {
    if (!ctx) return false;
    
    char key[256];
    
    // Check for variable-specific mock first
    if (variable_name) {
        snprintf(key, sizeof(key), "%s.%s", middleware_name, variable_name);
        if (hash_table_get(ctx->variable_mocks, key)) {
            return true;
        }
    }
    
    // Check for middleware-wide mock
    return hash_table_get(ctx->middleware_mocks, middleware_name) != NULL;
}

json_t *get_mock_result(test_context_t *ctx, const char *middleware_name, 
                        const char *variable_name) {
    char key[256];
    
    // Try variable-specific mock first
    if (variable_name) {
        snprintf(key, sizeof(key), "%s.%s", middleware_name, variable_name);
        json_t *result = hash_table_get(ctx->variable_mocks, key);
        if (result) return result;
    }
    
    // Fall back to middleware-wide mock
    return hash_table_get(ctx->middleware_mocks, middleware_name);
}
```

### 4. Test Execution Engine

#### Main Test Runner
```c
int execute_test_suite(ASTNode *program) {
    printf("Executing test suite...\n");
    
    MemoryArena *test_arena = arena_create(1024 * 1024 * 10); // 10MB for tests
    test_context_t *test_ctx = create_test_context(test_arena);
    set_test_context(test_ctx);
    
    int total_tests = 0;
    int passed_tests = 0;
    
    // Find and execute all describe blocks
    for (int i = 0; i < program->data.program.statement_count; i++) {
        ASTNode *stmt = program->data.program.statements[i];
        if (stmt->type == AST_DESCRIBE_BLOCK) {
            int suite_total, suite_passed;
            execute_describe_block(stmt, test_ctx, &suite_total, &suite_passed);
            total_tests += suite_total;
            passed_tests += suite_passed;
        }
    }
    
    // Print results
    printf("\nTest Results: %d/%d passed\n", passed_tests, total_tests);
    
    arena_free(test_arena);
    return (passed_tests == total_tests) ? 0 : 1;
}

int execute_describe_block(ASTNode *describe_node, test_context_t *ctx, 
                          int *total, int *passed) {
    describe_block_t *block = &describe_node->data.describe_block;
    printf("\n%s\n", block->description);
    
    *total = 0;
    *passed = 0;
    
    // Set up describe-level mocks
    apply_mocks(ctx, block->mocks);
    
    // Execute each test case
    it_block_t *test = block->tests;
    while (test) {
        (*total)++;
        if (execute_it_block(test, ctx)) {
            (*passed)++;
            printf("  ✓ %s\n", test->description);
        } else {
            printf("  ✗ %s\n", test->description);
        }
        test = test->next;
    }
    
    return 0;
}
```

#### Test Case Execution
```c
bool execute_it_block(it_block_t *test, test_context_t *ctx) {
    // Apply test-specific mocks (in addition to describe-level mocks)
    apply_mocks(ctx, test->local_mocks);
    
    // Execute the test
    json_t *result = NULL;
    int status_code = 200;
    
    switch (test->execution->type) {
        case EXEC_VARIABLE:
            result = execute_variable_test(test->execution, ctx);
            break;
        case EXEC_PIPELINE:
            result = execute_pipeline_test(test->execution, ctx);
            break;
        case EXEC_ROUTE:
            result = execute_route_test(test->execution, ctx, &status_code);
            break;
    }
    
    // Validate assertions
    return validate_assertions(test->assertions, result, status_code);
}

json_t *execute_variable_test(execution_config_t *exec, test_context_t *ctx) {
    // "when executing variable pg teamsQuery"
    // Find the variable in runtime->variables
    json_t *variable = json_object_get(runtime->variables, exec->target_name);
    if (!variable) {
        fprintf(stderr, "Variable not found: %s\n", exec->target_name);
        return NULL;
    }
    
    // Execute the variable as if it were a single-step pipeline
    // This will go through the normal middleware execution path
    // and trigger mock interception if configured
    
    char *middleware_name = extract_middleware_from_variable_name(exec->target_name);
    Middleware *mw = find_middleware(middleware_name);
    
    if (!mw) {
        fprintf(stderr, "Middleware not found for variable: %s\n", exec->target_name);
        return NULL;
    }
    
    // Create test input
    json_t *input = exec->input_data ? exec->input_data : json_object();
    
    // Execute with mock interception
    json_t *result;
    if (is_mock_active(ctx, middleware_name, exec->target_name)) {
        result = get_mock_result(ctx, middleware_name, exec->target_name);
    } else {
        result = mw->execute(input, ctx->test_arena, arena_alloc_wrapper,
                           arena_free_wrapper, json_string_value(variable),
                           get_middleware_config(mw->name), NULL, runtime->variables);
    }
    
    return result;
}

json_t *execute_pipeline_test(execution_config_t *exec, test_context_t *ctx) {
    // "when executing pipeline getTeams"
    // Find pipeline definition in runtime->variables
    json_t *pipeline_var = json_object_get(runtime->variables, exec->target_name);
    if (!pipeline_var) {
        fprintf(stderr, "Pipeline not found: %s\n", exec->target_name);
        return NULL;
    }
    
    // Verify it's a pipeline definition
    json_t *type_field = json_object_get(pipeline_var, "_type");
    if (!json_is_string(type_field) || strcmp(json_string_value(type_field), "pipeline") != 0) {
        fprintf(stderr, "Variable '%s' is not a pipeline definition\n", exec->target_name);
        return NULL;
    }
    
    // Get pipeline AST node
    ASTNode *pnode = (ASTNode *)(uintptr_t)json_integer_value(json_object_get(pipeline_var, "_definition"));
    if (!pnode || pnode->type != AST_PIPELINE_DEFINITION) {
        fprintf(stderr, "Corrupted pipeline definition for '%s'\n", exec->target_name);
        return NULL;
    }
    
    // Create test input
    json_t *input = exec->input_data ? exec->input_data : json_object();
    
    // Execute pipeline with mock interception
    json_t *result = NULL;
    int response_code;
    char *content_type;
    int ok = execute_pipeline_with_result(pnode->data.pipeline_def.pipeline, input,
                                         ctx->test_arena, &result, &response_code, &content_type);
    
    return (ok == 0) ? result : NULL;
}

json_t *execute_route_test(execution_config_t *exec, test_context_t *ctx, int *status_code) {
    // "when calling GET /hello"
    // Parse method and path from exec->target_name and exec->target_path
    const char *method = exec->target_name;  // "GET", "POST", etc.
    const char *url = exec->target_path;     // "/hello", "/users/:id", etc.
    
    // Create synthetic request object (no real HTTP connection needed)
    json_t *request = create_test_request_json(method, url, exec->input_data);
    
    // Set test arena context
    set_current_arena(ctx->test_arena);
    
    // Find matching route using existing server.c logic
    for (int i = 0; i < runtime->program->data.program.statement_count; i++) {
        ASTNode *stmt = runtime->program->data.program.statements[i];
        if (stmt->type == AST_ROUTE_DEFINITION) {
            if (strcmp(stmt->data.route_def.method, method) == 0) {
                json_t *params = json_object_get(request, "params");
                // Reuse existing match_route() function (server.c:1116)
                if (match_route(stmt->data.route_def.route, url, params)) {
                    // Execute route pipeline directly (bypassing HTTP layer)
                    return execute_route_pipeline(stmt, request, ctx->test_arena, status_code);
                }
            }
        }
    }
    
    // Route not found
    *status_code = 404;
    return create_error_json("Route not found");
}

// Helper function to create test request JSON (bypasses HTTP)
json_t *create_test_request_json(const char *method, const char *url, json_t *test_input) {
    json_t *request = json_object();
    
    // Basic HTTP request structure
    json_object_set_new(request, "method", json_string(method));
    json_object_set_new(request, "url", json_string(url));
    json_object_set_new(request, "params", json_object());  // Populated by match_route()
    json_object_set_new(request, "query", json_object());
    json_object_set_new(request, "headers", json_object());
    json_object_set_new(request, "cookies", json_object());
    json_object_set_new(request, "setCookies", json_array());
    
    // Use test input as body if provided, otherwise null
    if (test_input) {
        json_object_set(request, "body", test_input);
    } else {
        json_object_set_new(request, "body", json_null());
    }
    
    return request;
}

// Helper function to execute route pipeline (bypasses HTTP response handling)
json_t *execute_route_pipeline(ASTNode *route_stmt, json_t *request, 
                              MemoryArena *arena, int *status_code) {
    // If pipeline is empty, return the request object
    if (!route_stmt->data.route_def.pipeline) {
        *status_code = 200;
        return request;
    }
    
    // Execute pipeline with result handling (reuses server.c:762 logic)
    json_t *final_response = NULL;
    int response_code = 200;
    char *content_type = NULL;
    
    int result = execute_pipeline_with_result(route_stmt->data.route_def.pipeline, 
                                            request, arena, &final_response, 
                                            &response_code, &content_type);
    
    if (result == 0 && final_response) {
        *status_code = response_code;
        return final_response;
    } else {
        // Error in pipeline execution
        *status_code = 500;
        return create_error_json("Internal server error");
    }
}

json_t *create_error_json(const char *message) {
    json_t *error = json_object();
    json_object_set_new(error, "error", json_string(message));
    return error;
}
```

### 5. Route Testing Strategy - Key Insights

#### Why Route Testing Isn't "Tricky" 
The existing `server.c` architecture provides clean separation between route logic and HTTP handling, making route testing straightforward:

**Reusable Components:**
- **`match_route()`** (server.c:1116) - Pattern matching & parameter extraction
- **`execute_pipeline_with_result()`** (server.c:920) - Pipeline execution with mock injection
- **Route discovery loop** (server.c:781-791) - Finds matching route definitions

**Test Execution Flow:**
```
when calling GET /hello
    ↓
create_test_request_json()  // Synthetic HTTP request (no network)
    ↓  
match_route()              // Reuse existing route matching logic
    ↓
execute_route_pipeline()   // Direct pipeline execution (bypasses HTTP layer)
    ↓
Mock injection occurs      // At middleware execution (server.c:1010)
    ↓
Return JSON + status code  // For test assertions
```

**Key Benefits:**
- ✅ **No HTTP server needed**: Direct function calls bypass network stack
- ✅ **Parameter extraction works**: `match_route()` populates URL parameters (`/users/:id` → `{"id": "42"}`)
- ✅ **Mock injection intact**: Same middleware execution path as production
- ✅ **Status codes supported**: Pipeline can set response codes via result steps
- ✅ **Clean separation**: Test logic vs. production HTTP handling

**Mock Integration Point:**
Route tests benefit from the same mock injection at `server.c:1010` as pipeline/variable tests:
```c
// In execute_pipeline_with_result() - line 1010
if (current_test_context && is_mock_active(current_test_context, mw->name, variable_name)) {
    result = get_mock_result(current_test_context, mw->name, variable_name);
    // Mock intercepted - works for route tests too!
} else {
    result = mw->execute(middleware_input, arena, ...);  // Normal execution
}
```

### 6. Integration with Existing CLI

#### Command Line Integration
The existing `--test` flag in the CLI already sets test mode. We need to modify `wp_runtime_init()` to detect test blocks and execute them instead of starting the HTTP server.

```c
// In wp_runtime_init() after parsing (around line 1538)
bool has_tests = has_test_blocks(runtime->program);
bool test_mode = is_test_mode_enabled(); // This checks the --test flag

if (test_mode && has_tests) {
    printf("Test mode enabled with test blocks found, executing tests...\n");
    return execute_test_suite(runtime->program);
} else if (test_mode && !has_tests) {
    printf("Test mode enabled but no test blocks found in %s\n", wp_file);
    return -1;
} else {
    // Normal HTTP server mode
    printf("Starting HTTP server on port %d...\n", port);
    // ... existing server startup code ...
}
```

### 6. Hash Table Implementation

Since the runtime doesn't have a hash table implementation, we need a simple one for the mock registry:

```c
typedef struct hash_entry {
    char *key;
    void *value;
    struct hash_entry *next;
} hash_entry_t;

typedef struct {
    hash_entry_t **buckets;
    int bucket_count;
    MemoryArena *arena;
} hash_table_t;

hash_table_t *create_hash_table(MemoryArena *arena, int bucket_count) {
    hash_table_t *table = arena_alloc(arena, sizeof(hash_table_t));
    table->buckets = arena_alloc(arena, sizeof(hash_entry_t*) * bucket_count);
    table->bucket_count = bucket_count;
    table->arena = arena;
    
    // Initialize buckets to NULL
    for (int i = 0; i < bucket_count; i++) {
        table->buckets[i] = NULL;
    }
    
    return table;
}

// Simple hash function
unsigned int hash_string(const char *str, int bucket_count) {
    unsigned int hash = 5381;
    int c;
    while ((c = *str++)) {
        hash = ((hash << 5) + hash) + c;
    }
    return hash % bucket_count;
}

void hash_table_set(hash_table_t *table, const char *key, void *value) {
    unsigned int bucket = hash_string(key, table->bucket_count);
    
    hash_entry_t *entry = arena_alloc(table->arena, sizeof(hash_entry_t));
    entry->key = arena_strdup(table->arena, key);
    entry->value = value;
    entry->next = table->buckets[bucket];
    table->buckets[bucket] = entry;
}

void *hash_table_get(hash_table_t *table, const char *key) {
    unsigned int bucket = hash_string(key, table->bucket_count);
    
    hash_entry_t *entry = table->buckets[bucket];
    while (entry) {
        if (strcmp(entry->key, key) == 0) {
            return entry->value;
        }
        entry = entry->next;
    }
    
    return NULL;
}
```

## Implementation Phases

### ✅ Phase 1: Foundation - COMPLETED
1. ✅ **Lexer tokens**: All testing tokens implemented (lexer.c:124-156)
2. ✅ **AST nodes**: Test block structures defined (wp.h:97-102, 188-210)  
3. ⚠️ **Hash table**: Need simple implementation for mock registry
4. ⚠️ **Test context**: Need thread-local test context management

### ✅ Phase 2: Parsing - MOSTLY COMPLETED  
1. ✅ **Core parsing**: `parse_describe_block()` and `parse_it_block()` implemented (parser.c:1087+)
2. ✅ **Mock parsing**: `parse_mock_config()` function exists
3. ⚠️ **Test execution parsing**: `parse_test_execution()` declared but needs implementation
4. ⚠️ **Assertion parsing**: `parse_test_assertion()` declared but needs implementation
5. ✅ **Program integration**: Test blocks parsed in `parse_statement()` (parser.c:703)

### 🔲 Phase 3: Runtime Integration - TO DO
1. 🔲 **Test mode detection**: Modify `wp_runtime_init()` to detect test blocks and execute tests
2. 🔲 **Mock injection**: Add mock interception to `execute_pipeline_with_result()` (server.c:1010)
3. 🔲 **Test execution engine**: Implement `execute_test_suite()` and related functions
4. 🔲 **Test types**: Implement variable, pipeline, and route test execution

### 🔲 Phase 4: Completion - TO DO
1. 🔲 **Error handling**: Comprehensive test error collection and reporting
2. 🔲 **Hash table**: Simple arena-based hash table for mock registry
3. 🔲 **Integration testing**: Test with existing middleware
4. 🔲 **Documentation**: Update build system and CLI documentation

## Current Status: ~60% Complete
**What's Done**: Lexing, parsing, AST structures  
**What's Left**: Runtime integration, test execution engine, mock system

### ⚡ Next Steps - Runtime Integration Priority
1. **Immediate**: Implement hash table for mock registry (server.c)
2. **Critical**: Add test mode detection to `wp_runtime_init()` (server.c:1471)
3. **Core**: Implement mock injection at `execute_pipeline_with_result()` (server.c:1010)
4. **Essential**: Build test execution engine (`execute_test_suite()`, `execute_describe_block()`, `execute_it_block()`)

### 🔍 Verification Needed
To determine exact parsing completeness, check:
- `parser_parse_test_execution()` implementation status
- `parser_parse_test_assertion()` implementation status  
- Whether `when executing`, `then output equals`, etc. are fully parsed
- Mock parsing handles both `with mock pg returning` and `with mock pg.variable returning`

## Memory Management Strategy

The mocking system will use the existing arena allocator pattern:
- **Test Suite Arena**: 10MB arena for entire test execution
- **Per-Test Arena**: Reuse test suite arena with cleanup between tests
- **Mock Data**: Stored in test arena, automatically freed at test completion
- **Hash Tables**: Arena-allocated, no manual cleanup required

## Error Handling Strategy

```c
typedef struct {
    char *test_name;
    char *error_message;
    error_type_t type;
} test_error_t;

typedef enum {
    TEST_ERROR_MOCK_NOT_FOUND,
    TEST_ERROR_ASSERTION_FAILED,
    TEST_ERROR_EXECUTION_FAILED,
    TEST_ERROR_PARSE_ERROR
} error_type_t;
```

All test errors will be collected and reported at the end of the test run, similar to modern test frameworks.

## Compatibility Considerations

- **Existing Code**: No changes to existing middleware interface
- **Production Builds**: Test code can be conditionally compiled with `#ifdef WP_ENABLE_TESTING`
- **Memory Overhead**: Test code only loaded in test mode
- **Performance**: Zero runtime overhead in production mode

This implementation provides a robust, middleware-agnostic testing framework that integrates seamlessly with the existing Web Pipe runtime while maintaining clean separation between production and test code.