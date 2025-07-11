# Web Pipeline (wp)

A high-performance web server runtime that processes HTTP requests through declarative pipeline configurations. Built in C with libmicrohttpd and JSON data flow between pipeline steps.

## Overview

Web Pipeline (wp) is a domain-specific language and runtime for building web APIs through pipeline-based request processing. Each HTTP request flows through a series of plugins that transform JSON data, enabling powerful composition of data processing steps.

## Architecture

The system consists of:
- **Lexer/Parser**: Parses `.wp` files into an Abstract Syntax Tree (AST)
- **Runtime**: HTTP server built on libmicrohttpd that executes pipeline steps
- **Plugin System**: Dynamically loaded `.so` files that process JSON data
- **Memory Management**: Per-request bump allocator arenas for efficient memory usage

## Pipeline Processing

Each HTTP request follows this flow:

1. **Request Creation**: Incoming HTTP request is converted to JSON with `query`, `body`, `params`, `headers`, and other request metadata
2. **Pipeline Execution**: Request JSON flows through each pipeline step sequentially
3. **Plugin Processing**: Each plugin receives JSON input and returns JSON output
4. **Response Generation**: Final JSON is converted to HTTP response

### Data Flow Example

```
HTTP Request → JSON Request Object → Plugin 1 → Plugin 2 → Plugin N → HTTP Response
```

The request object is maintained throughout the pipeline, with each step potentially modifying or augmenting the data.

## Language Syntax

### Basic Route Definition

```wp
GET /page/:id
  |> jq: `{ id: .params.id }`
  |> lua: `return { sqlParams: { request.id } }`
  |> pg: `select * from items where id = $1`
```

### Variable Assignments

```wp
pg articlesQuery = `select * from articles`

GET /articles
  |> pg: articlesQuery
```

### Error Handling with Result Steps

```wp
GET /api/data
  |> jq: `{ message: "Hello World" }`
  |> result
    ok(200):
      |> jq: `{
        success: true,
        data: .message,
        timestamp: now
      }`
    validationError(400):
      |> jq: `{
        error: "Validation failed",
        field: .errors[0].field
      }`
    default(500):
      |> jq: `{
        error: "Internal server error"
      }`
```

## Built-in Plugins

### JQ Plugin
Processes JSON using jq expressions for data transformation and filtering.

```wp
GET /transform
  |> jq: `{ message: "Hello, " + .params.name }`
```

### Lua Plugin
Executes Lua scripts with full access to the request object.

```wp
GET /process
  |> lua: `
    return {
      sqlParams = { request.body.name, request.body.email }
    }
  `
```

### PostgreSQL Plugin
Executes SQL queries with parameter binding from `sqlParams`.

```wp
GET /users/:id
  |> jq: `{ sqlParams: [.params.id] }`
  |> pg: `SELECT * FROM users WHERE id = $1`
```

## Building and Installation

### Prerequisites

#### Ubuntu/Linux
```bash
sudo apt-get install -y \
  clang \
  libmicrohttpd-dev \
  libpq-dev \
  libjansson-dev \
  libjq-dev \
  liblua5.4-dev \
  postgresql-client \
  valgrind
```

#### macOS
```bash
brew install \
  llvm \
  postgresql@14 \
  libmicrohttpd \
  jansson \
  jq \
  lua
```

### Build Commands

```bash
# Build main executable and plugins
make all

# Build debug version
make debug

# Build and install plugins
make install-plugins

# Run server
make run
```

### Testing

The project uses Unity testing framework with comprehensive test coverage:

```bash
# Run all tests
make test

# Run specific test suites
make test-unit       # Unit tests for core components
make test-integration # Integration tests for plugins  
make test-system     # System/end-to-end tests

# Run performance tests
make test-perf

# Run memory leak detection
make test-leaks

# Run static analysis
make test-analyze

# Run code linting
make test-lint
```

## Usage

### Starting the Server

```bash
# Run with a .wp file
./build/wp routes.wp

# Server starts on default port with routes loaded
```

### Example .wp File

```wp
# Variable assignment
pg getUserQuery = `SELECT * FROM users WHERE id = $1`

# Simple route
GET /health
  |> jq: `{ status: "ok", timestamp: now }`

# Parameterized route with database query
GET /users/:id
  |> jq: `{ sqlParams: [.params.id | tostring] }`
  |> pg: getUserQuery
  |> jq: `{ user: .data.rows[0] }`

# POST with validation and error handling
POST /users
  |> jq: `{ 
    name: .body.name,
    email: .body.email
  }`
  |> lua: `
    if not request.name or not request.email then
      return { errors = {{ type = "validationError", message = "Missing required fields" }} }
    end
    return { sqlParams = { request.name, request.email } }
  `
  |> pg: `INSERT INTO users (name, email) VALUES ($1, $2) RETURNING *`
  |> result
    ok(201):
      |> jq: `{
        success: true,
        user: .data.rows[0]
      }`
    validationError(400):
      |> jq: `{
        error: "Invalid input",
        message: .errors[0].message
      }`
    default(500):
      |> jq: `{ error: "Internal server error" }`
```

## Plugin Development

Plugins are shared libraries that implement the plugin interface:

```c
typedef struct {
    char *name;
    void *handle;
    json_t *(*execute)(json_t *input, void *arena, 
                      arena_alloc_func alloc_func, 
                      arena_free_func free_func, 
                      const char *config);
} Plugin;
```

Each plugin receives:
- `input`: JSON data from previous pipeline step
- `arena`: Memory arena for allocations
- `alloc_func`/`free_func`: Arena allocation functions
- `config`: Plugin configuration string

## Memory Management

The runtime uses per-request memory arenas for efficient allocation and cleanup:

- **Parse Arena**: For AST nodes and parser data structures
- **Runtime Arena**: For long-lived runtime data
- **Request Arena**: Per-request allocations, freed after response

This approach eliminates memory leaks and provides predictable performance characteristics.

## Performance Features

- **Compiled Pipelines**: Routes are parsed once at startup
- **Plugin Caching**: Compiled jq programs and Lua scripts are cached
- **Arena Allocation**: Fast bump allocation with automatic cleanup
- **Minimal Copying**: JSON data flows through pipeline with minimal serialization

## Error Handling

The system provides structured error handling through the `result` step:

- **Condition Matching**: Errors are matched against specific condition types
- **Status Codes**: Each condition specifies HTTP status codes
- **Error Propagation**: Errors flow through the pipeline like regular data
- **Fallback Handling**: `default` condition handles unmatched errors

## Continuous Integration

The project uses GitHub Actions for automated testing across platforms:

- **Ubuntu Latest**: Full test suite with PostgreSQL service
- **macOS Latest**: Native macOS builds and testing
- **Test Coverage**: Unit, integration, system, and leak detection tests
- **Static Analysis**: Clang analyzer and linting with warnings as errors

See `.github/workflows/test.yml` for the complete CI configuration.

## Contributing

1. Fork the repository
2. Create a feature branch
3. Write tests for new functionality
4. Ensure all tests pass with `make test`
5. Verify static analysis passes with `make test-analyze`
6. Check for memory leaks with `make test-leaks`
7. Submit a pull request

## License

This project is licensed under the MIT License.