# Native GraphQL Support

WebPipe 2.0 introduces a fully integrated GraphQL runtime, allowing you to define schemas, map resolvers to pipelines, and execute queries efficiently.

## Defining a Schema

Use the `graphqlSchema` block in your `.wp` files to define your GraphQL schema using standard Schema Definition Language (SDL).

```graphql
graphqlSchema = `
  type User {
    id: ID!
    name: String!
    email: String!
  }

  type Query {
    users: [User!]!
    user(id: ID!): User
  }

  type Mutation {
    createUser(name: String!, email: String!): User!
  }
`
```

## Mapping Resolvers

Resolvers are mapped to WebPipe pipelines using the `query` and `mutation` keywords. Each resolver pipeline receives the GraphQL arguments and parent object in its input.

### Query Resolvers

```wp
# Resolver for Query.users
query users =
  |> pg: `SELECT * FROM users`
  |> jq: `.data.rows`

# Resolver for Query.user(id: ID!)
query user =
  |> jq: `{ sqlParams: [.id] }`
  |> pg: `SELECT * FROM users WHERE id = $1`
  |> jq: `.data.rows[0]`
```

### Mutation Resolvers

```wp
# Resolver for Mutation.createUser
mutation createUser =
  |> auth: "required"
  |> jq: `{ sqlParams: [.name, .email] }`
  |> pg: `INSERT INTO users (name, email) VALUES ($1, $2) RETURNING *`
  |> jq: `.data.rows[0]`
```

## Execution Pipeline

To expose your GraphQL API, use the `graphql` middleware in a route. This middleware executes the query against your defined schema and resolvers.

```wp
POST /graphql
  |> auth: "optional"
  |> graphql: `query` # The config here is usually dynamic or passed from input
```

Typically, you'll want to accept the query from the HTTP request body:

```wp
POST /graphql
  |> auth: "optional"
  # Extract query and variables from request body
  |> jq: `.graphqlParams = .body` 
  # Execute GraphQL
  |> graphql: .body.query
```

*(Note: The actual `graphql` middleware implementation handles the execution details. The example above simplifies the common pattern.)*

## Advanced Usage

### Using @result Tags

Use `@result` tags for cleaner result naming:

```wp
GET /dashboard
  |> graphql: `query { todos { id title } }` @result(todos)
  |> graphql: `query { currentUser { name email } }` @result(user)
  |> jq: `{
    todos: .data.todos.data.todos,
    user: .data.user.data.currentUser
  }`
```

### Async GraphQL Queries

Execute multiple GraphQL queries in parallel:

```wp
GET /async-dashboard
  |> graphql: `query { users { id name } }` @async(usersQuery) @result(users)
  |> graphql: `query { stats { total active } }` @async(statsQuery) @result(stats)
  |> graphql: `query { posts(limit: 5) { id title } }` @async(postsQuery) @result(posts)

  |> join: `usersQuery, statsQuery, postsQuery`

  |> jq: `{
    users: .async.usersQuery.data.users.data.users,
    stats: .async.statsQuery.data.stats.data.stats,
    posts: .async.postsQuery.data.posts.data.posts
  }`
```

---

## Advanced Resolver Patterns

### Resolvers with @async

Execute async operations within GraphQL resolvers:

```wp
query userWithPosts =
  # Fetch user from database
  |> jq: `{ sqlParams: [.id] }`
  |> pg: `SELECT * FROM users WHERE id = $1` @result(user)

  # Fetch user's posts in parallel
  |> pg: `SELECT * FROM posts WHERE user_id = $1` @async(posts) @result(userPosts)

  # Wait for posts query
  |> join: `posts`

  # Combine results
  |> jq: `
    .data.user.rows[0] + {
      posts: .async.posts.data.userPosts.rows
    }
  `
```

### Resolvers with @guard

Conditionally execute resolver logic:

```wp
query adminData =
  # Only execute if user is admin
  |> pg: `SELECT * FROM sensitive_data` @guard(`.user.role == "admin"`)
  |> jq: `.data.rows`

query regularData =
  # Always executes
  |> pg: `SELECT * FROM public_data`
  |> jq: `.data.rows`
```

### Resolvers with @when

Use dynamic conditions in resolvers:

```wp
query conditionalData =
  |> lua: `
    setWhen("is_premium", request.user and request.user.premium == true)
    return request
  `
  |> dispatch
    case @when(is_premium):
      |> pg: `SELECT * FROM premium_content`
    default:
      |> pg: `SELECT * FROM basic_content`
  |> jq: `.data.rows`
```

