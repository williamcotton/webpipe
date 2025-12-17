# pg Middleware

Execute SQL against PostgreSQL using sqlx with parameter binding and connection pooling.

---

## Overview

The `pg` middleware enables SQL queries against PostgreSQL databases. It provides automatic parameter binding, connection pooling, and flexible result naming.

---

## Configuration

Configure PostgreSQL connection in your Web Pipe file:

```wp
config pg {
  host: $WP_PG_HOST || "localhost"
  port: $WP_PG_PORT || "5432"
  database: $WP_PG_DATABASE || "mydb"
  user: $WP_PG_USER || "postgres"
  password: $WP_PG_PASSWORD || "postgres"
  ssl: false
  initialPoolSize: 10
  maxPoolSize: 20
}
```

---

## Basic Usage

### Simple Query

Execute a SELECT query:

```wp
GET /teams
  |> jq: `{ sqlParams: [] }`
  |> pg: `SELECT * FROM teams`
```

**Output** (stored in `.data`):
```json
{
  "rows": [
    { "id": 1, "name": "Platform" },
    { "id": 2, "name": "Growth" }
  ],
  "rowCount": 2
}
```

### Query with Parameters (Traditional)

Use `sqlParams` array for parameter binding:

```wp
GET /users/:id
  |> jq: `{ sqlParams: [.params.id] }`
  |> pg: `SELECT * FROM users WHERE id = $1`
```

### Query with Inline Array Parameters (Modern)

Pass parameters directly using inline array syntax:

```wp
GET /users/:id
  |> pg([.params.id]): `SELECT * FROM users WHERE id = $1`
```

**Benefits of inline syntax:**
- More concise (no intermediate JQ step)
- Clear parameter mapping
- Automatic SQL injection protection

---

## Inline Array Parameters

### Single Parameter

```wp
|> pg([.userId]): `SELECT * FROM users WHERE id = $1`
```

### Multiple Parameters

```wp
|> pg([.userId, .status]): `
  SELECT * FROM users
  WHERE id = $1 AND status = $2
`
```

### Extracting from Nested Objects

```wp
POST /orders
  |> pg([.body.userId, .body.productId, .body.quantity]): `
    INSERT INTO orders (user_id, product_id, quantity)
    VALUES ($1, $2, $3)
    RETURNING *
  `
```

### With Type Conversions

```wp
|> pg([(.params.id | tonumber), (.query.limit // 10)]): `
  SELECT * FROM items
  WHERE id = $1
  LIMIT $2
`
```

---

## Named Results

### Using resultName (Legacy)

Store query results under a specific name:

```wp
GET /user/:id/profile
  |> jq: `{ sqlParams: [.params.id], resultName: "user" }`
  |> pg: `SELECT * FROM users WHERE id = $1`

  |> jq: `{ sqlParams: [.data.user.rows[0].company_id], resultName: "company" }`
  |> pg: `SELECT * FROM companies WHERE id = $1`

  |> jq: `{
    userName: .data.user.rows[0].name,
    companyName: .data.company.rows[0].name
  }`
```

### Using @result Tag (Modern)

Use the `@result(name)` tag for cleaner result naming:

```wp
GET /user/:id/profile
  |> pg([.params.id]): `SELECT * FROM users WHERE id = $1` @result(user)
  |> pg([.data.user.rows[0].company_id]): `SELECT * FROM companies WHERE id = $1` @result(company)

  |> jq: `{
    userName: .data.user.rows[0].name,
    companyName: .data.company.rows[0].name
  }`
```

### Variable Auto-Naming

When using a pg variable, results are automatically named:

```wp
pg getActiveUsers = `SELECT * FROM users WHERE status = 'active'`

GET /users
  |> pg: getActiveUsers
  |> jq: `{ users: .data.getActiveUsers.rows }`
```

---

## Multiple Queries with Named Results

### Dashboard Pattern

Collect multiple datasets:

```wp
GET /dashboard/summary
  |> pg: `SELECT * FROM departments` @result(departments)
  |> pg: `SELECT * FROM employees` @result(employees)
  |> pg: `SELECT * FROM projects WHERE status = 'active'` @result(projects)

  |> jq: `{
    totalDepartments: .data.departments.rowCount,
    totalEmployees: .data.employees.rowCount,
    totalProjects: .data.projects.rowCount,
    departments: .data.departments.rows,
    employees: .data.employees.rows,
    projects: .data.projects.rows
  }`
```

### Related Data Loading

Load related entities sequentially:

```wp
GET /posts/:id/full
  |> pg([.params.id]): `SELECT * FROM posts WHERE id = $1` @result(post)
  |> pg([.data.post.rows[0].author_id]): `SELECT * FROM users WHERE id = $1` @result(author)
  |> pg([.params.id]): `SELECT * FROM comments WHERE post_id = $1` @result(comments)

  |> jq: `{
    post: .data.post.rows[0],
    author: .data.author.rows[0],
    comments: .data.comments.rows,
    commentCount: .data.comments.rowCount
  }`
```

---

## INSERT, UPDATE, DELETE

### INSERT with RETURNING

```wp
POST /users
  |> pg([.body.name, .body.email, .body.age]): `
    INSERT INTO users (name, email, age)
    VALUES ($1, $2, $3)
    RETURNING *
  `
  |> jq: `{ user: .data.rows[0], created: true }`
```

### UPDATE

```wp
PUT /users/:id
  |> pg([.body.name, .body.email, .params.id]): `
    UPDATE users
    SET name = $1, email = $2
    WHERE id = $3
    RETURNING *
  `
  |> jq: `{ user: .data.rows[0], updated: true }`
```

### DELETE

