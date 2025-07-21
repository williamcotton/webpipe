![Test Suite](https://github.com/williamcotton/webpipe/workflows/Test%20Suite/badge.svg)

<img src="./wp.png" width="200">

```wp
GET /hello
  |> jq: `{ world: ":)"}`
```

# Web Pipe

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
  |> jq: `{ sqlParams: [] }`
  |> pg: articlesQuery
```

### Pipeline Variables

Define reusable pipeline sequences that can be referenced by name:

```wp
pipeline getPage = 
  |> jq: `{ sqlParams: [.params.id | tostring] }`
  |> pg: `SELECT * FROM teams WHERE id = $1`
  |> jq: `{ team: .data.rows[0] }`

GET /page/:id
  |> pipeline: getPage
```

Pipeline variables allow you to:
- **Reuse Logic**: Define complex pipeline sequences once and use them in multiple routes
- **Modular Design**: Break down complex processing into named, testable components
- **Maintainability**: Change pipeline logic in one place and have it apply everywhere it's used

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
Executes Lua scripts with full access to the request object and database functions.

```wp
GET /process
  |> lua: `
    return {
      sqlParams = { request.body.name, request.body.email }
    }
  `
```

#### Database Access in Lua
The Lua middleware provides access to the database through the `executeSql` function, **but only when a database middleware that has registered with the database registry is loaded**:

```wp
GET /lua-db-test
  |> lua: `
    local result, err = executeSql("SELECT * FROM users LIMIT 5")
    
    if err then
      return {
        error = "Database error: " .. err,
        sql = "SELECT * FROM users LIMIT 5"
      }
    end
    
    return {
      message = "Database query successful",
      data = result,
      luaVersion = _VERSION
    }
  `
```

**Prerequisites for Database Access:**
- A database middleware that implements the `execute_sql` function must be loaded in your application
- The database middleware must register itself with the database registry system
- The database middleware must be properly configured with its respective config block

If no database middleware is registered, calls to `executeSql` will return an error.

### PostgreSQL Middleware
Executes SQL queries with parameter binding from `sqlParams`.

```wp
GET /users/:id
  |> jq: `{ sqlParams: [.params.id] }`
  |> pg: `SELECT * FROM users WHERE id = $1`
```

**Features:**
- Implements `middleware_init` for database connection initialization using config blocks
- Registers with the database registry system to provide `execute_sql` function
- Thread-safe database operations with connection pooling
- Automatic parameter binding with type conversion
- Comprehensive error reporting with PostgreSQL-specific diagnostics

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

## HTTP Methods Support

WebPipe supports all standard HTTP methods with proper request body handling:

### GET Requests
Standard GET requests with query parameters and URL parameters:

```wp
GET /users/:id
  |> jq: `{ userId: .params.id, filters: .query }`
```

### POST Requests
POST requests with JSON or form data bodies:

```wp
POST /users
  |> jq: `{
    method: .method,
    name: .body.name,
    email: .body.email,
    action: "create"
  }`
```

### PUT Requests
PUT requests for full resource updates:

```wp
PUT /users/:id
  |> jq: `{
    method: .method,
    id: (.params.id | tonumber),
    name: .body.name,
    email: .body.email,
    action: "update"
  }`
```

### PATCH Requests
PATCH requests for partial resource updates:

```wp
PATCH /users/:id
  |> jq: `{
    method: .method,
    id: (.params.id | tonumber),
    body: .body,
    action: "partial_update"
  }`
```

### DELETE Requests
DELETE requests for resource removal:

```wp
DELETE /users/:id
  |> jq: `{ sqlParams: [.params.id] }`
  |> pg: `DELETE FROM users WHERE id = $1`
  |> jq: `{ success: true, message: "User deleted" }`
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
  lua \
  gnuplot
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
# Configuration blocks
config pg {
  host: $DB_HOST || "localhost"
  port: $DB_PORT || 5432
  database: $DB_NAME || "myapp"
  user: $DB_USER || "postgres"
  password: $DB_PASSWORD || "secret"
  ssl: true
}

config validate {
  strictMode: true
  customMessages: {
    required: "This field is required"
    email: "Please enter a valid email address"
  }
}

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
  |> validate: `{
    name: string(3..50, required),
    email: email(required)
  }`
  |> jq: `{ sqlParams: [.body.name, .body.email] }`
  |> pg: `INSERT INTO users (name, email) VALUES ($1, $2) RETURNING *`
  |> result
    ok(201):
      |> jq: `{
        success: true,
        user: .data.rows[0]
      }`
    validationError(400):
      |> jq: `{
        error: "Validation failed",
        field: .errors[0].field,
        message: .errors[0].message
      }`
    sqlError(500):
      |> jq: `{
        error: "Database error",
        sqlstate: .errors[0].sqlstate,
        message: .errors[0].message,
        query: .errors[0].query
      }`
    default(500):
      |> jq: `{ error: "Internal server error" }`

# Cookie handling example
GET /login
  |> jq: `{
    message: "Login successful",
    userId: .cookies.sessionId // "guest",
    setCookies: [
      "sessionId=abc123; HttpOnly; Secure; Max-Age=3600",
      "userId=" + (.cookies.sessionId // "guest") + "; Max-Age=86400"
    ]
  }`

# Form data handling
POST /contact
  |> jq: `{
    name: .body.name,
    email: .body.email,
    message: .body.message,
    timestamp: now
  }`
  |> pg: `INSERT INTO contacts (name, email, message) VALUES ($1, $2, $3) RETURNING *`
  |> result
    ok(201):
      |> jq: `{
        success: true,
        message: "Contact form submitted successfully"
      }`
    default(500):
      |> jq: `{ error: "Failed to submit contact form" }`

# Lua with database access (requires database middleware to be registered)
GET /lua-stats
  |> lua: `
    local userCount, err = executeSql("SELECT COUNT(*) as count FROM users")
    if err then
      return { error = "Database error: " .. err }
    end
    
    local activeUsers, err = executeSql("SELECT COUNT(*) as count FROM users WHERE active = true")
    if err then
      return { error = "Database error: " .. err }
    end
    
    return {
      totalUsers = userCount.rows[1].count,
      activeUsers = activeUsers.rows[1].count,
      luaVersion = _VERSION
    }
  `
```

## Cookie Support

WebPipe provides comprehensive cookie support for both reading incoming cookies and setting outgoing cookies.

### Reading Cookies

Incoming cookies are automatically parsed and made available in the request object:

```wp
GET /cookie-test
  |> jq: `{
    message: "Cookie values received",
    sessionId: .cookies.sessionId,
    userId: .cookies.userId,
    theme: .cookies.theme
  }`
```

### Setting Cookies

Cookies can be set by adding them to the `setCookies` array in the response:

```wp
GET /login
  |> jq: `{
    message: "Login successful",
    setCookies: [
      "sessionId=abc123; HttpOnly; Secure; Max-Age=3600",
      "userId=john; Max-Age=86400; Path=/",
      "theme=dark; Path=/; SameSite=Strict"
    ]
  }`
```

**Cookie Attributes Supported:**
- `HttpOnly`: Prevents JavaScript access to the cookie
- `Secure`: Cookie only sent over HTTPS
- `Max-Age`: Cookie expiration time in seconds
- `Path`: URL path where cookie is valid
- `Domain`: Domain where cookie is valid
- `SameSite`: CSRF protection (Strict, Lax, None)

### Cookie Example

```wp
GET /cookies
  |> jq: `{
    message: "Cookie test response",
    cookies: .cookies,
    setCookies: [
      "sessionId=abc123; HttpOnly; Secure; Max-Age=3600",
      "userId=john; Max-Age=86400",
      "theme=dark; Path=/"
    ]
  }`
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

## Form Data Support

WebPipe supports both JSON and form-encoded request bodies for POST, PUT, and PATCH requests.

### JSON Request Bodies

JSON request bodies are automatically parsed and made available in the `body` field:

```wp
POST /users
  |> jq: `{
    name: .body.name,
    email: .body.email,
    action: "create"
  }`
```

### Form-Encoded Request Bodies

Form-encoded data (`application/x-www-form-urlencoded`) is automatically parsed and converted to JSON:

```wp
# HTML form submitting to this endpoint
POST /form-submit
  |> jq: `{
    username: .body.username,
    password: .body.password,
    remember: (.body.remember // false)
  }`
```

**Example HTML Form:**
```html
<form action="/form-submit" method="post">
  <input type="text" name="username" required>
  <input type="password" name="password" required>
  <input type="checkbox" name="remember" value="true">
  <button type="submit">Submit</button>
</form>
```

### Request Body Handling

The middleware automatically detects the content type and processes the request body accordingly:

- **`application/json`**: Parsed as JSON object
- **`application/x-www-form-urlencoded`**: Parsed as form data and converted to JSON
- **Other content types**: Stored as string in the `body` field

### Testing Request Bodies

```wp
POST /test-body
  |> jq: `{
    method: .method,
    body: .body,
    hasBody: (.body != null),
    bodyType: (.body | type)
  }`
```


## Database Registry System

WebPipe includes a database registry system that allows middleware to register as database providers and share database functionality with other middleware.

### Database Providers

Middleware can register as database providers by implementing the `execute_sql` function:

```c
// Public execute_sql function for database registry
json_t *execute_sql(const char *sql, json_t *params, void *arena, arena_alloc_func alloc_func) {
    // Implementation depends on database type (PostgreSQL, MySQL, SQLite, etc.)
    return execute_sql_internal(sql, params, arena, alloc_func, middleware_config);
}
```

Any middleware can become a database provider by implementing this function and registering with the database registry system.

### Using Database Functions in Middleware

Middleware can access database functions through the WebPipe Database API:

```c
typedef struct {
    json_t* (*execute_sql)(const char* sql, json_t* params, void* arena, arena_alloc_func alloc_func);
    DatabaseProvider* (*get_database_provider)(const char* name);
    bool (*has_database_provider)(void);
    const char* (*get_default_database_provider_name)(void);
} WebpipeDatabaseAPI;

// In middleware, access the API through the global symbol
extern WebpipeDatabaseAPI webpipe_db_api;
```

### Example: Using Database in Lua Middleware

The Lua middleware uses the database registry to provide the `executeSql` function, **but only when a database provider middleware is registered**:

```wp
# This requires a database middleware to be loaded and registered
GET /lua-db-example
  |> lua: `
    local result, err = executeSql("SELECT * FROM users WHERE active = true")
    if err then
      return { error = "Database error: " .. err }
    end
    return { users = result.rows }
  `
```

**Note:** The `executeSql` function is only available in Lua when a database middleware that implements the `execute_sql` function is registered with the database registry.

### Database Provider Registration

The runtime automatically:
1. Loads middleware and checks for `execute_sql` function
2. Registers middleware as database provider if function exists
3. Injects database API into middleware that request it
4. Provides database functions to middleware through the API

**Examples of Database Middleware:**
- **PostgreSQL (`pg`)**: Implements `middleware_init` and `execute_sql` for PostgreSQL databases
- **Custom Database Middleware**: Any middleware can become a database provider by implementing the `execute_sql` function

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
                      json_t *middleware_config,
                      char **contentType, 
                      json_t *variables);
} Middleware;
```

**Required Functions:**
- `middleware_execute`: Main processing function (required)

**Optional Functions:**
- `middleware_init`: Initialization function called at startup
- `execute_sql`: Database provider function (for database middleware)

Each middleware receives:
- `input`: JSON data from previous pipeline step
- `arena`: Memory arena for allocations
- `alloc_func`/`free_func`: Arena allocation functions
- `config`: Step-specific configuration string
- `middleware_config`: Middleware-wide configuration from config blocks
- `contentType`: Pointer to content type string (can be modified)
- `variables`: User-defined variables from the WP file

### Automatic Memory Management with Jansson

The runtime automatically configures jansson to use per-request arena allocators at startup by calling `json_set_alloc_funcs()` in `server.c`. This means that **middleware developers can use jansson JSON objects normally** without worrying about memory management - all JSON allocations will automatically use the per-request arena allocator.

When creating, manipulating, or returning `json_t` objects in middleware:
- Use standard jansson functions (`json_object()`, `json_array()`, `json_string()`, etc.)
- No need to manually manage memory for JSON objects
- All JSON memory is automatically freed when the request completes
- The arena ensures no memory leaks even if middleware don't explicitly decref objects

This seamless integration allows middleware to focus on business logic rather than memory management details.

### Middleware Initialization

Middleware can optionally implement a `middleware_init` function that is called at startup with the middleware's configuration:

```c
// Optional middleware initialization function
int middleware_init(json_t *config) {
    // Perform initialization tasks using the middleware's config block
    // Return 0 for success, non-zero for failure
    
    // Example: Initialize database connection
    if (config) {
        json_t *host = json_object_get(config, "host");
        if (host && json_is_string(host)) {
            // Initialize connection with host
            printf("Initializing middleware with host: %s\n", json_string_value(host));
        }
    }
    
    return 0; // Success
}
```

**Middleware Initialization Details:**
- **Function Signature**: `int middleware_init(json_t *config)`
- **Called**: Once at server startup, after middleware is loaded
- **Parameters**: `config` - The middleware's configuration block from the `.wp` file
- **Return Value**: `0` for success, non-zero for failure
- **Optional**: If not implemented, middleware loads without initialization
- **Error Handling**: If initialization fails, a warning is logged but the middleware remains loaded

**Example Configuration Usage:**
```wp
# Configuration block for custom middleware
config my_middleware {
  host: "localhost"
  port: 8080
  timeout: 30
}