### Nested Field Resolvers

Resolve nested fields with separate queries:

```wp
# Parent resolver
query user =
  |> jq: `{ sqlParams: [.id] }`
  |> pg: `SELECT * FROM users WHERE id = $1`
  |> jq: `.data.rows[0]`

# Field resolver for user.posts
query userPosts =
  # .parent contains the parent User object
  |> jq: `{ sqlParams: [.parent.id] }`
  |> pg: `SELECT * FROM posts WHERE user_id = $1`
  |> jq: `.data.rows`

# Field resolver for user.company
query userCompany =
  |> jq: `{ sqlParams: [.parent.company_id] }`
  |> pg: `SELECT * FROM companies WHERE id = $1`
  |> jq: `.data.rows[0]`
```

### Mutation Resolvers with Validation

```wp
mutation createPost =
  # Validate input
  |> validate: `{
    title: string(3..200),
    body: string(10..10000),
    userId: number
  }`
  |> result
    ok(200):
      # Insert post
      |> pg([.title, .body, .userId]): `
        INSERT INTO posts (title, body, user_id)
        VALUES ($1, $2, $3)
        RETURNING *
      `
      |> jq: `.data.rows[0]`
    validationError(400):
      # Return validation error to GraphQL
      |> jq: `{ errors: .errors }`
```

### Resolver with Authentication

```wp
mutation deleteUser =
  |> auth: "required"
  |> result
    ok(200):
      # Check if user is admin
      |> lua: `
        setWhen("is_admin", request.user.role == "admin")
        return request
      `
      |> dispatch
        case @when(is_admin):
          |> pg([.id]): `DELETE FROM users WHERE id = $1`
          |> jq: `{ success: true, deletedId: .id }`
        default:
          |> jq: `{ errors: [{ message: "Unauthorized" }] }`
    authRequired(401):
      |> jq: `{ errors: [{ message: "Authentication required" }] }`
```

---

## Complex Resolver Scenarios

### Aggregating Multiple Data Sources

```wp
query dashboard =
  # Fetch from database
  |> pg: `SELECT * FROM users` @async(dbUsers) @result(users)

  # Fetch from external API
  |> fetch("https://api.example.com/analytics") @async(analytics) @result(stats)

  # Execute another GraphQL query
  |> graphql: `query { posts { id title } }` @async(postsQuery) @result(posts)

  # Wait for all
  |> join: `dbUsers, analytics, postsQuery`

  # Aggregate results
  |> jq: `{
    users: .async.dbUsers.data.users.rows,
    analytics: .async.analytics.data.stats.response,
    posts: .async.postsQuery.data.posts.data.posts
  }`
```

### Conditional Field Resolution

```wp
query userProfile =
  # Fetch basic user data
  |> jq: `{ sqlParams: [.id] }`
  |> pg: `SELECT * FROM users WHERE id = $1` @result(user)

  # Conditionally fetch sensitive data
  |> lua: `
    setWhen("is_owner", request.user and request.user.id == request.id)
    return request
  `

  # Only fetch sensitive data if user is viewing their own profile
  |> pg([.id]): `SELECT email, phone FROM user_private WHERE user_id = $1` @when(is_owner) @result(private)

  # Combine results
  |> jq: `
    .data.user.rows[0] +
    (if .data.private then { email: .data.private.rows[0].email, phone: .data.private.rows[0].phone } else {} end)
  `
```

### Batched Resolver

```wp
query batchUsers =
  # Receives array of IDs in .ids
  |> jq: `{ sqlParams: .ids }`
  |> pg: `SELECT * FROM users WHERE id = ANY($1::int[])`
  |> jq: `.data.rows`
```

---

## Error Handling in Resolvers

### Graceful Error Handling

```wp
query userOrNull =
  |> jq: `{ sqlParams: [.id] }`
  |> pg: `SELECT * FROM users WHERE id = $1`
  |> result
    ok(200):
      |> jq: `
        if .data.rowCount > 0 then
          .data.rows[0]
        else
          null
        end
      `
    sqlError(500):
      |> jq: `{ errors: [{ message: "Database error", details: .errors }] }`
```

### Returning GraphQL Errors

```wp
mutation riskyOperation =
  |> pg([.input]): `INSERT INTO data VALUES ($1) RETURNING *`
  |> result
    ok(200):
      |> jq: `.data.rows[0]`
    sqlError(500):
      # Return error in GraphQL error format
      |> jq: `{
        errors: [{
          message: .errors[0].message,
          extensions: {
            code: "DATABASE_ERROR",
            sqlstate: .errors[0].sqlstate
          }
        }]
      }`
```

