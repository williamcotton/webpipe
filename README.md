![Test Suite](https://github.com/williamcotton/webpipe/workflows/Test%20Suite/badge.svg)

# Web Pipe (wp)

A high-performance web server runtime that processes HTTP requests through declarative pipeline configurations. Built in C with libmicrohttpd and JSON data flow between pipeline steps.

```wp
GET /hello
  |> jq: `{ world: ":)"}`
```

## Overview

Web Pipe (wp) is a DSL and runtime for building web APIs through pipeline-based request processing. Each HTTP request flows through a series of middleware that transform JSON data, enabling powerful composition of data processing steps.

## Architecture

The system consists of:
- **Lexer/Parser**: Parses `.wp` files into an Abstract Syntax Tree (AST)
- **Runtime**: HTTP server built on libmicrohttpd that executes pipeline steps
- **Middleware System**: Dynamically loaded `.so` files that process JSON data
- **Memory Management**: Per-request bump allocator arenas for efficient memory usage

## Pipeline Processing

Each HTTP request follows this flow:

1. **Request Creation**: Incoming HTTP request is converted to JSON with `query`, `body`, `params`, `headers`, and other request metadata
2. **Pipeline Execution**: Request JSON flows through each pipeline step sequentially
3. **Middleware Processing**: Each middleware receives JSON input and returns JSON output
4. **Response Generation**: Final JSON is converted to HTTP response

### Data Flow Example

```
HTTP Request → JSON Request Object → Middleware 1 → Middleware 2 → Middleware N → HTTP Response
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

## Built-in Middleware

### JQ Middleware
Processes JSON using jq expressions for data transformation and filtering.

```wp
GET /transform
  |> jq: `{ message: "Hello, " + .params.name }`
```

### Lua Middleware
Executes Lua scripts with full access to the request object.

```wp
GET /process
  |> lua: `
    return {
      sqlParams = { request.body.name, request.body.email }
    }
  `
```

### PostgreSQL Middleware
Executes SQL queries with parameter binding from `sqlParams`.

```wp
GET /users/:id
  |> jq: `{ sqlParams: [.params.id] }`
  |> pg: `SELECT * FROM users WHERE id = $1`
```

### Validate Middleware
Validates request body fields using a simple DSL with built-in validation rules.

```wp
POST /api/users
  |> validate: `
    name: string(3..50)
    email: email
    age?: number(18..120)
    team_id?: number
  `
  |> jq: `{ sqlParams: [.body.name, .body.email, .body.age] }`
  |> pg: `INSERT INTO users (name, email, age) VALUES ($1, $2, $3) RETURNING *`
  |> result
    ok(201):
      |> jq: `{ success: true, user: .data.rows[0] }`
    validationError(400):
      |> jq: `{
        error: "Validation failed",
        field: .errors[0].field,
        rule: .errors[0].rule,
        message: .errors[0].message
      }`
```

**Validation Rules:**
- `string(min..max)` - String with length constraints
- `string` - String without constraints  
- `number(min..max)` - Number with range constraints
- `number` - Number without constraints
- `email` - Valid email format validation
- `boolean` - Boolean value validation
- `field?: type` - Optional field (with `?` suffix)

**Error Format:**
Returns standardized validation errors when constraints are violated:
```json
{
  "errors": [
    {
      "type": "validationError",
      "field": "name",
      "rule": "minLength", 
      "message": "String must be at least 3 characters long"
    }
  ]
}
```

### Mustache Middleware
Renders HTML templates using mustache syntax with JSON data.

```wp
GET /hello-mustache
  |> jq: `{ name: "World", message: "Hello from mustache!" }`
  |> mustache: `
    <html>
      <head>
        <title>{{message}}</title>
      </head>
      <body>
        <h1>{{message}}</h1>
        <p>Hello, {{name}}!</p>
      </body>
    </html>
  `
```

#### Mustache Partials

The mustache middleware supports partials for reusable template components. Partials are defined as variables and can be included in other templates using the `{{>partialName}}` syntax.

**Defining Partials:**

```wp
# Define reusable template components
mustache cardPartial = `
  <div class="card">
    <h3>{{title}}</h3>
    <p>{{description}}</p>
  </div>
`

mustache headerPartial = `
  <header>
    <h1>{{siteName}}</h1>
    <nav>{{>navPartial}}</nav>
  </header>
`

mustache navPartial = `
  <ul>
    <li><a href="/">Home</a></li>
    <li><a href="/about">About</a></li>
  </ul>
`
```

**Using Partials in Templates:**

```wp
GET /test-partials
  |> jq: `{ 
    title: "Welcome", 
    description: "This is a test card",
    siteName: "My Website"
  }`
  |> mustache: `
    <html>
      <head>
        <title>{{siteName}}</title>
      </head>
      <body>
        {{>headerPartial}}
        <main>
          {{>cardPartial}}
        </main>
      </body>
    </html>
  `
```

**Features of Mustache Partials:**
- **Reusability**: Define once, use multiple times across different templates
- **Nesting**: Partials can include other partials (e.g., `headerPartial` includes `navPartial`)
- **Context Sharing**: Partials have access to the same JSON data context as the main template
- **Error Handling**: Missing partials are handled gracefully (rendered as empty strings)
- **Variable Scope**: All mustache variables defined in your `.wp` file are available as partials

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
  libcurl4-openssl-dev \
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
# Build main executable and middleware
make all

# Build debug version
make debug

# Build and install middleware
make install-middleware

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
make test-integration # Integration tests for middleware  
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

## Content-Type Support

The runtime supports multiple content types beyond the default `application/json`. Middleware can set the response content type by modifying the `contentType` parameter.

### Supported Content Types

- **application/json** (default): Standard JSON responses
- **text/html**: HTML responses for web pages
- **text/plain**: Plain text responses
- **text/css**: CSS stylesheets
- **text/javascript**: JavaScript code
- **Other text/***: Any text-based content type

### Mustache Middleware

The runtime includes a mustache middleware for HTML template rendering. This middleware processes JSON data with mustache templates to generate HTML responses.

```wp
GET /hello-mustache
  |> jq: `{ name: "World", message: "Hello from mustache!" }`
  |> mustache: `
    <html>
      <head>
        <title>{{message}}</title>
      </head>
      <body>
        <h1>{{message}}</h1>
        <p>Hello, {{name}}!</p>
      </body>
    </html>
  `
```

**Features:**
- Variable substitution with `{{variable}}`
- Conditional sections with `{{#condition}}...{{/condition}}`
- Array iteration with `{{#array}}...{{/array}}`
- Missing variable handling (empty string substitution)
- Error handling for malformed templates
- Automatic content-type setting to `text/html`

**Template Syntax Examples:**
```mustache
<!-- Basic variable substitution -->
<h1>{{title}}</h1>

<!-- Conditional rendering -->
{{#showMessage}}
<p>{{message}}</p>
{{/showMessage}}

<!-- Array iteration -->
<ul>
{{#items}}
  <li>{{.}}</li>
{{/items}}
</ul>
```

### Setting Content Type in Middleware

Middleware can set the content type by assigning to the `contentType` parameter:

```c
json_t *middleware_execute(json_t *input, void *arena, 
                          arena_alloc_func alloc_func, 
                          arena_free_func free_func, 
                          const char *config,
                          char **contentType) {
    // Set content type to HTML
    *contentType = arena_strdup(arena, "text/html");
    
    // Return HTML content as a JSON string
    return json_string("<html><body><h1>Hello World</h1></body></html>");
}
```

### Response Handling

The server automatically handles different content types:
- **JSON responses**: Content is serialized as JSON
- **HTML/Text responses**: JSON string values are extracted and sent as-is
- **Fallback**: Non-string JSON is serialized as JSON with `application/json` content type

This enables seamless support for web pages, APIs, and other content types in the same pipeline framework.


## Middleware Development

Middleware are shared libraries that implement the middleware interface:

```c
typedef struct {
    char *name;
    void *handle;
    json_t *(*execute)(json_t *input, void *arena, 
                      arena_alloc_func alloc_func, 
                      arena_free_func free_func, 
                      const char *config,
                      char **contentType);
} Middleware;
```

Each middleware receives:
- `input`: JSON data from previous pipeline step
- `arena`: Memory arena for allocations
- `alloc_func`/`free_func`: Arena allocation functions
- `config`: Middleware configuration string
- `contentType`: Pointer to content type string (can be modified)

### Automatic Memory Management with Jansson

The runtime automatically configures jansson to use per-request arena allocators at startup by calling `json_set_alloc_funcs()` in `server.c`. This means that **middleware developers can use jansson JSON objects normally** without worrying about memory management - all JSON allocations will automatically use the per-request arena allocator.

When creating, manipulating, or returning `json_t` objects in middleware:
- Use standard jansson functions (`json_object()`, `json_array()`, `json_string()`, etc.)
- No need to manually manage memory for JSON objects
- All JSON memory is automatically freed when the request completes
- The arena ensures no memory leaks even if middleware don't explicitly decref objects

This seamless integration allows middleware to focus on business logic rather than memory management details.

### Hello World Middleware Example

Here's a complete example of a simple middleware that adds a `hello` key to the JSON object:

```c
#include <jansson.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

// Arena allocation function types for middleware
typedef void* (*arena_alloc_func)(void* arena, size_t size);
typedef void (*arena_free_func)(void* arena);

// Memory arena type (forward declaration)
typedef struct MemoryArena MemoryArena;

// Middleware interface function
json_t *middleware_execute(json_t *input, void *arena, arena_alloc_func alloc_func, arena_free_func free_func, const char *config, char **contentType) {
    // Suppress unused parameter warnings
    (void)free_func;
    (void)contentType;  // This middleware doesn't change content type
    
    // Handle null input - create empty object
    if (!input) {
        input = json_object();
    }
    
    // Handle null or empty config - use "world" as default
    const char *value = "world";
    if (config && strlen(config) > 0) {
        value = config;
    }
    
    // Demonstrate arena usage by copying the config value
    char *arena_value = NULL;
    if (alloc_func && arena) {
        size_t len = strlen(value);
        arena_value = alloc_func(arena, len + 1);
        if (arena_value) {
            memcpy(arena_value, value, len);
            arena_value[len] = '\0';
            value = arena_value;
        }
    }
    
    // Clone the input to avoid modifying the original
    json_t *result = json_deep_copy(input);
    if (!result) {
        // Return standardized error format
        json_t *error_obj = json_object();
        json_t *errors_array = json_array();
        json_t *error_detail = json_object();
        
        json_object_set_new(error_detail, "type", json_string("internalError"));
        json_object_set_new(error_detail, "message", json_string("Failed to copy input"));
        
        json_array_append_new(errors_array, error_detail);
        json_object_set_new(error_obj, "errors", errors_array);
        
        return error_obj;
    }
    
    // Add the hello key with the configured value
    json_object_set_new(result, "hello", json_string(value));
    
    return result;
}
```

This middleware demonstrates:
- **Arena Usage**: Uses the arena allocator to copy the config string
- **Config Handling**: Takes a config parameter and uses it as the value (defaults to "world")
- **JSON Manipulation**: Uses standard jansson functions which automatically use arena allocation
- **Error Handling**: Returns error objects when operations fail

Usage in a `.wp` file:
```wp
GET /hello
  |> hello: `world`
```

The middleware receives the initial request JSON and adds `{ "hello": "world" }` to it.

## Memory Management

The runtime uses per-request memory arenas for efficient allocation and cleanup:

- **Parse Arena**: For AST nodes and parser data structures
- **Runtime Arena**: For long-lived runtime data
- **Request Arena**: Per-request allocations, freed after response

This approach eliminates memory leaks and provides predictable performance characteristics.

## Performance Features

- **Compiled Pipelines**: Routes are parsed once at startup
- **Middleware Caching**: Compiled jq programs and Lua scripts are cached
- **Arena Allocation**: Fast bump allocation with automatic cleanup
- **Minimal Copying**: JSON data flows through pipeline with minimal serialization

## Error Handling

The system provides structured error handling through the `result` step and a standardized error format:

### Error Format

All errors in the system follow a standardized JSON format with an `errors` array:

```json
{
  "errors": [
    {
      "type": "validationError",
      "message": "Missing required field",
      "field": "email"
    }
  ]
}
```

Each error object contains:
- **type**: Error category (e.g., `validationError`, `sqlError`, `internalError`)
- **message**: Human-readable error description
- **Additional fields**: Context-specific data like `field`, `sqlstate`, `severity`, etc.

### Error Types

Common error types include:
- `validationError`: Input validation failures
- `sqlError`: Database operation errors
- `internalError`: System/middleware internal errors
- `authError`: Authentication/authorization failures

### Result Step Processing

The `result` step matches errors against condition types:

- **Condition Matching**: Errors are matched against specific condition types using the `type` field
- **Status Codes**: Each condition specifies HTTP status codes
- **Error Propagation**: Errors flow through the pipeline like regular data
- **Fallback Handling**: `default` condition handles unmatched errors

### Middleware Error Creation

Middleware should create errors using the standardized format:

```c
json_t *error_obj = json_object();
json_t *errors_array = json_array();
json_t *error_detail = json_object();

json_object_set_new(error_detail, "type", json_string("validationError"));
json_object_set_new(error_detail, "message", json_string("Invalid input"));
json_object_set_new(error_detail, "field", json_string("email"));

json_array_append_new(errors_array, error_detail);
json_object_set_new(error_obj, "errors", errors_array);

return error_obj;
```

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