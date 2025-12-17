# Inline Arguments

Inline arguments allow you to pass dynamic data directly to middleware calls, making pipelines more concise and expressive. Instead of using JQ steps to set intermediate variables, you can compute values inline within the middleware invocation.

## Overview

**Traditional Approach:**
```wp
|> jq: `{ sqlParams: [.userId, "active"] }`
|> pg: `SELECT * FROM users WHERE id = $1 AND status = $2`
```

**Inline Argument Approach:**
```wp
|> pg([.userId, "active"]): `SELECT * FROM users WHERE id = $1 AND status = $2`
```

Inline arguments reduce boilerplate and keep the data flow more visible in the pipeline.

---

## Basic Syntax

### Parentheses Syntax

Use parentheses `()` to pass inline arguments:

```wp
|> middleware(argument)
|> middleware(argument1, argument2)
```

### Array Parameters

Pass array parameters using square brackets `[...]`:

```wp
|> pg([.userId, .status]): `SELECT * FROM users WHERE id = $1 AND status = $2`
```

### Object Parameters

Pass object parameters using curly braces `{...}`:

```wp
|> graphql({ userId: .targetId, limit: 10 }): `query($userId: ID!, $limit: Int) { ... }`
```

### String Arguments

Pass string literals or JQ string expressions:

```wp
|> fetch("https://api.example.com/users/" + (.userId | tostring))
|> log("Processing user request")
```

---

## Array Parameters

Array parameters are primarily used with the `pg` middleware for SQL parameter binding.

### Simple Array Parameter

```wp
GET /users/:id
  |> jq: `{ id: (.params.id | tonumber) }`
  |> pg([.id]): `SELECT * FROM users WHERE id = $1`
```

### Multiple Parameters

```wp
GET /users/search
  |> jq: `{
    status: .query.status,
    role: .query.role
  }`
  |> pg([.status, .role]): `
    SELECT * FROM users
    WHERE status = $1 AND role = $2
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

### Mixed Types

Arrays can contain different JQ expressions:

```wp
|> pg([(.params.id | tonumber), .query.status, (.body.limit // 10)]): `
  SELECT * FROM items
  WHERE id = $1 AND status = $2
  LIMIT $3
`
```

---

## Dynamic String Arguments

Build URLs or strings dynamically using JQ expressions.

### Fetch with Dynamic URLs

```wp
GET /api/users/:id/profile
  |> jq: `{ userId: (.params.id | tostring) }`
  |> fetch("https://api.example.com/users/" + .userId)
```

### Complex URL Building

```wp
GET /external/:resource/:id
  |> jq: `{
    baseUrl: "https://api.example.com",
    resource: .params.resource,
    id: .params.id,
    apiKey: $ENV.API_KEY
  }`
  |> fetch(.baseUrl + "/" + .resource + "/" + (.id | tostring) + "?key=" + .apiKey)
```

### Query Parameters

