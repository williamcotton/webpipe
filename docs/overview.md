# Overview

Web Pipe is a **domain-specific language (DSL) and runtime** for building HTTP APIs through composable middleware pipelines. Instead of writing imperative code, you declare routes and data transformations that flow through a series of steps—from request to response.

## What is Web Pipe?

Web Pipe lets you build APIs by composing middleware steps into pipelines. Each step receives JSON, transforms it, and passes the result to the next step. The final step produces the HTTP response.

**Core Philosophy:**
- **Declarative over Imperative**: Describe what you want, not how to do it
- **Composition over Complexity**: Build complex behavior from simple, reusable pieces
- **Data Flow over Control Flow**: Transform JSON through a series of steps

## Why Web Pipe?

### Traditional API Framework
```javascript
app.get('/users/:id', async (req, res) => {
  const userId = parseInt(req.params.id);
  const user = await db.query('SELECT * FROM users WHERE id = $1', [userId]);
  const orders = await db.query('SELECT * FROM orders WHERE user_id = $1', [userId]);

  if (!user) {
    return res.status(404).json({ error: 'User not found' });
  }

  res.json({
    user: user.rows[0],
    orders: orders.rows,
    orderCount: orders.rowCount
  });
});
```

### Web Pipe Approach
```wp
GET /users/:id
  |> pg([.params.id]): `SELECT * FROM users WHERE id = $1` @result(user)
  |> pg([.params.id]): `SELECT * FROM orders WHERE user_id = $1` @result(orders)
  |> jq: `{
    user: .data.user.rows[0],
    orders: .data.orders.rows,
    orderCount: .data.orders.rowCount
  }`
```

**Benefits:**
- No boilerplate (no imports, app setup, route handler functions)
- Automatic error handling (validation, DB errors, network failures)
- Built-in result accumulation with `@result` tags
- Type-safe SQL parameter binding
- Declarative data flow

## Core Concepts

### 1. Routes
Map HTTP requests to processing pipelines:

```wp
GET /users/:id
POST /users
PUT /users/:id
DELETE /users/:id
```

### 2. Pipelines
Reusable sequences of middleware steps:

```wp
pipeline authenticateUser =
  |> auth: "required"
  |> jq: `{ userId: .user.id }`
```

### 3. Middleware
Built-in transformations and operations:

- **jq**: Transform JSON
- **pg**: Query PostgreSQL
- **fetch**: Call external APIs
- **graphql**: Execute GraphQL queries
- **handlebars**: Render HTML templates
- **lua**: Custom scripting
- **auth**: Authentication
- **cache**: Response caching
- **validate**: Input validation
- **log**: Request logging

### 4. Variables
Reusable SQL queries, templates, and configuration:

```wp
pg getUserById = `SELECT * FROM users WHERE id = $1`
handlebars userCard = `<div>{{name}}</div>`
```

### 5. Configuration
Environment-specific settings:

```wp
config pg {
  host: $WP_PG_HOST || "localhost"
  database: "myapp"
}

config auth {
  sessionTtl: 604800
}
```

## Execution Model

Each middleware step:
1. Receives the **current JSON state**
2. Performs an operation (query, transform, HTTP call)
3. Returns **updated JSON state** or an error
4. Passes the result to the next step

```wp
GET /users/:id
  # State: { params: { id: "123" }, query: {}, body: {}, headers: {} }
  |> jq: `{ userId: (.params.id | tonumber) }`
  # State: { userId: 123 }
  |> pg([.userId]): `SELECT * FROM users WHERE id = $1`
  # State: { userId: 123, data: { rows: [...], rowCount: 1 } }
  |> jq: `{ user: .data.rows[0] }`
  # Response: { user: { id: 123, name: "Alice", ... } }
```

## Key Features

### Named Results with @result
Accumulate multiple query results:

```wp
GET /dashboard
  |> pg: `SELECT * FROM users` @result(users)
  |> pg: `SELECT * FROM orders` @result(orders)
  |> pg: `SELECT * FROM products` @result(products)
  |> jq: `{
    userCount: .data.users.rowCount,
    orderCount: .data.orders.rowCount,
    productCount: .data.products.rowCount
  }`
```

### Async/Parallel Execution
Execute independent tasks concurrently:

```wp
GET /dashboard
  |> fetch: `https://api.example.com/users` @async(users)
  |> fetch: `https://api.example.com/stats` @async(stats)
  |> pg: `SELECT * FROM local_data` @async(local)
  |> join: `users, stats, local`
  |> jq: `{
    external: .async.users.data,
    statistics: .async.stats.data,
    internal: .async.local.data.rows
  }`