```wp
DELETE /users/:id
  |> pg([.params.id]): `DELETE FROM users WHERE id = $1`
  |> jq: `{ deleted: true, count: .data.rowCount }`
```

---

## Output Structure

### SELECT Queries

```json
{
  "data": {
    "rows": [
      { "id": 1, "name": "Alice" },
      { "id": 2, "name": "Bob" }
    ],
    "rowCount": 2
  }
}
```

### INSERT/UPDATE/DELETE

```json
{
  "data": {
    "rows": [],
    "rowCount": 1
  }
}
```

### With RETURNING

```json
{
  "data": {
    "rows": [
      { "id": 42, "name": "New User" }
    ],
    "rowCount": 1
  }
}
```

### With Named Results

```json
{
  "data": {
    "users": {
      "rows": [{ "id": 1, "name": "Alice" }],
      "rowCount": 1
    },
    "posts": {
      "rows": [{ "id": 100, "title": "Post 1" }],
      "rowCount": 1
    }
  }
}
```

---

## Error Handling

### SQL Errors

SQL errors are captured in the `.errors` array:

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

### Handling in Result Blocks

```wp
|> pg([.userId]): `SELECT * FROM users WHERE id = $1`
|> result
  ok(200):
    |> jq: `{ user: .data.rows[0] }`
  sqlError(500):
    |> jq: `{
      error: "Database error",
      message: .errors[0].message,
      query: .errors[0].query
    }`
```

---

## Advanced Patterns

### Conditional Queries

Execute different queries based on conditions:

```wp
GET /data
  |> dispatch
    case @env(production):
      |> pg: `SELECT * FROM prod_data` @result(data)
    case @env(development):
      |> pg: `SELECT * FROM dev_data` @result(data)
```

### Dynamic WHERE Clauses

Build dynamic queries based on input:

```wp
GET /users/search
  |> jq: `{
    filters: [],
    params: []
  } |
  if .query.name then
    .filters += ["name ILIKE $1"] | .params += [.query.name + "%"]
  else . end |
  if .query.status then
    .filters += ["status = $" + (.params | length + 1 | tostring)] | .params += [.query.status]
  else . end`

  |> pg(.params): `
    SELECT * FROM users
    WHERE ` + (.filters | join(" AND "))
```

### Transactions Pattern

Use multiple queries with proper error handling:

```wp
POST /transfer
  |> pg([.body.fromAccount, .body.amount]): `
    UPDATE accounts SET balance = balance - $2 WHERE id = $1
    RETURNING *
  ` @result(debit)

  |> pg([.body.toAccount, .body.amount]): `
    UPDATE accounts SET balance = balance + $2 WHERE id = $1
    RETURNING *
  ` @result(credit)

  |> result
    ok(200):
      |> jq: `{
        success: true,
        from: .data.debit.rows[0],
        to: .data.credit.rows[0]
      }`
    sqlError(500):
      |> jq: `{ error: "Transaction failed", details: .errors }`
```

---

## Integration with Other Middleware

### With Validation

```wp
POST /users
  |> validate: `{
    name: string(3..100),
    email: email,
    age: number
  }`
  |> result
    ok(200):
      |> pg([.body.name, .body.email, .body.age]): `
        INSERT INTO users (name, email, age)
        VALUES ($1, $2, $3)
        RETURNING *
      `
    validationError(400):
      |> jq: `{ error: "Validation failed", details: .errors }`
```

### With Authentication

```wp
GET /users/:id
  |> auth: "required"
  |> result
    ok(200):
      |> pg([.params.id]): `SELECT * FROM users WHERE id = $1`
    authRequired(401):
      |> jq: `{ error: "Authentication required" }`
```

### With Caching

```wp
GET /popular-posts
  |> cache: `ttl: 300, keyTemplate: "popular-posts"`
  |> pg: `
    SELECT * FROM posts
    ORDER BY views DESC
    LIMIT 10
  `
```

---

## Best Practices

### 1. Always Use Parameter Binding

**Good:**
```wp
|> pg([.userId]): `SELECT * FROM users WHERE id = $1`
```

**Avoid (SQL injection risk):**
```wp
|> pg: `SELECT * FROM users WHERE id = ` + .userId
```

### 2. Use @result for Multiple Queries

When executing multiple queries, name your results:

```wp
|> pg([.id]): `SELECT * FROM users WHERE id = $1` @result(user)
|> pg([.id]): `SELECT * FROM orders WHERE user_id = $1` @result(orders)
```

### 3. Check rowCount for Empty Results

```wp
|> pg([.id]): `SELECT * FROM users WHERE id = $1`
|> jq: `
  if .data.rowCount == 0 then
    { error: "User not found", status: 404 }
  else
    { user: .data.rows[0] }
  end
`
```

### 4. Use RETURNING for INSERT/UPDATE

Always use RETURNING to get the created/updated row:

```wp
|> pg([.body.name]): `
  INSERT INTO users (name) VALUES ($1) RETURNING *
`
```

### 5. Prefer Inline Args for Conciseness

**Prefer:**
```wp
|> pg([.userId, .status]): `SELECT * FROM users WHERE id = $1 AND status = $2`
```

**Over:**
```wp
|> jq: `{ sqlParams: [.userId, .status] }`
|> pg: `SELECT * FROM users WHERE id = $1 AND status = $2`
```

---

## See Also

- [Inline Arguments](../inline-arguments.md) - Passing parameters with inline syntax
- [Routes & Pipelines](../routes-and-pipelines.md) - Named results with @result
- [Result Routing](../result-routing.md) - Error handling with result blocks
- [DSL Syntax](../dsl-syntax.md) - Complete syntax reference
