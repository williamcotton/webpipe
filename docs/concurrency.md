# Concurrency & Async

WebPipe 2.0 introduces parallelism to the DSL, allowing you to execute long-running tasks in the background and join their results.

## Async Steps

You can tag any pipeline step with `@async(taskName)` to spawn it as a background task. The pipeline will immediately proceed to the next step without waiting for the async step to complete.

```wp
GET /dashboard
  # Start fetching user data in background
  |> fetch: `https://api.example.com/users` @async(users)
  
  # Start fetching posts in background
  |> fetch: `https://api.example.com/posts` @async(posts)
  
  # Do some other work while waiting...
  |> log: "Fetching data..."
```

## Joining Results

To retrieve the results of async tasks, use the `join` middleware. This step will suspend execution until the specified tasks are complete.

```wp
  # ... previous async steps ...
  
  # Wait for both tasks to finish
  |> join: `users, posts`
  
  # Results are available in the .async object
  |> jq: `{ 
      userCount: .async.users.data.length, 
      postCount: .async.posts.data.length 
    }`
```

The results of the async tasks are merged into the pipeline input under the `.async` key, keyed by the task name provided in the `@async` tag.

## Example Flow

```wp
GET /summary
  |> jq: `{ status: "processing" }`
  
  # Fork: Fetch external data
  |> fetch: `https://analytics.service/stats` @async(external_stats)
  
  # Fork: Run heavy DB query
  |> pg: `SELECT count(*) FROM huge_table` @async(db_stats)
  
  # Main thread continues...
  
  # Join: Sync both tasks
  |> join: `external_stats, db_stats`
  
  # Merge results
  |> jq: `{
      analytics: .async.external_stats.data,
      db: .async.db_stats.data.rows[0]
    }`
```

---

## Async with GraphQL

Execute multiple GraphQL queries in parallel for maximum performance.

### Basic GraphQL Async Pattern

```wp
GET /graphql-dashboard
  |> graphql: `query { users { id name email } }` @async(usersQuery)
  |> graphql: `query { stats { totalUsers totalPosts } }` @async(statsQuery)
  |> graphql: `query { posts(limit: 5) { id title } }` @async(postsQuery)

  |> join: `usersQuery, statsQuery, postsQuery`

  |> jq: `{
    users: .async.usersQuery.data.users,
    statistics: .async.statsQuery.data.stats,
    recentPosts: .async.postsQuery.data.posts
  }`
```

### GraphQL with @result Tags

Combine async execution with named results:

```wp
GET /dashboard/data
  |> graphql: `query { users { id name } }` @async(q1) @result(users)
  |> graphql: `query { posts { id title } }` @async(q2) @result(posts)
  |> graphql: `query { comments { id text } }` @async(q3) @result(comments)

  |> join: `q1, q2, q3`

  |> jq: `{
    userCount: (.async.q1.data.users.users | length),
    postCount: (.async.q2.data.posts.posts | length),
    commentCount: (.async.q3.data.comments.comments | length)
  }`
```

**Result Path with @async and @result:**
- Without @result: `.async.<taskName>.data`
- With @result: `.async.<taskName>.data.<resultName>`

### Performance Comparison

**Sequential (slow):**
```wp
GET /sequential
  |> graphql: `query { users { ... } }`
  |> graphql: `query { stats { ... } }`
  |> graphql: `query { posts { ... } }`
  # Total time: ~300ms (100ms each)
```

**Parallel (fast):**
```wp
GET /parallel
  |> graphql: `query { users { ... } }` @async(users)
  |> graphql: `query { stats { ... } }` @async(stats)
  |> graphql: `query { posts { ... } }` @async(posts)
  |> join: `users, stats, posts`
  # Total time: ~100ms (all execute concurrently)
```

---

## Combining Multiple Middleware Types

Mix different async middleware for complex data aggregation:

```wp
GET /mixed-async
  # Fetch from external API
  |> fetch: `https://api.external.com/data` @async(externalData)

  # Query internal GraphQL
  |> graphql: `query { users { id name } }` @async(graphqlData)

  # Query PostgreSQL
  |> pg: `SELECT * FROM stats` @async(dbData)

  # Wait for all
  |> join: `externalData, graphqlData, dbData`

  # Aggregate results
  |> jq: `{
    external: .async.externalData.data.response,
    internal: .async.graphqlData.data.users,
    stats: .async.dbData.data.rows
  }`
```

---

## Advanced Patterns

### Conditional Async Execution

Execute async tasks based on conditions:

```wp
GET /conditional-async
  |> lua: `
    setWhen("is_premium", request.user and request.user.premium)
    return request
  `

  # All users get basic data
  |> graphql: `query { basicData { ... } }` @async(basic)

  # Premium users get additional data
  |> graphql: `query { premiumData { ... } }` @async(premium) @when(is_premium)

  # Join only the tasks that executed
  |> join: `basic, premium`

  |> jq: `{
    basic: .async.basic.data,
    premium: (.async.premium.data // null)
  }`
```

### Nested Async Operations

Launch async tasks that depend on earlier results:

```wp
GET /nested-async
  # First fetch user
  |> fetch: `https://api.example.com/users/1` @result(user)

  # Then fetch related data in parallel using user data
  |> graphql({ userId: .data.user.response.id }): `
    query($userId: ID!) {
      posts(authorId: $userId) { id title }
    }
  ` @async(posts)

  |> fetch("https://api.example.com/followers/" + (.data.user.response.id | tostring)) @async(followers)

  |> join: `posts, followers`

  |> jq: `{
    user: .data.user.response,
    posts: .async.posts.data.posts,
    followers: .async.followers.data.response
  }`
```

---

## Error Handling with Async

Handle errors from async tasks:

```wp
GET /async-with-errors
  |> fetch: `https://api.example.com/data` @async(api1)
  |> graphql: `query { data { ... } }` @async(api2)

  |> join: `api1, api2`

  |> result
    ok(200):
      |> jq: `{
        data1: .async.api1.data,
        data2: .async.api2.data
      }`
    networkError(502):
      |> jq: `{ error: "External service unavailable" }`
    graphqlError(500):
      |> jq: `{ error: "GraphQL query failed", details: .errors }`
```

---

## Best Practices

### 1. Use Async for Independent Queries

If queries don't depend on each other, execute them in parallel:

**Good:**
```wp
|> graphql: `query { users }` @async(users)
|> graphql: `query { posts }` @async(posts)
|> join: `users, posts`
```

**Avoid:**
```wp
|> graphql: `query { users }`
|> graphql: `query { posts }`
```

### 2. Name Tasks Descriptively

Use clear task names for async operations:

```wp
@async(userProfileQuery)
@async(orderHistoryQuery)
@async(analyticsData)
```

### 3. Join Only What You Need

Only join the tasks you actually need:

```wp
|> join: `users, posts`  # Not all async tasks if some are optional
```

### 4. Combine @async with @result

For complex aggregations, use both tags:

```wp
|> graphql: `query { ... }` @async(task1) @result(users)
```

### 5. Handle Missing Results

Check for null when async tasks may be skipped:

```wp
|> jq: `{
  premium: (.async.premium.data // null),
  basic: .async.basic.data
}`
```

---

## See Also

- [GraphQL Middleware](./middleware/graphql.md) - GraphQL with async and @result
- [Routes & Pipelines](./routes-and-pipelines.md) - Named results patterns
- [Flow Control](./flow-control.md) - Conditional execution with @when, @guard
- [DSL Syntax](./dsl-syntax.md) - Complete @async syntax reference