```

### Conditional Execution
Branch logic with dispatch, @when, @guard:

```wp
GET /content
  |> dispatch
    case @env(production):
      |> pg: `SELECT * FROM prod_content`
    case @env(development):
      |> pg: `SELECT * FROM dev_content`
    default:
      |> jq: `{ error: "Unknown environment" }`
```

### GraphQL Support
Native GraphQL schema and resolver support:

```wp
graphql {
  type User {
    id: ID!
    name: String!
    email: String!
    posts: [Post!]!
  }

  type Query {
    user(id: ID!): User
    users: [User!]!
  }
}

resolver Query.users =
  |> pg: `SELECT * FROM users`
  |> jq: `{ users: .data.rows }`

resolver User.posts =
  |> loader(.parent.id): PostsByUserLoader
```

### Built-in Testing
BDD-style testing with mocking:

```wp
describe "User API"
  let userId = 1

  with mock pg.getUserById returning `{
    id: $userId,
    name: "Alice",
    email: "alice@example.com"
  }`

  it "fetches user by ID"
    when calling GET /users/{{userId}}
    then status is 200
    and response.user.name equals "Alice"
```

## Data Flow Example

Here's a complete example showing data flow through a pipeline:

```wp
config pg {
  host: $WP_PG_HOST || "localhost"
  database: "myapp"
}

pipeline loadUser =
  |> pg([.userId]): `SELECT * FROM users WHERE id = $1` @result(user)

GET /users/:id/profile
  # 1. Extract user ID from URL
  |> jq: `{ userId: (.params.id | tonumber) }`

  # 2. Load user data using pipeline
  |> pipeline: loadUser

  # 3. Check if user exists
  |> if
    |> jq: `.data.user.rows | length == 0`
    then:
      |> jq: `{ error: "User not found", status: 404 }`
    end

  # 4. Load related orders in parallel
  |> pg([.userId]): `SELECT * FROM orders WHERE user_id = $1` @result(orders)

  # 5. Transform and return response
  |> jq: `{
    user: .data.user.rows[0],
    orders: .data.orders.rows,
    totalOrders: .data.orders.rowCount
  }`
```

## When to Use Web Pipe

**Great for:**
- REST APIs with database backends
- GraphQL APIs
- API gateways and proxies
- Data aggregation services
- Backend-for-frontend (BFF) layers
- Microservices with simple logic

**Not ideal for:**
- Complex business logic with heavy computation
- Real-time websocket servers (though HTTP SSE works)
- Applications requiring custom protocol handling
- Projects where team prefers traditional programming languages

## Architecture

```
┌─────────────────────────────────────────────────┐
│              HTTP Request                       │
│         GET /users/123?include=orders           │
└─────────────────────────────────────────────────┘
                      │
                      ▼
┌─────────────────────────────────────────────────┐
│           Request Parser                        │
│  { params: {id: "123"}, query: {include: ...} } │
└─────────────────────────────────────────────────┘
                      │
                      ▼
┌─────────────────────────────────────────────────┐
│         featureFlags Pipeline                   │
│    (runs before every route if defined)         │
└─────────────────────────────────────────────────┘
                      │
                      ▼
┌─────────────────────────────────────────────────┐
│            Route Pipeline                       │
│   |> jq: transform                              │
│   |> pg: query database                         │
│   |> fetch: external API                        │
│   |> jq: format response                        │
└─────────────────────────────────────────────────┘
                      │
                      ▼
┌─────────────────────────────────────────────────┐
│          Response Builder                       │
│  Status: 200, Content-Type: application/json    │
└─────────────────────────────────────────────────┘
                      │
                      ▼
┌─────────────────────────────────────────────────┐
│              HTTP Response                      │
│    { user: {...}, orders: [...] }               │
└─────────────────────────────────────────────────┘
```

## Next Steps

- **[Getting Started](./getting-started.md)**: Install and run your first Web Pipe application
- **[DSL Syntax](./dsl-syntax.md)**: Complete language reference
- **[Routes & Pipelines](./routes-and-pipelines.md)**: Learn about composable pipelines
- **[GraphQL](./graphql.md)**: Build GraphQL APIs with Web Pipe

## Philosophy

Web Pipe is inspired by functional programming and Unix pipes. Like Unix commands that transform text streams, Web Pipe middleware transforms JSON through a pipeline. The goal is to make API development:

1. **Simple**: Express complex logic through composition
2. **Declarative**: Describe transformations, not control flow
3. **Testable**: Built-in mocking and BDD testing
4. **Fast**: Async execution and optimized runtime
5. **Maintainable**: Small, focused middleware steps

Web Pipe is not trying to replace general-purpose programming languages—it's a specialized tool for a specific domain: building HTTP APIs that transform and aggregate data.
