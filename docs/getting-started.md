# Getting Started

This guide will help you install Web Pipe, run your first API, and understand the basics of building with the DSL.

## Prerequisites

- **Rust** (1.70+): Web Pipe is written in Rust. Install from [rustup.rs](https://rustup.rs/)
- **PostgreSQL** (optional): For database-backed examples. Install from [postgresql.org](https://www.postgresql.org/)
- **jq** (optional): Helpful for testing JSON responses. Install from [jqlang.github.io/jq/](https://jqlang.github.io/jq/)

## Installation

### From Source

Clone the repository and build:

```bash
git clone https://github.com/your-org/webpipe.git
cd webpipe
cargo build --release
```

The compiled binary will be at `target/release/webpipe`.

### Add to PATH (Optional)

```bash
# Linux/macOS
sudo cp target/release/webpipe /usr/local/bin/

# Or add to your shell profile:
export PATH="$PATH:/path/to/webpipe/target/release"
```

### Verify Installation

```bash
webpipe --version
# or if not in PATH:
cargo run -- --version
```

## Your First API

Create a file called `hello.wp`:

```wp
GET /hello/:name
  |> jq: `{ greeting: "Hello, " + .params.name + "!" }`
```

Run it:

```bash
# Using cargo
cargo run hello.wp

# Or using the binary
webpipe hello.wp
```

The server will start on `http://localhost:3000` (default). Test it:

```bash
curl http://localhost:3000/hello/world
# {"greeting":"Hello, world!"}
```

## Basic Concepts

### 1. Routes and Pipelines

Routes define HTTP endpoints. The `|>` operator chains middleware steps:

```wp
GET /users/:id
  |> jq: `{ userId: (.params.id | tonumber) }`
  |> jq: `{ message: "User ID is " + (.userId | tostring) }`
```

**How it works:**
- Request comes in: `GET /users/123`
- First step extracts and converts the ID: `{ userId: 123 }`
- Second step creates a message: `{ message: "User ID is 123" }`
- Response returned as JSON

### 2. Accessing Request Data

Web Pipe provides request data in the initial JSON state:

```wp
GET /example
  |> jq: `{
    path: .params,           # URL parameters
    query: .query,           # Query string (?foo=bar)
    body: .body,             # POST/PUT body
    headers: .headers,       # HTTP headers
    method: .method,         # HTTP method
    path_str: .path          # Full path
  }`
```

Test it:

```bash
curl "http://localhost:3000/example?foo=bar&limit=10"
```

### 3. Multiple Routes

Define multiple routes in one file:

```wp
GET /
  |> jq: `{ message: "Welcome to the API" }`

GET /users
  |> jq: `{ users: ["Alice", "Bob", "Charlie"] }`

GET /users/:id
  |> jq: `{ userId: .params.id, name: "Alice" }`

POST /users
  |> jq: `{ created: true, user: .body }`
```

### 4. Variables for Reusability

Define reusable query fragments:

```wp
jq greetingTemplate = `{ greeting: "Hello, " + .name + "!" }`

GET /greet/:name
  |> jq: `{ name: .params.name }`
  |> jq: greetingTemplate
```

## Working with Databases

### Setup PostgreSQL Connection

Create a `.env` file in the same directory as your `.wp` file:

```env
WP_PG_HOST=localhost
WP_PG_PORT=5432
WP_PG_USER=postgres
WP_PG_PASSWORD=password
WP_PG_DATABASE=myapp
```

Configure in your `.wp` file:

```wp
config pg {
  host: $WP_PG_HOST || "localhost"
  port: $WP_PG_PORT || 5432
  user: $WP_PG_USER || "postgres"
  password: $WP_PG_PASSWORD || ""
  database: $WP_PG_DATABASE || "myapp"
}
```

### Create a Test Table

```sql
CREATE TABLE users (
  id SERIAL PRIMARY KEY,
  name VARCHAR(100) NOT NULL,
  email VARCHAR(100) NOT NULL UNIQUE,
  created_at TIMESTAMP DEFAULT NOW()
);

INSERT INTO users (name, email) VALUES
  ('Alice', 'alice@example.com'),
  ('Bob', 'bob@example.com'),
  ('Charlie', 'charlie@example.com');
```

### Query the Database

```wp
config pg {
  host: $WP_PG_HOST || "localhost"
  database: "myapp"
}

GET /users
  |> pg: `SELECT * FROM users ORDER BY id`
  |> jq: `{ users: .data.rows }`

GET /users/:id
  |> pg([.params.id]): `SELECT * FROM users WHERE id = $1`
  |> jq: `{ user: .data.rows[0] }`

POST /users
  |> validate: `{
    name: string(2..100),
    email: email
  }`
  |> pg([.body.name, .body.email]): `
    INSERT INTO users (name, email)
    VALUES ($1, $2)
    RETURNING *
  `
  |> jq: `{ user: .data.rows[0] }`
```

Test it:

```bash
# Get all users
curl http://localhost:3000/users

# Get specific user
curl http://localhost:3000/users/1

# Create new user
curl -X POST http://localhost:3000/users \
  -H "Content-Type: application/json" \
  -d '{"name":"David","email":"david@example.com"}'
```

## Named Results

Accumulate multiple query results using `@result`:

```wp
config pg {
  host: $WP_PG_HOST || "localhost"
  database: "myapp"
}

GET /dashboard
  |> pg: `SELECT COUNT(*) as count FROM users` @result(userCount)
  |> pg: `SELECT COUNT(*) as count FROM posts` @result(postCount)
  |> pg: `SELECT COUNT(*) as count FROM comments` @result(commentCount)
  |> jq: `{
    users: .data.userCount.rows[0].count,
    posts: .data.postCount.rows[0].count,
    comments: .data.commentCount.rows[0].count
  }`
```

## Calling External APIs

Use the `fetch` middleware to call external services:

```wp
GET /external/users/:id
  |> fetch("https://jsonplaceholder.typicode.com/users/" + .params.id)
  |> jq: `{ user: .data.response }`

GET /combined/:id
  |> fetch("https://jsonplaceholder.typicode.com/users/" + .params.id) @result(user)
  |> fetch("https://jsonplaceholder.typicode.com/posts?userId=" + .params.id) @result(posts)
  |> jq: `{
    user: .data.user.response,
    posts: .data.posts.response
  }`
```

## HTML Responses with Handlebars

Render HTML templates:

```wp
handlebars userCard = `
<!DOCTYPE html>
<html>
<head>
  <title>User Profile</title>
  <style>
    body { font-family: Arial, sans-serif; margin: 40px; }
    .card { border: 1px solid #ddd; padding: 20px; border-radius: 8px; }
  </style>
</head>
<body>
  <div class="card">
    <h1>{{name}}</h1>
    <p>Email: {{email}}</p>
    <p>Member since: {{created_at}}</p>
  </div>
</body>
</html>
`

config pg {
  host: $WP_PG_HOST || "localhost"
  database: "myapp"
}

GET /users/:id/profile
  |> pg([.params.id]): `SELECT * FROM users WHERE id = $1`
  |> jq: `.data.rows[0]`
  |> handlebars: userCard
```

Visit `http://localhost:3000/users/1/profile` in your browser.

## Pipelines for Reusability

Extract common logic into reusable pipelines:

```wp
config pg {
  host: $WP_PG_HOST || "localhost"
  database: "myapp"
}

pipeline loadUser =
  |> pg([.userId]): `SELECT * FROM users WHERE id = $1` @result(user)
  |> jq: `{ user: .data.user.rows[0] }`

GET /users/:id
  |> jq: `{ userId: (.params.id | tonumber) }`
  |> pipeline: loadUser

GET /users/:id/profile
  |> jq: `{ userId: (.params.id | tonumber) }`
  |> pipeline: loadUser
  |> jq: `. + { profile: true }`
```

## Conditional Logic

Use `if/then/else` for conditional behavior:

```wp
config pg {
  host: $WP_PG_HOST || "localhost"
  database: "myapp"
}

GET /users/:id
  |> pg([.params.id]): `SELECT * FROM users WHERE id = $1` @result(user)
  |> if
    |> jq: `.data.user.rows | length == 0`
    then:
      |> jq: `{ error: "User not found", status: 404 }`
    else:
      |> jq: `{ user: .data.user.rows[0] }`
    end
```

## Error Handling

Web Pipe automatically handles common errors. Use `result` blocks for custom error responses:

```wp
config pg {
  host: $WP_PG_HOST || "localhost"
  database: "myapp"
}

POST /users
  |> validate: `{
    name: string(2..100),
    email: email
  }`
  |> pg([.body.name, .body.email]): `
    INSERT INTO users (name, email) VALUES ($1, $2) RETURNING *
  `
  |> result
    ok(200):
      |> jq: `{ success: true, user: .data.rows[0] }`
    validationError(400):
      |> jq: `{ error: "Validation failed", details: .errors }`
    default(500):
      |> jq: `{ error: "Internal server error" }`
```

## Testing Your API

Web Pipe includes built-in testing with BDD syntax:

```wp
config pg {
  host: $WP_PG_HOST || "localhost"
  database: "myapp"
}

GET /users/:id
  |> pg([.params.id]): `SELECT * FROM users WHERE id = $1`
  |> jq: `{ user: .data.rows[0] }`

# Tests
describe "User API"
  let userId = 1

  with mock pg returning `{
    rows: [{
      id: $userId,
      name: "Alice",
      email: "alice@example.com"
    }],
    rowCount: 1
  }`

  it "returns user by ID"
    when calling GET /users/{{userId}}
    then status is 200
    and response.user.name equals "Alice"
    and response.user.email equals "alice@example.com"
```

Run tests:

```bash
cargo run hello.wp --test
# or
webpipe hello.wp --test
```

## Configuration Options

### Server Configuration

Create a `config server` block:

```wp
config server {
  port: $PORT || 3000
  host: $HOST || "0.0.0.0"
}

GET /
  |> jq: `{ message: "Hello, World!" }`
```

### Environment Variables

Web Pipe automatically loads environment variables from:
1. `.env` file in the same directory as your `.wp` file
2. `.env.local` (overrides `.env`, gitignored by default)
3. System environment variables (highest priority)

Example `.env`:

```env
PORT=8080
WP_PG_HOST=localhost
WP_PG_DATABASE=myapp
WP_PG_USER=postgres
WP_PG_PASSWORD=secret
API_KEY=your-secret-key
```

Use in your `.wp` file:

```wp
config server {
  port: $PORT || 3000
}

config pg {
  host: $WP_PG_HOST || "localhost"
  database: $WP_PG_DATABASE || "myapp"
  user: $WP_PG_USER || "postgres"
  password: $WP_PG_PASSWORD
}

GET /external
  |> fetch: `https://api.example.com/data?key=${API_KEY}`
```

## Command Line Options

```bash
# Run server
cargo run example.wp

# Run tests
cargo run example.wp --test

# Specify port
PORT=8080 cargo run example.wp

# Release build (faster, for production)
cargo build --release
./target/release/webpipe example.wp
```

## Project Structure

For larger projects, organize your `.wp` files:

```
my-api/
├── .env                    # Environment variables
├── .env.local             # Local overrides (gitignored)
├── main.wp                # Main API file
├── schema.sql             # Database schema
└── README.md
```

## Example: Complete REST API

Here's a complete example with CRUD operations:

```wp
config server {
  port: $PORT || 3000
}

config pg {
  host: $WP_PG_HOST || "localhost"
  database: $WP_PG_DATABASE || "myapp"
  user: $WP_PG_USER || "postgres"
  password: $WP_PG_PASSWORD
}

# List all users
GET /users
  |> pg: `SELECT * FROM users ORDER BY id`
  |> jq: `{ users: .data.rows, count: .data.rowCount }`

# Get single user
GET /users/:id
  |> pg([.params.id]): `SELECT * FROM users WHERE id = $1`
  |> if
    |> jq: `.data.rows | length == 0`
    then:
      |> jq: `{ error: "User not found", status: 404 }`
    else:
      |> jq: `{ user: .data.rows[0] }`
    end

# Create user
POST /users
  |> validate: `{
    name: string(2..100),
    email: email
  }`
  |> pg([.body.name, .body.email]): `
    INSERT INTO users (name, email)
    VALUES ($1, $2)
    RETURNING *
  `
  |> result
    ok(201):
      |> jq: `{ user: .data.rows[0] }`
    validationError(400):
      |> jq: `{ error: "Validation failed", details: .errors }`

# Update user
PUT /users/:id
  |> validate: `{
    name: string(2..100),
    email: email
  }`
  |> pg([.body.name, .body.email, .params.id]): `
    UPDATE users
    SET name = $1, email = $2
    WHERE id = $3
    RETURNING *
  `
  |> if
    |> jq: `.data.rows | length == 0`
    then:
      |> jq: `{ error: "User not found", status: 404 }`
    else:
      |> jq: `{ user: .data.rows[0] }`
    end

# Delete user
DELETE /users/:id
  |> pg([.params.id]): `DELETE FROM users WHERE id = $1 RETURNING *`
  |> if
    |> jq: `.data.rows | length == 0`
    then:
      |> jq: `{ error: "User not found", status: 404 }`
    else:
      |> jq: `{ message: "User deleted", user: .data.rows[0] }`
    end
```

## Next Steps

Now that you have the basics, explore more advanced features:

- **[DSL Syntax](./dsl-syntax.md)**: Complete language reference with all tags and control flow
- **[Routes & Pipelines](./routes-and-pipelines.md)**: Learn about pipeline composition and named results
- **[Inline Arguments](./inline-arguments.md)**: Pass dynamic data directly to middleware
- **[Flow Control](./flow-control.md)**: Master `dispatch`, `foreach`, `@when`, and `@guard`
- **[GraphQL](./graphql.md)**: Build GraphQL APIs with resolvers and DataLoaders
- **[Concurrency](./concurrency.md)**: Execute tasks in parallel with `@async` and `join`
- **[Testing](./testing-bdd.md)**: Write comprehensive tests with BDD syntax
- **[Deployment](./deployment-docker.md)**: Deploy with Docker and production setup

## Common Patterns

### Health Check Endpoint

```wp
GET /health
  |> jq: `{ status: "ok", timestamp: now }`
```

### CORS Headers

```wp
GET /api/data
  |> jq: `{
    data: "example",
    headers: {
      "Access-Control-Allow-Origin": "*",
      "Access-Control-Allow-Methods": "GET, POST, PUT, DELETE"
    }
  }`
```

### Pagination

```wp
config pg {
  host: $WP_PG_HOST || "localhost"
  database: "myapp"
}

GET /users
  |> jq: `{
    limit: (.query.limit // 10 | tonumber),
    offset: (.query.offset // 0 | tonumber)
  }`
  |> pg([.limit, .offset]): `
    SELECT * FROM users
    ORDER BY id
    LIMIT $1 OFFSET $2
  `
  |> jq: `{
    users: .data.rows,
    count: .data.rowCount,
    limit: .limit,
    offset: .offset
  }`
```

Test:
```bash
curl "http://localhost:3000/users?limit=5&offset=10"
```

### Authentication Guard

```wp
config auth {
  sessionTtl: 604800
}

pipeline requireAuth =
  |> auth: "required"
  |> result
    authRequired(401):
      |> jq: `{ error: "Authentication required" }`

GET /protected
  |> pipeline: requireAuth
  |> jq: `{ message: "Protected resource", user: .user }`
```

## Troubleshooting

### Database Connection Errors

```
Error: Failed to connect to PostgreSQL
```

**Solutions:**
1. Check `.env` file exists and has correct credentials
2. Verify PostgreSQL is running: `pg_isready`
3. Test connection: `psql -h localhost -U postgres -d myapp`

### Port Already in Use

```
Error: Address already in use (os error 48)
```

**Solutions:**
1. Change port: `PORT=8080 cargo run example.wp`
2. Kill process using port: `lsof -ti:3000 | xargs kill`

### JQ Syntax Errors

```
Error: jq parse error
```

**Solutions:**
1. Validate JQ expressions: `echo '{"test":1}' | jq '.test'`
2. Check for unescaped quotes in strings
3. Use backticks for multi-line JQ: `` `{ ... }` ``

## Getting Help

- **Documentation**: [Full documentation](./README.md)
- **Examples**: Check the `examples/` directory in the repository
- **Issues**: Report bugs at [GitHub Issues](https://github.com/your-org/webpipe/issues)

Welcome to Web Pipe! Start building your first API and explore the documentation as you go.
