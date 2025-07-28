# Testing Framework Implementation Plan

## Overview
The testing-file.wp introduces a comprehensive testing DSL that needs lexer, parser, and server runtime integration. The testing framework will be coupled with the runtime to enable easy mock injection and test execution.

## New Language Constructs

### 1. Test Blocks
- `describe "description"` - Test suite grouping
- `it "description"` - Individual test case
- `with mock <middleware>.<variable> returning <json>` - Mock setup
- `when executing variable <type> <name>` - Execute standalone variable
- `when executing pipeline <name>` - Execute pipeline directly
- `when calling <method> <path>` - Make HTTP request
- `with input <json>` - Provide input data
- `then output equals <json>` - Assert output matches
- `then status is <number>` - Assert HTTP status
- `and <assertion>` - Chain additional assertions

## Lexer Updates (lexer.c)

### New Token Types
```c
typedef enum {
    // ... existing tokens ...
    TOKEN_DESCRIBE,
    TOKEN_IT,
    TOKEN_WITH,
    TOKEN_MOCK,
    TOKEN_RETURNING,
    TOKEN_WHEN,
    TOKEN_EXECUTING,
    TOKEN_VARIABLE,
    TOKEN_PIPELINE,
    TOKEN_CALLING,
    TOKEN_INPUT,
    TOKEN_THEN,
    TOKEN_OUTPUT,
    TOKEN_EQUALS,
    TOKEN_STATUS,
    TOKEN_IS,
    TOKEN_AND,
} token_type_t;
```

### Keywords Addition
- Add testing keywords to lexer keyword recognition
- Handle nested JSON literals within test assertions
- Support multi-line JSON formatting with proper tokenization

## Parser Updates (parser.c)

### New AST Node Types
```c
typedef enum {
    // ... existing nodes ...
    NODE_DESCRIBE_BLOCK,
    NODE_IT_BLOCK,
    NODE_MOCK_CONFIG,
    NODE_TEST_EXECUTION,
    NODE_TEST_ASSERTION,
} ast_node_type_t;

typedef struct {
    char *description;
    mock_config_t *mocks;
    test_block_t *tests;
} describe_block_t;

typedef struct {
    char *description;
    execution_config_t *execution;
    assertion_t *assertions;
} it_block_t;

typedef struct {
    char *middleware_name;
    char *variable_name;
    json_t *return_value;
} mock_config_t;
```

### Parsing Functions
- `parse_describe_block()` - Parse describe statements
- `parse_it_block()` - Parse individual test cases  
- `parse_mock_config()` - Parse mock configurations
- `parse_test_execution()` - Parse when/executing statements
- `parse_test_assertion()` - Parse then/and assertions

## Server Updates (server.c)

### Test Mode Detection and Execution
The existing `wp_runtime_init()` function needs modification to detect when `WP_MODE_TEST` is active and execute tests instead of starting the HTTP server.

```c
// In wp_runtime_init() - check for test mode
if (is_test_mode && has_test_blocks(ast)) {
    return execute_test_suite(ast);
} else {
    // Normal HTTP server startup
    return start_http_server(port);
}
```

### Test Execution Engine
```c
typedef struct {
    hash_table_t *mocks;           // middleware -> variable -> mock_data
    test_results_t *results;       // Test execution results
    arena_t *test_arena;          // Memory arena for test execution
} test_context_t;

// Core test execution functions
int execute_test_suite(ast_node_t *ast);
int execute_describe_block(describe_block_t *block, test_context_t *ctx);
int execute_it_block(it_block_t *block, test_context_t *ctx);
```

### Mock Integration System
```c
// Generic middleware mock registry - no hardcoded middleware names
typedef struct {
    char *middleware_name;     // Resolved at runtime from AST
    char *variable_name;       // Optional - for variable-specific mocks
    json_t *mock_data;         // Generic JSON response
    bool is_active;
} mock_entry_t;

// Generic mock injection - works with any middleware
json_t *execute_middleware_with_mocks(const char *middleware_name, 
                                     const char *config,
                                     json_t *input,
                                     test_context_t *test_ctx);

// Middleware lookup remains dynamic through existing registry
extern middleware_function_t get_middleware_function(const char *name);
```