---

## Performance Optimization

### Caching Resolver Results

```wp
query expensiveStats =
  |> cache: `ttl: 300, keyTemplate: "stats-global"`
  |> pg: `
    SELECT
      COUNT(*) as total_users,
      AVG(age) as avg_age,
      MAX(created_at) as last_signup
    FROM users
  `
  |> jq: `.data.rows[0]`
```

### Parallel Field Resolution

Instead of sequential field resolution, use async:

```wp
query richUserProfile =
  |> jq: `{ userId: .id }`

  # Fetch all related data in parallel
  |> pg([.userId]): `SELECT * FROM users WHERE id = $1` @async(user) @result(userData)
  |> pg([.userId]): `SELECT * FROM posts WHERE user_id = $1` @async(posts) @result(userPosts)
  |> pg([.userId]): `SELECT * FROM comments WHERE user_id = $1` @async(comments) @result(userComments)

  |> join: `user, posts, comments`

  # Combine all data
  |> jq: `
    .async.user.data.userData.rows[0] + {
      posts: .async.posts.data.userPosts.rows,
      comments: .async.comments.data.userComments.rows,
      postCount: .async.posts.data.userPosts.rowCount,
      commentCount: .async.comments.data.userComments.rowCount
    }
  `
```

---

## Best Practices

### 1. Use @result for Named Results

Always name your resolver results:

```wp
query user =
  |> pg: `SELECT * FROM users WHERE id = $1` @result(user)
  |> jq: `.data.user.rows[0]`
```

### 2. Leverage Async for Independent Queries

When resolver needs multiple data sources:

```wp
query dashboard =
  |> pg: `SELECT * FROM users` @async(users)
  |> pg: `SELECT * FROM posts` @async(posts)
  |> join: `users, posts`
```

### 3. Validate Mutation Input

Always validate input in mutation resolvers:

```wp
mutation createUser =
  |> validate: `{ name: string(3..100), email: email }`
  |> result
    ok(200):
      # ... mutation logic ...
```

### 4. Use @guard for Access Control

Protect sensitive resolvers with guards:

```wp
query adminStats =
  |> pg: `SELECT * FROM admin_stats` @guard(`.user.role == "admin"`)
```

### 5. Return Proper GraphQL Errors

Format errors correctly for GraphQL:

```wp
|> result
  sqlError(500):
    |> jq: `{ errors: [{ message: .errors[0].message }] }`
```

---

## DataLoaders (N+1 Query Prevention)

DataLoaders solve the N+1 query problem in GraphQL by batching multiple resolver calls into a single database query.

### The N+1 Problem

Without DataLoaders, fetching nested data causes N+1 queries:

```wp
# Without DataLoader: 1 + N queries
# - 1 query to fetch all teams
# - N queries to fetch employees for each team (one per team)

query teams =
  |> pg: `SELECT * FROM teams`
  |> jq: `.data.rows`

query employees =
  # Called once per team! (N+1 problem)
  |> pg([.teamId]): `SELECT * FROM employees WHERE team_id = $1`
  |> jq: `.data.rows`
```

**Problem:** Fetching 10 teams with employees = 11 database queries (1 for teams, 10 for employees)

### DataLoader Solution

Use the `loader` middleware to batch resolver calls:

```wp
# Define a loader pipeline that handles batched requests
pipeline EmployeesByTeamLoader =
  # Receives:
  # - .keys: [1, 2, 3] (batched team IDs)
  # - .args: {limit: 5} (GraphQL field arguments)
  # - .context: {...} (request context)

  |> pg([(.keys | map(tostring) | join(",")), (.args.limit // 10)]): `!raw
    SELECT
      json_object_agg(
        sub.team_id,
        sub.employees
      )
    FROM (
      SELECT
        team_id,
        json_agg(
          json_build_object(
            'id', id::text,
            'name', name,
            'email', email,
            'teamId', team_id::text
          )
        ) AS employees
      FROM (
        SELECT
          id,
          name,
          email,
          team_id,
          ROW_NUMBER() OVER (
            PARTITION BY team_id
            ORDER BY id
          ) AS rn
        FROM employees
        WHERE team_id::text = ANY(string_to_array($1, ','))
      ) ranked
      WHERE ranked.rn <= $2
      GROUP BY team_id
    ) sub
  `

# Use loader in nested resolver
resolver Team.employees =
  |> loader(.parent.id): EmployeesByTeamLoader
```

