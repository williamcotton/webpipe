# GraphQL Middleware

Execute GraphQL queries and mutations against your GraphQL schema.

---

## Overview

The `graphql` middleware allows you to execute GraphQL queries and mutations from within Web Pipe pipelines. It integrates with the GraphQL engine configured via `config graphql` and supports variables, named results, and async execution.

---

## Configuration

Configure the GraphQL endpoint in your Web Pipe file:

```wp
config graphql {
  endpoint: "/graphql"
}
```

---

## Basic Usage

### Simple Query

Execute a GraphQL query without variables:

```wp
GET /test-graphql-simple
  |> graphql: `
    query {
      currentTime
      randomNumber
    }
  `
```

**Output** (stored in `.data`):
```json
{
  "currentTime": "2024-01-15T10:30:00Z",
  "randomNumber": 42
}
```

### Query with Variables

Pass variables using `graphqlParams`:

```wp
GET /test-graphql-args
  |> jq: `{ graphqlParams: { id: 1, limit: 3 } }`
  |> graphql: `
    query($id: Int!, $limit: Int) {
      user(id: $id) {
        id
        name
        email
      }
      posts(limit: $limit) {
        id
        title
      }
    }
  `
```

### Inline Object Parameters

Use inline object parameters (modern approach):

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
      }
    }
  `
```

---

## Mutations

### Basic Mutation

Execute a mutation with variables:

```wp
graphql testMutation = `
  mutation($title: String!, $body: String!) {
    createPost(title: $title, body: $body) {
      id
      title
      body
    }
  }
`

POST /test-graphql-mutation
  |> jq: `{ graphqlParams: { title: .body.title, body: .body.body } }`
  |> graphql: testMutation
```

### Mutation with Inline Parameters

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

## Named Results

### Using resultName (Legacy)

Store GraphQL results under a specific name:

```wp
GET /test-graphql-multi
  |> jq: `{ resultName: "time" }`
  |> graphql: `query { currentTime }`

  |> jq: `{ resultName: "users" }`
  |> graphql: `query { users { id name email } }`

  |> jq: `{ resultName: "stats" }`
  |> graphql: `query { stats { totalUsers totalPosts activeUsers } }`

  |> jq: `{
    loadedAt: .data.time.currentTime,
    statistics: .data.stats.stats,
    users: .data.users.users
  }`
```

### Using @result Tag (Modern)

Use the `@result(name)` tag for cleaner result naming:

```wp
GET /dashboard/data
  |> graphql: `query { currentTime }` @result(time)
  |> graphql: `query { users { id name email } }` @result(users)
  |> graphql: `query { stats { totalUsers totalPosts } }` @result(stats)

  |> jq: `{
    loadedAt: .data.time.currentTime,
    userCount: (.data.users.users | length),
    totalUsers: .data.stats.stats.totalUsers
  }`
```

---

## Async Execution

### Parallel GraphQL Queries

Execute multiple GraphQL queries in parallel for maximum performance:

```wp
GET /test-graphql-async-parallel
  |> graphql: `query { users { id name email } }` @async(usersQuery)
  |> graphql: `query { stats { totalUsers totalPosts activeUsers } }` @async(statsQuery)
  |> graphql: `query { posts(limit: 5) { id title userId published } }` @async(postsQuery)
  |> graphql: `query { currentTime randomNumber }` @async(timeQuery)

  |> join: `usersQuery, statsQuery, postsQuery, timeQuery`

  |> jq: `{
    dashboard: {
      loadedAt: .async.timeQuery.data.currentTime,
      randomNumber: .async.timeQuery.data.randomNumber,
      statistics: .async.statsQuery.data.stats,
      users: .async.usersQuery.data.users,
      recentPosts: .async.postsQuery.data.posts,
      metadata: {
        queriesExecuted: 4,
        executionMode: "parallel"
      }
    }
  }`
```

**How it works:**
- Each `@async(name)` tag launches the query in the background
- `join` waits for all queries to complete
- Results are available under `.async.<taskName>.data`

### Sequential vs Parallel Comparison

**Sequential (slower):**
```wp
GET /test-graphql-sync-sequential
  |> jq: `{ resultName: "users" }`
  |> graphql: `query { users { id name email } }`
  |> jq: `{ resultName: "stats" }`
  |> graphql: `query { stats { totalUsers totalPosts } }`
  |> jq: `{ resultName: "posts" }`
  |> graphql: `query { posts(limit: 5) { id title } }`
```

**Parallel (faster):**
```wp
GET /test-graphql-async-parallel
  |> graphql: `query { users { id name email } }` @async(users)
  |> graphql: `query { stats { totalUsers totalPosts } }` @async(stats)
  |> graphql: `query { posts(limit: 5) { id title } }` @async(posts)
  |> join: `users, stats, posts`
```

---

## GraphQL Variables

### Using GraphQL Variables

Define a GraphQL query variable and pass parameters:

```wp
graphql getUserQuery = `
  query($id: ID!) {
    user(id: $id) {
      name
      email
    }
  }
`

GET /users/:id
  |> jq: `{ graphqlParams: { id: .params.id } }`
  |> graphql: getUserQuery
```

### Multiple Variables