### Test Mode Runtime
- Add `--test-mode` command line flag
- Modify middleware execution pipeline to check for mocks
- Implement test result collection and reporting
- Add test-specific memory management

## Runtime Coupling Strategy

**CRITICAL: Middleware Agnostic Design**
The testing framework must maintain strict separation from middleware implementations. No middleware names (like "pg", "jq", "lua") should be hardcoded anywhere in the runtime code. All middleware references must be dynamic and loaded at runtime.

### 1. Middleware Mock Injection
```c
// In middleware execution loop - completely middleware agnostic
if (test_context && has_mock(test_context, middleware_name, variable_name)) {
    result = get_mock_result(test_context, middleware_name, variable_name);
} else {
    result = middleware_execute(input, arena, alloc_func, free_func, 
                               config, middleware_config, contentType, variables);
}
```

### 2. Generic Mock System
```c
// Mock registry is completely generic - no middleware-specific logic
typedef struct {
    char *middleware_name;    // Dynamic - could be "pg", "jq", "custom", etc.
    char *variable_name;      // Dynamic - any variable name
    json_t *mock_data;        // Generic JSON response
    bool is_active;
} mock_entry_t;
```

### 3. Variable Execution Testing
- Direct execution of any middleware variables with mock responses
- Pipeline execution with middleware mocking at each step  
- Input/output validation with JSON comparison
- No hardcoded middleware types - all resolved dynamically

### 4. HTTP Request Testing
- Internal HTTP request simulation without network calls
- Route matching and pipeline execution in test context
- Response status and body validation

## Test Execution Flow

### 1. Parse Phase
```
testing-file.wp → Lexer → Parser → AST (routes + tests)
```

### 2. Test Discovery
```
AST → Extract describe/it blocks → Build test suite
```

### 3. Mock Setup
```
Test Context → Load mocks → Configure middleware overrides
```

### 4. Test Execution
```
For each test:
  - Setup mocks
  - Execute (variable/pipeline/route)
  - Collect results
  - Validate assertions
  - Report results
```

## Implementation Phases

### Phase 1: Lexer/Parser Extensions
- Add test-related tokens and keywords
- Implement test block parsing
- Extend AST to include test nodes

### Phase 2: Mock System
- Create mock registry and injection system
- Modify middleware execution to support mocks
- Add test context management

### Phase 3: Test Execution Engine
- Implement test runner
- Add assertion validation
- Build test result reporting

### Phase 4: Integration
- Modify `wp_runtime_init()` to detect `WP_MODE_TEST` and execute tests
- Add test discovery and execution in server.c
- Integrate with existing CLI infrastructure

## Trade-offs Analysis

### Benefits of Runtime Coupling
- **Direct Integration**: Tests run against actual runtime without separate test harness
- **Mock Flexibility**: Easy middleware mocking without complex dependency injection
- **Real Pipeline**: Tests execute actual pipeline logic with controlled inputs
- **Memory Management**: Uses same arena allocators as production

### Costs
- **Runtime Complexity**: Adds test-specific code to production runtime
- **Binary Size**: Increases executable size with test functionality
- **Maintenance**: Test and production code intermingled

### Mitigation Strategies
- Compile-time flags to exclude test code in production builds
- Clean separation of test context from production execution
- Comprehensive test coverage of mock injection system

## Integration Points

### Command Line Interface
```bash
./build/wp testing-file.wp --test
```

The existing `--test` flag is already implemented and sets `WP_MODE_TEST`. We need to modify the runtime to detect test blocks in the AST and execute them instead of starting the HTTP server when in test mode.

### Build System Updates
- Add test execution targets to Makefile
- Include test validation in CI pipeline
- Support for test-only builds

This plan provides a comprehensive testing framework that integrates directly with the runtime while maintaining clean separation of concerns through the test context system.