**Solution:** Fetching 10 teams with employees = 2 database queries (1 for teams, 1 batched for all employees)

### Loader Pipeline Contract

A loader pipeline must:

1. **Accept batched keys** in `.keys` array
2. **Return a keyed object** mapping each key to its result
3. **Handle field arguments** from `.args`
4. **Access request context** via `.context`

**Input Structure:**
```json
{
  "keys": [1, 2, 3],
  "args": { "limit": 5 },
  "context": { "user": {...}, "headers": {...} }
}
```

**Output Structure:**
```json
{
  "1": [{ "id": 10, "name": "Alice" }],
  "2": [{ "id": 20, "name": "Bob" }],
  "3": []
}
```

### Using Loaders in Resolvers

The `loader` middleware syntax:

```wp
resolver Parent.field =
  |> loader(<key-expression>): <loader-pipeline>
```

**Example:**
```wp
resolver Team.employees =
  |> loader(.parent.id): EmployeesByTeamLoader
```

**How it works:**
1. GraphQL resolver is called for each team
2. Loader collects all `.parent.id` values (batching phase)
3. Loader calls pipeline once with `.keys = [1, 2, 3]`
4. Loader distributes results back to each resolver call

### Complete DataLoader Example

```wp
# GraphQL Schema
graphqlSchema = `
  type Employee {
    id: ID!
    name: String!
    email: String!
    teamId: Int!
  }

  type Team {
    id: ID!
    name: String!
    employees(limit: Int): [Employee!]!
  }

  type Query {
    teams: [Team!]!
    team(id: ID!): Team
  }
`

# Query resolvers
query teams =
  |> pg: `SELECT id, name FROM teams`
  |> jq: `.data.rows`

query team =
  |> pg([.id]): `SELECT id, name FROM teams WHERE id = $1`
  |> jq: `.data.rows[0]`

# Loader pipeline for employees
pipeline EmployeesByTeamLoader =
  |> pg([(.keys | map(tostring) | join(",")), (.args.limit // 10)]): `!raw
    SELECT
      json_object_agg(
        sub.team_id,
        sub.employees
      )
    FROM (
      SELECT
        team_id,
        json_agg(
          json_build_object(
            'id', id::text,
            'name', name,
            'email', email,
            'teamId', team_id::text
          )
        ) AS employees
      FROM (
        SELECT
          id,
          name,
          email,
          team_id,
          ROW_NUMBER() OVER (
            PARTITION BY team_id
            ORDER BY id
          ) AS rn
        FROM employees
        WHERE team_id::text = ANY(string_to_array($1, ','))
      ) ranked
      WHERE ranked.rn <= $2
      GROUP BY team_id
    ) sub
  `

# Nested resolver with DataLoader
resolver Team.employees =
  |> loader(.parent.id): EmployeesByTeamLoader

# Use it in a query
GET /test-teams-with-employees
  |> graphql: `
    query {
      teams {
        id
        name
        employees(limit: 5) {
          id
          name
          email
        }
      }
    }
  `
```

### DataLoader with Field Arguments

Pass GraphQL field arguments to the loader:

```wp
# Schema with arguments
graphqlSchema = `
  type Team {
    employees(limit: Int, status: String): [Employee!]!
  }
`

# Loader handles arguments
pipeline EmployeesByTeamLoader =
  # .args contains { limit: 5, status: "active" }
  |> pg([
    (.keys | join(",")),
    (.args.limit // 10),
    (.args.status // "active")
  ]): `
    SELECT json_object_agg(team_id, employees)
    FROM (
      SELECT team_id, json_agg(...) AS employees
      FROM employees
      WHERE team_id = ANY(string_to_array($1, ','))
        AND status = $3
      LIMIT $2
      GROUP BY team_id
    ) sub
  `

resolver Team.employees =
  |> loader(.parent.id): EmployeesByTeamLoader
```

### DataLoader with Access Control

Use `.context` for authorization:

```wp
pipeline EmployeesByTeamLoader =
  # Check user permissions in context
  |> lua: `
    if not context.user or context.user.role ~= "admin" then
      return { errors = {{ type = "authError", message = "Unauthorized" }} }
    end
    return request
  `
  |> result
    ok(200):
      |> pg([(.keys | join(","))]): `
        SELECT json_object_agg(team_id, employees)
        FROM (...)
      `
    authError(403):
      |> jq: `{ errors: .errors }`
```