```wp
GET /gql/posts
  |> jq: `{
    graphqlParams: {
      author: (.query.author | tonumber),
      limit: (.query.limit // 10 | tonumber),
      offset: (.query.offset // 0 | tonumber)
    }
  }`
  |> graphql: `
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

---

## Integration with Other Middleware

### With Authentication

Combine GraphQL with auth middleware:

```wp
POST /gql/protected
  |> auth: "required"
  |> result
    ok(200):
      |> jq: `{ graphqlParams: { userId: .user.id } }`
      |> graphql: `query($userId: ID!) { userProfile(id: $userId) { ... } }`
    authRequired(401):
      |> jq: `{ error: "Authentication required" }`
```

### With Caching

Cache GraphQL results:

```wp
GET /gql/cached-users
  |> cache: `ttl: 300, keyTemplate: "gql-users"`
  |> graphql: `query { users { id name email } }`
```

### With Rate Limiting

Rate limit GraphQL endpoints:

```wp
POST /gql/public
  |> rateLimit: `limit: 100, window: 3600, keyTemplate: "gql-public"`
  |> graphql: `query { publicData { ... } }`
```

---

## Advanced Patterns

### Conditional GraphQL Queries

Execute different queries based on conditions:

```wp
GET /gql/conditional
  |> lua: `
    setWhen("is_admin", request.user and request.user.role == "admin")
    return request
  `
  |> dispatch
    case @when(is_admin):
      |> graphql: `query { adminData { sensitive stats } }`
    default:
      |> graphql: `query { publicData { basic info } }`
```

### Dynamic Query Building

Build GraphQL queries dynamically:

```wp
GET /gql/dynamic
  |> jq: `{
    fields: (if .query.detailed == "true" then "id name email bio avatar" else "id name" end)
  }`
  |> graphql: `query { users { ` + .fields + ` } }`
```

### Combining REST and GraphQL

Mix REST and GraphQL data sources:

```wp
GET /mixed/user-profile
  |> fetch: `https://api.external.com/users/1` @result(basicInfo)
  |> graphql: `query { userPreferences(id: 1) { theme language } }` @result(preferences)
  |> graphql: `query { userActivity(id: 1) { lastLogin posts } }` @result(activity)

  |> jq: `{
    name: .data.basicInfo.response.name,
    email: .data.basicInfo.response.email,
    theme: .data.preferences.userPreferences.theme,
    lastLogin: .data.activity.userActivity.lastLogin
  }`
```

---

## Input Parameters

The `graphql` middleware accepts the following input parameters:

- **`graphqlParams`** (object) - Variables to pass to the GraphQL query
- **`resultName`** (string) - Name to store result under `.data.<name>` (legacy)

Alternatively, use inline object parameters:
```wp
|> graphql({ userId: .id, limit: 10 }): `query($userId: ID!, $limit: Int) { ... }`
```

---

## Output Structure

GraphQL results are stored under `.data` (or `.data.<resultName>` if specified):

```json
{
  "data": {
    "user": { "id": 1, "name": "Alice" },
    "posts": [{ "id": 100, "title": "Post 1" }]
  }
}
```

When using `@async`, results are stored under `.async.<taskName>.data`:

```json
{
  "async": {
    "usersQuery": {
      "data": {
        "users": [{ "id": 1, "name": "Alice" }]
      }
    }
  }
}
```

---

## Error Handling

GraphQL errors are returned in the standard GraphQL error format:

```json
{
  "errors": [
    {
      "message": "Cannot query field 'nonExistent' on type 'User'",
      "locations": [{ "line": 2, "column": 3 }]
    }
  ]
}
```

Handle GraphQL errors in result blocks:

```wp
|> graphql: `query { user(id: $id) { name } }`
|> result
  ok(200):
    |> jq: `{ user: .data.user }`
  graphqlError(400):
    |> jq: `{ error: "GraphQL query failed", details: .errors }`
```

---

## Best Practices

### 1. Use @result for Multiple Queries

When executing multiple GraphQL queries, use `@result` tags:

```wp
|> graphql: `query { users { ... } }` @result(users)
|> graphql: `query { posts { ... } }` @result(posts)
```

### 2. Leverage Async for Independent Queries

If queries don't depend on each other, execute them in parallel:

```wp
|> graphql: `query { data1 }` @async(q1)
|> graphql: `query { data2 }` @async(q2)
|> join: `q1, q2`
```

### 3. Cache Expensive Queries

Cache GraphQL results that don't change frequently:

```wp
|> cache: `ttl: 600`
|> graphql: `query { expensiveAggregation { ... } }`
```

### 4. Use Variables for Security

Always use GraphQL variables instead of string interpolation:

**Good:**
```wp
|> jq: `{ graphqlParams: { id: .params.id } }`
|> graphql: `query($id: ID!) { user(id: $id) { ... } }`
```

**Avoid:**
```wp
|> graphql: `query { user(id: ` + .params.id + `) { ... } }`
```

### 5. Combine with @guard for Conditional Execution

Skip GraphQL queries based on conditions:

```wp
|> graphql: `query { adminData { ... } }` @guard(`.user.role == "admin"`)
```

---

## See Also

- [Inline Arguments](../inline-arguments.md) - Passing variables with inline syntax
- [Routes & Pipelines](../routes-and-pipelines.md) - Named results with @result
- [Concurrency](../concurrency.md) - Async execution and join middleware
- [Flow Control](../flow-control.md) - Conditional execution with @guard, @when
- [GraphQL](../graphql.md) - GraphQL schema and resolver documentation
