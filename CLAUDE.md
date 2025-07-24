# Web Pipe (wp) - Development Guide

You are developing a web server runtime for a DSL that processes HTTP requests through pipeline-based middleware.

## Core Architecture

- **Runtime**: HTTP server using libmicrohttpd that executes pipeline steps
- **Lexer/Parser**: Parses `.wp` files into AST
- **Middleware System**: Dynamically loaded `.so` files that process JSON data
- **Memory Management**: Per-request bump allocator arenas

## Language Syntax

### Basic Pipeline
```wp
GET /page/:id
  |> jq: `{ id: .params.id }`
  |> lua: `return { sqlParams: { request.id } }`
  |> pg: `select * from pages where id = $1`
```

### Variables and Pipelines
```wp
pg getUserQuery = `SELECT * FROM users WHERE id = $1`
pipeline getPage = |> jq: `{ sqlParams: [.params.id] }` |> pg: getUserQuery

GET /users/:id
  |> pipeline: getPage
```

### Error Handling with Result Steps
```wp
POST /users
  |> validate: `{ name: string(3..50), email: email }`
  |> pg: `INSERT INTO users (name, email) VALUES ($1, $2) RETURNING *`
  |> result
    ok(201): |> jq: `{ success: true, user: .data.rows[0] }`
    validationError(400): |> jq: `{ error: "Validation failed" }`
    default(500): |> jq: `{ error: "Internal server error" }`
```

### Configuration Blocks
```wp
config pg {
  host: $DB_HOST || "localhost"
  port: $DB_PORT || 5432
  database: $DB_NAME || "myapp"
  user: $DB_USER || "postgres"
  password: $DB_PASSWORD
}
```

## Request Flow

1. **HTTP Request** → **JSON Request Object** with `query`, `body`, `params`, `headers`
2. **Pipeline Execution** → Each middleware receives and returns `json_t`
3. **Memory Arena** → Per-request allocations, freed after response
4. **Response Generation** → Final JSON converted to HTTP response

## Built-in Middleware

- **jq**: JSON transformations using jq expressions
- **lua**: Lua scripts with `request` object access and optional `executeSql()` function
- **pg**: PostgreSQL queries with parameter binding via `sqlParams`
- **validate**: Input validation with rules like `string(3..50)`, `email`, `number`
- **mustache**: HTML template rendering with partials support
- **auth**: User authentication with session management, password hashing
- **cache**: Response caching with TTL, LRU eviction, template-based keys

## Middleware Development

### Required Interface
```c
json_t *middleware_execute(json_t *input, void *arena, 
                          arena_alloc_func alloc_func, 
                          arena_free_func free_func, 
                          const char *config,
                          json_t *middleware_config,
                          char **contentType, 
                          json_t *variables);
```

### Optional Functions
```c
int middleware_init(json_t *config);  // Called at startup
json_t *execute_sql(const char *sql, json_t *params, void *arena, arena_alloc_func alloc_func);  // Database providers
```

### Key Points
- **Jansson Integration**: `json_set_alloc_funcs()` configured for arena allocation
- **Memory Management**: All JSON objects automatically use per-request arena
- **Database Registry**: Middleware can register as database providers via `execute_sql`
- **Error Format**: Standardized `{ "errors": [{ "type": "...", "message": "..." }] }`

## Build System

### Main Targets
- `make` - Build main executable and middleware
- `make debug` - Build with debug symbols and AddressSanitizer
- `make middleware` - Build all middleware .so files
- `make install-middleware` - Copy .so files to ./middleware/

### Testing (Full CI Command)
```bash
make clean && make && make install-middleware && make test-lint && make test-analyze && make test && make test-leaks
```

### Test Categories
- `make test-unit` - Core component tests
- `make test-integration` - Middleware tests  
- `make test-system` - End-to-end tests
- `make test-leaks` - Memory leak detection (valgrind/leaks)

## Running

```bash
./build/wp
Error: wp_file is required
Usage: ./build/wp <wp_file> [options]

Options:
  --daemon         Run in daemon mode (background service)
  --test           Run in test mode (until SIGTERM)
  --timeout <sec>  Run for specified seconds then exit
  --port <num>     Port to listen on (default: 8080, env: WP_PORT)
  --watch          Enable file monitoring (default: enabled)
  --no-watch       Disable file monitoring
  --help           Show this help message

Default mode is interactive (press Ctrl+C to stop)
```

```bash
make clean && make run-debug
```

## File Structure
```
test.wp          - Example wp file
src/
  wp.c           - Main entry point
  lexer.c        - Tokenization
  parser.c       - AST generation
  server.c       - HTTP server and pipeline execution
  database_registry.c - Database provider system
  middleware/    - All middleware implementations
build/
  wp             - Main executable
  *.so           - Compiled middleware
middleware/      - Runtime middleware directory
test/
  unit/          - Unit tests
  integration/   - Middleware tests
  system/        - E2E tests
```

## Development Guidelines

- **C99 Standard**: Use clang compiler
- **Memory Safety**: All allocations use arena allocators
- **Error Handling**: Follow standardized error format with `errors` array
- **Platform**: Primary target is macOS with Homebrew dependencies and Linux with apt dependencies
- **Testing**: Run full test suite before commits
- **Static Analysis**: Code must pass clang-tidy and clang analyzer

## Key Dependencies
- libmicrohttpd (HTTP server)
- jansson (JSON processing)
- jq (JSON queries)
- lua (Scripting)
- libpq (PostgreSQL)
- argon2 (Password hashing)