### Multiple DataLoaders

Define multiple loaders for different relationships:

```wp
pipeline EmployeesByTeamLoader =
  # ... as before

pipeline ProjectsByTeamLoader =
  |> pg([(.keys | join(","))]): `
    SELECT json_object_agg(team_id, projects)
    FROM (
      SELECT team_id, json_agg(...) AS projects
      FROM projects
      WHERE team_id = ANY(string_to_array($1, ','))
      GROUP BY team_id
    ) sub
  `

resolver Team.employees =
  |> loader(.parent.id): EmployeesByTeamLoader

resolver Team.projects =
  |> loader(.parent.id): ProjectsByTeamLoader
```

### Alternative: Manual Batching Without Resolver

You can also call loader pipelines directly in routes:

```wp
GET /test-teams-with-employees-manual
  # Fetch all teams
  |> pg: `SELECT id, name FROM teams`

  # Prepare batched loader call
  |> jq: `.data.rows | map(.id) as $keys | {
    keys: $keys,
    args: { limit: 5 }
  }`

  # Call loader pipeline
  |> pipeline: EmployeesByTeamLoader

  # Combine results
  |> jq: `{
    teams: .keys | map({
      id: .,
      employees: $result[. | tostring] // []
    })
  }`
```

### Testing DataLoaders

Test loader pipelines with batched inputs:

```wp
describe "DataLoader N+1 Prevention"
  it "fetches teams with employees using batching"
    when calling GET /test-teams-with-employees
    then status is 200
    and output `.data.teams | type` equals `"array"`
    and output `.data.teams[0].name | type` equals `"string"`
    and output `.data.teams[0].employees | type` equals `"array"`
```

### Performance Benefits

**Without DataLoader:**
```
Query: teams { employees { name } }
Execution:
  - SELECT * FROM teams           (1 query)
  - SELECT * FROM employees WHERE team_id = 1  (1 query)
  - SELECT * FROM employees WHERE team_id = 2  (1 query)
  - SELECT * FROM employees WHERE team_id = 3  (1 query)
  Total: 4 queries for 3 teams
```

**With DataLoader:**
```
Query: teams { employees { name } }
Execution:
  - SELECT * FROM teams           (1 query)
  - SELECT * FROM employees WHERE team_id IN (1,2,3)  (1 query)
  Total: 2 queries for 3 teams
```

### Performance Comparison Table

| Scenario | Without DataLoader | With DataLoader |
|----------|-------------------|-----------------|
| 10 teams | 11 queries | 2 queries |
| 100 teams | 101 queries | 2 queries |
| 1000 teams | 1001 queries | 2 queries |

The performance benefit increases dramatically with more parent entities.

### Advanced DataLoader Patterns

#### Multiple Nested Loaders

Chain DataLoaders for deeply nested queries:

```wp
# Loader for posts by user ID
pipeline PostsByUserLoader =
  |> pg([(.keys | map(tostring) | join(","))]): `!raw
    SELECT json_object_agg(sub.user_id, sub.posts)
    FROM (
      SELECT user_id, json_agg(json_build_object('id', id, 'title', title)) as posts
      FROM posts
      WHERE user_id::text = ANY(string_to_array($1, ','))
      GROUP BY user_id
    ) sub
  `

# Loader for comments by post ID
pipeline CommentsByPostLoader =
  |> pg([(.keys | map(tostring) | join(","))]): `!raw
    SELECT json_object_agg(sub.post_id, sub.comments)
    FROM (
      SELECT post_id, json_agg(json_build_object('id', id, 'text', text)) as comments
      FROM comments
      WHERE post_id::text = ANY(string_to_array($1, ','))
      GROUP BY post_id
    ) sub
  `

# Nested resolvers
resolver User.posts =
  |> loader(.parent.id): PostsByUserLoader

resolver Post.comments =
  |> loader(.parent.id): CommentsByPostLoader
```

**Query with 3 levels of nesting:**
```graphql
query {
  users {           # 1 query
    posts {         # 1 batched query for all users
      comments {    # 1 batched query for all posts
        text
      }
    }
  }
}
# Total: 3 queries (instead of potentially 1000s)
```

#### Conditional Loading

Return null for missing relationships:

```wp
resolver Post.author =
  |> jq: `if .parent.authorId then . else { result: null } end`
  |> if
    |> jq: `.result == null`
    then:
      |> jq: `null`
    else:
      |> loader(.parent.authorId): UserLoader
    end
```

