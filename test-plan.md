# Web Pipeline (wp) Runtime - E2E Test Plan

## Overview
This test plan outlines comprehensive end-to-end testing for the Web Pipeline (wp) runtime, including unit tests, integration tests, and system tests using the Unity testing framework.

## Test Categories

### 1. Unit Tests

#### 1.1 Memory Arena Tests
- **Arena Creation/Destruction**
  - Test arena_create() with various sizes
  - Test arena_free() properly releases memory
  - Test arena allocation boundary conditions
  - Test arena_alloc() with insufficient space
  - Test arena_strdup() and arena_strndup()

#### 1.2 Lexer Tests
- **Token Recognition**
  - HTTP methods (GET, POST, PUT, DELETE, PATCH)
  - Route patterns with parameters (/page/:id)
  - Pipeline operators (|>)
  - String literals with backticks
  - Identifiers and keywords
  - Braces, parentheses, colons, equals
  - Numbers and special characters

- **Edge Cases**
  - Empty input
  - Malformed strings
  - Nested quotes
  - Line/column tracking accuracy
  - Unicode characters

#### 1.3 Parser Tests
- **AST Generation**
  - Route definitions with pipelines
  - Variable assignments
  - Result steps with conditions
  - Nested pipeline structures
  - Error recovery and reporting

- **Pipeline Parsing**
  - Single step pipelines
  - Multi-step pipelines
  - Plugin parameters
  - Variable references

#### 1.4 Plugin Interface Tests
- **Plugin Loading**
  - Dynamic library loading (.so files)
  - Plugin registration
  - Plugin lookup by name
  - Error handling for missing plugins

- **Plugin Execution**
  - JSON input/output validation
  - Arena allocation within plugins
  - Plugin error handling
  - Memory leak detection

### 2. Integration Tests

#### 2.1 Plugin-Specific Tests

##### 2.1.1 JQ Plugin Tests
- **Basic Operations**
  - Simple field selection: `{ id: .params.id }`
  - Object construction and manipulation
  - Array operations
  - Conditional expressions
  - String manipulation functions

- **Error Handling**
  - Invalid JQ syntax
  - Missing fields
  - Type mismatches
  - Compilation caching verification

##### 2.1.2 Lua Plugin Tests
- **Basic Operations**
  - Simple return statements
  - Request object access
  - Table construction
  - String/number operations
  - Control flow structures

- **Error Handling**
  - Lua syntax errors
  - Runtime errors
  - Memory allocation failures
  - Request object corruption

##### 2.1.3 PostgreSQL Plugin Tests
- **Database Operations**
  - Simple SELECT queries
  - Parameterized queries ($1, $2, etc.)
  - INSERT/UPDATE/DELETE operations
  - Multiple result sets
  - Transaction handling

- **Error Handling**
  - Connection failures
  - SQL syntax errors
  - Parameter count mismatches
  - Table/column not found
  - Connection timeouts

#### 2.2 Pipeline Integration Tests
- **Multi-Plugin Pipelines**
  - jq → lua → pg chains
  - Data transformation between plugins
  - Error propagation through pipeline
  - Memory arena sharing

- **Result Step Processing**
  - Condition matching (ok, validationError, sqlError)
  - Status code handling
  - Default condition fallback
  - Nested result processing

### 3. HTTP Server Tests

#### 3.1 Request Handling
- **Route Matching**
  - Static routes (/test)
  - Parameterized routes (/page/:id)
  - Multiple parameters (/user/:id/post/:postId)
  - Route conflicts and precedence
  - Case sensitivity

- **HTTP Methods**
  - GET, POST, PUT, DELETE, PATCH
  - Method-specific routing
  - Invalid method handling

- **Request Processing**
  - Query parameter parsing
  - Request body handling (JSON, form-data)
  - Header extraction
  - URL parameter extraction

#### 3.2 Response Generation
- **Content Types**
  - JSON responses
  - Error responses
  - Custom headers
  - Status codes

- **Memory Management**
  - Per-request arena allocation
  - Arena cleanup after response
  - Memory leak detection
  - Concurrent request handling

### 4. System Tests

#### 4.1 End-to-End Scenarios
Based on test.wp file:

- **Simple Routes**
  - `/test` - Basic jq passthrough
  - `/test2` - Lua request return
  - `/test3` - Success/error result handling

- **Database Integration**
  - `/page/:id` - Parameter extraction and DB query
  - `/teams` - Variable query execution
  - `/test-sql-error` - SQL error handling

- **Error Scenarios**
  - `/test4` - Validation error handling
  - `/test5` - Authentication error handling
  - `/test6` - Unknown error type handling

#### 4.2 Performance Tests
- **Throughput**
  - Concurrent request handling
  - Memory arena reuse efficiency
  - Plugin execution performance
  - Database connection pooling

- **Memory Usage**
  - Arena allocation patterns
  - Memory growth under load
  - Garbage collection efficiency
  - Plugin memory isolation

#### 4.3 Reliability Tests
- **Error Recovery**
  - Plugin crashes
  - Database disconnections
  - Malformed requests
  - Resource exhaustion

- **Stability**
  - Long-running server stability
  - Memory leak detection
  - Resource cleanup verification
  - Graceful shutdown

## Test Implementation Structure

### Test Files Organization
```
test/
├── unity/              # Unity testing framework
├── unit/
│   ├── test_arena.c    # Memory arena tests
│   ├── test_lexer.c    # Lexer tests
│   ├── test_parser.c   # Parser tests
│   └── test_plugins.c  # Plugin interface tests
├── integration/
│   ├── test_jq.c       # JQ plugin tests
│   ├── test_lua.c      # Lua plugin tests
│   ├── test_pg.c       # PostgreSQL plugin tests
│   └── test_pipeline.c # Pipeline integration tests
├── system/
│   ├── test_server.c   # HTTP server tests
│   ├── test_e2e.c      # End-to-end scenarios
│   └── test_perf.c     # Performance tests
├── fixtures/
│   ├── test_routes.wp  # Test route definitions
│   ├── test_data.sql   # Test database schema/data
│   └── test_requests/  # Sample HTTP requests
└── helpers/
    ├── test_utils.c    # Common test utilities
    ├── mock_plugins.c  # Mock plugin implementations
    └── test_server.c   # Test server setup/teardown
```

### Test Data Requirements
- **Database Setup**
  - Test PostgreSQL instance
  - Sample tables (teams, users, etc.)
  - Test data fixtures
  - Migration scripts

- **Mock Services**
  - Mock HTTP endpoints
  - Fake database responses
  - Error simulation utilities

### Test Execution
- **Automated Test Runner**
  - Unity test runner integration
  - Makefile test targets
  - CI/CD pipeline integration
  - Test result reporting

- **Test Categories**
  - `make test-unit` - Unit tests only
  - `make test-integration` - Integration tests
  - `make test-system` - System tests
  - `make test-all` - Complete test suite
  - `make test-perf` - Performance tests

### Test Quality Metrics
- **Code Coverage**
  - Line coverage > 90%
  - Branch coverage > 85%
  - Function coverage > 95%

- **Test Completeness**
  - All plugins tested
  - All error conditions covered
  - All route patterns tested
  - All HTTP methods verified

This comprehensive test plan ensures the wp runtime is thoroughly tested across all components, from low-level memory management to high-level HTTP request processing, with particular attention to the plugin system and pipeline execution that form the core of the wp language runtime.