```wp
GET /search
  |> jq: `{
    term: .query.q,
    limit: (.query.limit // 10 | tostring)
  }`
  |> fetch("https://api.example.com/search?q=" + .term + "&limit=" + .limit)
```

---

## Object Parameters

Object parameters are used with `graphql` middleware for passing variables.

### Basic GraphQL Variables

```wp
GET /gql/user/:id/todos
  |> jq: `{ targetId: (.params.id | tonumber) }`
  |> graphql({ userId: .targetId }): `
    query($userId: ID!) {
      user(id: $userId) {
        name
        email
      }
      posts(limit: 10) {
        id
        title
        body
      }
    }
  `
```

### Multiple Variables

```wp
GET /gql/posts
  |> jq: `{
    authorId: (.query.author | tonumber),
    pageSize: (.query.limit // 10 | tonumber),
    offset: (.query.offset // 0 | tonumber)
  }`
  |> graphql({
    author: .authorId,
    limit: .pageSize,
    offset: .offset
  }): `
    query($author: ID!, $limit: Int!, $offset: Int!) {
      posts(authorId: $author, limit: $limit, offset: $offset) {
        id
        title
        body
        createdAt
      }
    }
  `
```

### Complex Objects

```wp
POST /gql/create-user
  |> jq: `{
    userData: {
      name: .body.name,
      email: .body.email,
      role: .body.role // "user"
    }
  }`
  |> graphql({ input: .userData }): `
    mutation($input: UserInput!) {
      createUser(input: $input) {
        id
        name
        email
        createdAt
      }
    }
  `
```

---

## Middleware-Specific Patterns

### PostgreSQL (pg)

The `pg` middleware accepts array parameters for SQL binding:

```wp
|> pg([.userId, .status]): `SELECT * FROM users WHERE id = $1 AND status = $2`
```

**Benefits:**
- Automatic SQL injection protection
- Type coercion handled by sqlx
- Clear parameter mapping

### Fetch

The `fetch` middleware accepts string URLs:

```wp
|> fetch("https://api.example.com/data")
|> fetch(.customUrl)
|> fetch("https://api.example.com/users/" + (.userId | tostring))
```

**Additional Options:**
You can still use the traditional approach for complex configurations:
```wp
|> jq: `{
  fetchUrl: "https://api.example.com/data",
  fetchMethod: "POST",
  fetchBody: { key: "value" },
  fetchHeaders: { "Authorization": "Bearer " + .token }
}`
|> fetch: `_`
```

### GraphQL

The `graphql` middleware accepts object parameters for variables:

```wp
|> graphql({ userId: .id, limit: 10 }): `query($userId: ID!, $limit: Int!) { ... }`
```

**Benefits:**
- Clearer variable mapping
- Type safety through GraphQL schema
- Less boilerplate than setting `graphqlParams`

### Log

The `log` middleware accepts string messages:

```wp
|> log("Starting user registration")
|> log("Processing order for user " + (.userId | tostring))
```

### Cache

The `cache` middleware accepts string or object configuration:

```wp
|> cache: `ttl: 300, enabled: true`
|> cache({ ttl: 300, keyTemplate: "user-" + (.userId | tostring) })
```

### Rate Limit

The `rateLimit` middleware accepts configuration objects:

```wp
|> rateLimit({
  keyTemplate: "user-" + (.userId | tostring),
  limit: 100,
  window: 3600
})
```

### Loader (GraphQL DataLoaders)

The `loader` middleware is used exclusively in GraphQL field resolvers to batch data loading and prevent N+1 queries. It accepts:
1. A JQ expression to extract the key from the parent object
2. Optional object for field arguments

**Basic Usage:**
```wp
resolver Team.employees =
  |> loader(.parent.id): EmployeesByTeamLoader
```

**With Field Arguments:**
```wp
resolver Team.employees =
  |> loader(.parent.id, { limit: .args.limit, offset: .args.offset }): EmployeesByTeamLoader
```

**How it works:**
- The loader batches keys from multiple parent objects
- The loader pipeline receives: `{ keys: [1, 2, 3], args: {...}, context: {...} }`
- The pipeline must return a map: `{ "1": [...], "2": [...], "3": [...] }`

**Example Loader Pipeline:**
```wp
pipeline EmployeesByTeamLoader =
  |> pg([(.keys | map(tostring) | join(",")), (.args.limit // 10)]): `!raw
    SELECT json_object_agg(sub.team_id, sub.employees)
    FROM (
      SELECT team_id, json_agg(...) AS employees
      FROM employees
      WHERE team_id::text = ANY(string_to_array($1, ','))
      GROUP BY team_id
    ) sub
  `
```

See [GraphQL Middleware](./middleware/graphql.md#dataloaders) for complete DataLoader documentation.

---

## Comparison: Inline vs Traditional

### Traditional Two-Step Approach

**Setting up SQL parameters:**
```wp
GET /users/:id/orders
  |> jq: `{ sqlParams: [.params.id] }`
  |> pg: `SELECT * FROM users WHERE id = $1`
  |> jq: `{ sqlParams: [.data.rows[0].id] }`
  |> pg: `SELECT * FROM orders WHERE user_id = $1`
```

**Pros:**
- Explicit and clear what's being set
- Easier to debug intermediate state

**Cons:**
- Verbose with repeated `jq` steps
- Harder to follow data flow
- More lines of code

### Inline Argument Approach

**Same logic with inline arguments:**
```wp
GET /users/:id/orders
  |> pg([.params.id]): `SELECT * FROM users WHERE id = $1`
  |> pg([.data.rows[0].id]): `SELECT * FROM orders WHERE user_id = $1`
```

**Pros:**
- Concise and readable
- Clear data flow
- Less boilerplate

**Cons:**
- Slightly less explicit
- May be harder to debug complex expressions

---

## When to Use Inline Arguments

### Use Inline Arguments When:

1. **Simple Parameter Extraction:**
   ```wp
   |> pg([.userId, .status]): `SELECT * FROM users WHERE id = $1 AND status = $2`
   ```

2. **Direct URL Building:**
   ```wp
   |> fetch("https://api.example.com/users/" + (.id | tostring))
   ```

3. **Straightforward Variable Mapping:**
   ```wp
   |> graphql({ userId: .targetId }): `query($userId: ID!) { ... }`
   ```

### Use Traditional Approach When:

1. **Complex Data Transformations:**
   ```wp
   |> jq: `{
     sqlParams: [
       (.params.id | tonumber),
       (.query.status // "active"),
       (.body.filters | keys | map(. + ":" + .value) | join(","))
     ]
   }`
   |> pg: `SELECT * FROM complex_query ...`
   ```

2. **Reusing Computed Values:**
   ```wp
   |> jq: `{
     normalizedEmail: (.body.email | ascii_downcase | ltrimstr | rtrimstr),
     hashedPassword: (.body.password | hash("sha256"))
   }`
   |> pg([.normalizedEmail, .hashedPassword]): `
     INSERT INTO users (email, password_hash) VALUES ($1, $2)
   `
   ```

3. **Debugging Complex Logic:**
   ```wp
   |> jq: `{ intermediate: .complex | calculation }`
   |> log: `level: debug`
   |> pg([.intermediate.value]): `SELECT * FROM ...`
   ```

---

## Best Practices

### 1. Keep Expressions Simple

**Good:**
```wp
|> pg([.userId, .status]): `SELECT * FROM users WHERE id = $1 AND status = $2`
```

**Avoid:**
```wp
|> pg([
  (.userId | tonumber // 0),
  (if .status == "premium" then "active" else "pending" end),
  (.metadata | keys | map(. + "=" + (.value | tostring)) | join("&"))
]): `SELECT * FROM ...`
```

For complex expressions, use a JQ step first to compute the values, then use inline arguments to pass them.

### 2. Use Type Conversions When Needed

```wp
|> pg([(.params.id | tonumber)]): `SELECT * FROM users WHERE id = $1`
|> fetch("https://api.example.com/users/" + (.userId | tostring))
```

### 3. Provide Defaults for Optional Parameters

```wp
|> pg([.userId, (.query.limit // 10)]): `SELECT * FROM items WHERE user_id = $1 LIMIT $2`
```

### 4. Combine with @result Tag

```wp
|> pg([.userId]): `SELECT * FROM users WHERE id = $1` @result(user)
|> pg([.data.user.rows[0].dept_id]): `SELECT * FROM departments WHERE id = $1` @result(dept)
```

### 5. Use Parentheses for Clarity

When expressions are complex, use parentheses:

```wp
|> pg([(.params.id | tonumber), (.query.status // "active")]): `SELECT * FROM ...`
```

---

## Advanced Patterns

### Chaining Inline Arguments

```wp
GET /users/:id/full-profile
  |> pg([.params.id]): `SELECT * FROM users WHERE id = $1` @result(user)
  |> pg([.data.user.rows[0].company_id]): `SELECT * FROM companies WHERE id = $1` @result(company)
  |> pg([.params.id]): `SELECT * FROM orders WHERE user_id = $1` @result(orders)
  |> jq: `{
    user: .data.user.rows[0],
    company: .data.company.rows[0],
    orders: .data.orders.rows
  }`
```

### Conditional Parameters

```wp
GET /search
  |> jq: `{
    filters: (if .query.category then [.query.category] else [] end)
  }`
  |> pg([.query.term] + .filters): `
    SELECT * FROM products
    WHERE name LIKE $1
    ${if .filters | length > 0 then " AND category = $2" else "" end}
  `
```

### Dynamic Middleware Selection

```wp
GET /data/:source/:id
  |> dispatch
    case @env(production):
      |> fetch("https://prod-api.example.com/" + .params.source + "/" + .params.id)
    case @env(development):
      |> fetch("https://dev-api.example.com/" + .params.source + "/" + .params.id)
```

---

## Error Handling

### Invalid Parameter Types

If inline arguments don't match the expected type, middleware will fail with an error:

```wp
# This will fail if .userId is not a number
|> pg([.userId]): `SELECT * FROM users WHERE id = $1`
```

**Solution:** Use type conversion:
```wp
|> pg([(.userId | tonumber)]): `SELECT * FROM users WHERE id = $1`
```

### Missing Required Parameters

If a parameter is missing or null, the middleware may fail:

```wp
# This will fail if .userId is null
|> pg([.userId]): `SELECT * FROM users WHERE id = $1`
```

**Solution:** Provide defaults or validate first:
```wp
|> pg([(.userId // 0)]): `SELECT * FROM users WHERE id = $1`
```

Or use validation middleware:
```wp
|> validate: `{ userId: number }`
|> pg([.userId]): `SELECT * FROM users WHERE id = $1`
```

---

## See Also

- [pg Middleware](./middleware/pg.md) - PostgreSQL integration with parameter binding
- [fetch Middleware](./middleware/fetch.md) - HTTP requests with dynamic URLs
- [graphql Middleware](./middleware/graphql.md) - GraphQL queries with inline variables
- [DSL Syntax](./dsl-syntax.md) - Complete syntax reference
- [Routes & Pipelines](./routes-and-pipelines.md) - Pipeline composition patterns