GET /test
  |> my_middleware: `some config`
```

The `middleware_init` function receives the `my_middleware` configuration block as its `config` parameter.

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
json_t *middleware_execute(json_t *input, void *arena, arena_alloc_func alloc_func, arena_free_func free_func, const char *config, json_t *middleware_config, char **contentType, json_t *variables) {
    // Suppress unused parameter warnings
    (void)free_func;
    (void)contentType;  // This middleware doesn't change content type
    (void)variables;    // This middleware doesn't use variables
    
    // Handle null input - create empty object
    if (!input) {
        input = json_object();
    }
    
    // Handle null or empty config - use "world" as default
    const char *value = "world";
    if (config && strlen(config) > 0) {
        value = config;
    }
    
    // Check middleware configuration for default value
    if (middleware_config) {
        json_t *default_value = json_object_get(middleware_config, "defaultValue");
        if (default_value && json_is_string(default_value)) {
            value = json_string_value(default_value);
        }
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
config hello {
  defaultValue: "world"
}

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

#### SQL Error Details

PostgreSQL errors include additional diagnostic information:

```json
{
  "errors": [
    {
      "type": "sqlError",
      "message": "relation \"nonexistent_table\" does not exist",
      "sqlstate": "42P01",
      "severity": "ERROR",
      "query": "SELECT * FROM nonexistent_table"
    }
  ]
}
```

**Additional SQL Error Fields:**
- `sqlstate`: PostgreSQL error code (e.g., "42P01" for undefined table)
- `severity`: Error severity level (ERROR, WARNING, etc.)
- `query`: The SQL query that caused the error (added by middleware)

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

## Configuration System

WebPipe provides a powerful configuration system that allows you to configure middleware through configuration blocks and environment variables. This system supports automatic `.env` file loading for seamless environment variable management.

### Configuration Blocks

Configuration blocks use native WP language syntax to define structured configuration for middleware:

```wp
config pg {
  host: "localhost"
  port: 5432
  database: "myapp"
  user: $DB_USER || "postgres"
  password: $DB_PASSWORD || "secret"
  ssl: true
}