#### Complex Composite Keys

Use compound keys for complex relationships:

```wp
# Loader that uses composite keys (type:id format)
pipeline ItemsByCompositeKeyLoader =
  |> jq: `{
    parsed: (.keys | map(split(":")) | map({type: .[0], id: .[1]})),
    keys: .keys
  }`
  |> pg([.parsed]): `
    SELECT
      item_type || ':' || item_id as composite_key,
      json_build_object('id', id, 'name', name, 'type', item_type) as item
    FROM items
    WHERE (item_type, item_id::text) IN (
      SELECT elem->>'type', elem->>'id'
      FROM jsonb_array_elements($1::jsonb) elem
    )
  `
  |> jq: `
    .data.rows | reduce .[] as $row (
      {};
      . + { ($row.composite_key): $row.item }
    )
  `

# Resolver with composite key
resolver Order.item =
  |> jq: `.parent.itemType + ":" + (.parent.itemId | tostring)`
  |> loader(.): ItemsByCompositeKeyLoader
```

### Troubleshooting DataLoaders

#### Error: "Loader pipeline must return an object/map"

Your loader is returning an array instead of an object. Make sure to convert rows to a map:

```wp
# ❌ Wrong - returns array
|> pg: `SELECT * FROM users WHERE id = ANY($1)`
|> jq: `.data.rows`

# ✅ Correct - returns map keyed by ID
|> pg: `SELECT * FROM users WHERE id = ANY($1)`
|> jq: `
  .data.rows | reduce .[] as $row (
    {};
    . + { ($row.id | tostring): $row }
  )
`
```

**Best practice:** Use PostgreSQL's `json_object_agg` to build the map in the database:

```wp
|> pg([(.keys | map(tostring) | join(","))]): `!raw
  SELECT json_object_agg(id::text, row_to_json(u.*))
  FROM users u
  WHERE id::text = ANY(string_to_array($1, ','))
`
```

#### Keys Are Not Batching

Make sure you're using the `resolver` keyword, not `query`:

```wp
# ❌ Won't batch - runs for each parent
query Post.author =
  |> pg([.parent.authorId]): `SELECT * FROM users WHERE id = $1`

# ✅ Will batch - collects all keys first
resolver Post.author =
  |> loader(.parent.authorId): UserLoader
```

#### Parent Value Is Undefined

Check that your GraphQL schema defines the field:

```wp
# Schema must define the nested field
graphql {
  type Post {
    id: ID!
    title: String!
    author: User  # ← Must be defined here
  }

  type User {
    id: ID!
    name: String!
  }
}

# Then define the resolver
resolver Post.author =
  |> loader(.parent.authorId): UserLoader
```

If `.parent.authorId` is undefined, check:
1. The parent query returns objects with `authorId` field
2. Field name matches exactly (case-sensitive)
3. The parent resolver is working correctly

#### Missing Keys in Result Map

If some keys don't have data, return `null` or empty array:

```wp
pipeline UserLoader =
  |> pg([(.keys | map(tostring) | join(","))]): `!raw
    SELECT json_object_agg(id::text, row_to_json(u.*))
    FROM users u
    WHERE id::text = ANY(string_to_array($1, ','))
  `
  |> jq: `
    # Build complete map with nulls for missing keys
    .keys | reduce .[] as $key (
      .result // {};
      if has($key | tostring) then . else . + {($key | tostring): null} end
    )
  `
```

### Best Practices

1. **Use DataLoaders for all nested resolvers** that query databases
2. **Return keyed objects** mapping each key to its result (use `json_object_agg`)
3. **Handle missing keys** by returning empty arrays/null in the result map
4. **Convert IDs to strings** - JSON object keys must be strings
5. **Use field arguments** for filtering, limiting, sorting via `.args`
6. **Check permissions** in loader pipeline using `.context`
7. **Test with multiple entities** to verify batching works
8. **Monitor query logs** to confirm N+1 queries are eliminated
9. **Use `!raw` flag** when PostgreSQL returns pre-formatted JSON
10. **Keep loaders simple** - complex logic belongs in the parent resolver

---

## See Also

- [GraphQL Middleware](./middleware/graphql.md) - GraphQL middleware usage
- [Concurrency](./concurrency.md) - Async execution patterns
- [Flow Control](./flow-control.md) - @guard, @when, dispatch
- [Routes & Pipelines](./routes-and-pipelines.md) - @result and named results
- [DSL Syntax](./dsl-syntax.md) - Complete syntax reference