config auth {
  sessionTtl: 3600
  cookieName: "wp_session"
  cookieSecure: true
  cookieHttpOnly: true
  cookieSameSite: "strict"
}
```

### Environment Variable Integration

The `$VAR || "default"` syntax allows you to use environment variables with optional default values:

```wp
config pg {
  host: $DB_HOST || "localhost"       # Uses DB_HOST or defaults to "localhost"
  port: $DB_PORT || 5432             # Uses DB_PORT or defaults to 5432
  password: $DB_PASSWORD             # Required environment variable
}
```

Variables are resolved in this order:
1. System environment variables (highest priority)
2. Variables from `.env` file (loaded automatically)
3. Default values provided in `||` expressions (lowest priority)

### .env File Support

WebPipe automatically loads `.env` files from the same directory as your `.wp` file:

```bash
# .env file
DB_HOST=localhost
DB_PORT=5432
DB_USER=myuser
DB_PASSWORD=mypassword
DB_NAME=myapp
SESSION_SECRET=supersecret

# Variable expansion is supported
DATABASE_URL=postgres://${DB_USER}:${DB_PASSWORD}@${DB_HOST}:${DB_PORT}/${DB_NAME}
```

### Configuration Types

Configuration blocks support various data types:

```wp
config example {
  # Basic types
  stringValue: "hello world"
  intValue: 42
  floatValue: 3.14
  boolValue: true
  nullValue: null
  
  # Nested objects
  database: {
    host: "localhost"
    port: 5432
    ssl: true
  }
  
  # Arrays
  allowedDomains: ["example.com", "subdomain.example.com"]
  
  # Comments are supported
  sessionTtl: 3600  # 1 hour
}
```

### Middleware Configuration Examples

#### Database Configuration
```wp
config pg {
  host: $DB_HOST || "localhost"
  port: $DB_PORT || 5432
  database: $DB_NAME || "myapp"
  user: $DB_USER || "postgres"
  password: $DB_PASSWORD || "secret"
  ssl: true
  connectionTimeout: 30
  maxConnections: 10
}

GET /users
  |> pg: `SELECT * FROM users`
```

#### Authentication Configuration
```wp
config auth {
  sessionTtl: 7200  # 2 hours
  cookieName: "wp_session"
  cookieSecure: true
  cookieHttpOnly: true
  cookieSameSite: "strict"
  cookieDomain: ".example.com"
  passwordPolicy: {
    minLength: 8
    requireSpecialChars: true
    requireNumbers: true
  }
}

POST /login
  |> auth: "login"
  |> result
    ok(200):
      |> jq: `{success: true, user: .user}`
    authError(401):
      |> jq: `{error: "Invalid credentials"}`
```

#### Validation Configuration
```wp
config validate {
  strictMode: true
  customMessages: {
    required: "This field is required"
    email: "Please enter a valid email address"
    minLength: "Must be at least {min} characters"
  }
  maxFileSize: 10485760  # 10MB
}

POST /users
  |> validate: `{
    name: string(3..50, required),
    email: email(required),
    age: number(18..120)
  }`
  |> pg: `INSERT INTO users (name, email, age) VALUES ($1, $2, $3)`
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
