# Routes & Pipelines

Routes define HTTP endpoints and their processing pipelines. Pipelines are reusable sequences of middleware steps that can be composed and invoked from routes.

---

## Routes

Routes map HTTP requests to processing pipelines.

### Basic Syntax

```wp
<METHOD> <path>
  |> <middleware1>
  |> <middleware2>
  |> <middleware3>
```

### HTTP Methods

Web Pipe supports all standard HTTP methods:

```wp
GET /users
POST /users
PUT /users/:id
DELETE /users/:id
PATCH /users/:id
```

### Path Parameters

Use `:paramName` for dynamic URL segments:

```wp
GET /users/:id
  |> jq: `{ userId: (.params.id | tonumber) }`
  |> pg([.userId]): `SELECT * FROM users WHERE id = $1`
```

**Multiple Parameters:**
```wp
GET /posts/:postId/comments/:commentId
  |> jq: `{
    postId: (.params.postId | tonumber),
    commentId: (.params.commentId | tonumber)
  }`
```

### Query String Parameters

Access via `.query`:

```wp
GET /search
  |> jq: `{
    term: .query.q,
    limit: (.query.limit // 10 | tonumber),
    offset: (.query.offset // 0 | tonumber)
  }`
```

### Request Body

Access POST/PUT body via `.body`:

```wp
POST /users
  |> validate: `{
    name: string(3..100),
    email: email
  }`
  |> pg([.body.name, .body.email]): `
    INSERT INTO users (name, email) VALUES ($1, $2) RETURNING *
  `
```

### Headers

Access request headers via `.headers`:

```wp
GET /api/data
  |> jq: `{
    auth: .headers.authorization,
    contentType: .headers["content-type"],
    userAgent: .headers["user-agent"]
  }`
```

---

## Pipelines

Pipelines are named, reusable sequences of middleware steps.

### Basic Pipeline Definition

```wp
pipeline loadUser =
  |> jq: `{ sqlParams: [.params.id] }`
  |> pg: `SELECT * FROM users WHERE id = $1`
```

### Pipeline Invocation

```wp
GET /users/:id
  |> pipeline: loadUser
  |> jq: `{ user: .data.rows[0] }`
```

### Pipeline Invocation with Arguments

Pass arguments to pipelines using inline object syntax:

```wp
pipeline getTeam =
  |> pg: `SELECT * FROM teams WHERE id = $1`

GET /team/:id
  |> pipeline({ sqlParams: [.params.id] }): getTeam
  |> jq: `{ team: .data.rows[0] }`
```

**How it works:**
- The object `{ sqlParams: [.params.id] }` is merged into the pipeline input
- The pipeline receives both the current state and the inline arguments
- Useful for parameterized pipelines

**Multiple Arguments:**
```wp
pipeline getUserWithFilters =
  |> pg: `SELECT * FROM users WHERE id = $1 AND status = $2`

GET /user/:id
  |> pipeline({
    sqlParams: [.params.id, .query.status // "active"]
  }): getUserWithFilters
```

**Pattern: Reusable Data Loaders:**
```wp
pipeline EmployeeLoader =
  |> pg([.keys]): `
    SELECT * FROM employees
    WHERE team_id = ANY($1::int[])
  `

GET /teams/:id/employees
  |> pipeline({ keys: [.params.id] }): EmployeeLoader
```

### Complex Pipelines

Pipelines can contain multiple steps, conditionals, and transformations:

```wp
pipeline authenticateUser =
  |> validate: `{
    email: string(5..100),
    password: string(8..100)
  }`
  |> auth: "login"
  |> result
    ok(200):
      |> jq: `{ success: true, user: .user }`
    validationError(400):
      |> jq: `{ error: "Validation failed", details: .errors }`
    authError(401):
      |> jq: `{ error: "Invalid credentials" }`
```

### Composing Pipelines

Pipelines can call other pipelines:

```wp
pipeline loadUserData =
  |> pg: `SELECT * FROM users WHERE id = $1`

pipeline enrichUserData =
  |> pipeline: loadUserData
  |> pg: `SELECT * FROM user_preferences WHERE user_id = $1`
  |> jq: `{
    user: .data.rows[0],
    preferences: .data.rows[1]
  }`
```

### Special Pipelines

**featureFlags Pipeline:**

Runs before every route to set feature flags:

```wp
pipeline featureFlags =
  |> lua: `
    math.randomseed(os.time())
    setFlag("experimental-feature", (math.random() < 0.5))
    setFlag("beta-features", true)
    return request
  `
```

---

## Named Results

Web Pipe provides three ways to name and accumulate results from middleware calls, allowing you to capture multiple query results and combine them in a single route.

### Three Methods for Naming Results

#### 1. Variable Auto-Naming (Automatic)

When using a variable with middleware, results are automatically captured under `.data.<variableName>`:

```wp
pg getActiveUsers = `SELECT * FROM users WHERE status = 'active'`

GET /users
  |> pg: getActiveUsers
  |> jq: `{ users: .data.getActiveUsers.rows }`
```

**How it works:**
- The variable name becomes the result key
- No manual configuration needed
- Result available at `.data.<variableName>`

#### 2. resultName Variable (Legacy)

Set `resultName` in JQ before calling middleware:

```wp
GET /legacy/user-with-dept
  |> jq: `{ sqlParams: [1], resultName: "user" }`
  |> pg: `SELECT * FROM users WHERE id = $1`

  |> jq: `. + { sqlParams: [.data.user.rows[0].dept_id], resultName: "department" }`
  |> pg: `SELECT * FROM departments WHERE id = $1`

  |> jq: `{
    userName: .data.user.rows[0].name,
    deptName: .data.department.rows[0].name
  }`
```

**How it works:**
- Set `resultName` field in the input state
- Middleware stores result under `.data.<resultName>`
- More verbose but explicit

#### 3. @result Tag (Modern, Recommended)

Use the `@result(name)` tag to name results declaratively:

```wp
GET /modern/user-with-dept
  |> jq: `{ sqlParams: [1] }`
  |> pg: `SELECT * FROM users WHERE id = $1` @result(user)

  |> jq: `{ sqlParams: [.data.user.rows[0].dept_id] }`
  |> pg: `SELECT * FROM departments WHERE id = $1` @result(department)

  |> jq: `{
    userName: .data.user.rows[0].name,
    deptName: .data.department.rows[0].name
  }`
```

**How it works:**
- Tag specifies the result name directly on the middleware call
- Cleaner and more declarative than `resultName` variable
- Result available at `.data.<name>`

### Precedence Rules

When multiple naming methods are present, the precedence is:

**@result tag > resultName variable > variable auto-naming**

```wp
pg myQuery = `SELECT * FROM data`

GET /precedence-demo
  |> jq: `{ resultName: "fromVariable" }`
  |> pg: myQuery @result(fromTag)
  # Result will be at .data.fromTag (tag wins)
```

### Accessing Named Results

All named results accumulate under the `.data` object:

```wp
GET /multiple-results
  |> pg: `SELECT * FROM users` @result(users)
  |> pg: `SELECT * FROM posts` @result(posts)
  |> pg: `SELECT * FROM comments` @result(comments)

  |> jq: `{
    userCount: .data.users.rowCount,
    postCount: .data.posts.rowCount,
    commentCount: .data.comments.rowCount,
    allUsers: .data.users.rows,
    allPosts: .data.posts.rows,
    allComments: .data.comments.rows
  }`
```

---

## Multiple Result Accumulation

Named results enable powerful data aggregation patterns.

### Sequential Queries with Dependencies

Each query can use results from previous queries:

```wp
GET /user/:id/full-profile
  |> jq: `{ sqlParams: [.params.id] }`
  |> pg: `SELECT * FROM users WHERE id = $1` @result(user)

  |> jq: `{ sqlParams: [.data.user.rows[0].company_id] }`
  |> pg: `SELECT * FROM companies WHERE id = $1` @result(company)

  |> jq: `{ sqlParams: [.params.id] }`
  |> pg: `SELECT * FROM orders WHERE user_id = $1` @result(orders)

  |> jq: `{
    user: .data.user.rows[0],
    company: .data.company.rows[0],
    orders: .data.orders.rows,
    totalOrders: .data.orders.rowCount
  }`
```

### Dashboard-Style Aggregation

Collect multiple datasets for dashboard views:

```wp
GET /dashboard/summary
  |> pg: `SELECT * FROM departments` @result(departments)
  |> pg: `SELECT * FROM employees` @result(employees)
  |> pg: `SELECT * FROM projects WHERE status = 'active'` @result(projects)

  |> jq: `{
    totalDepartments: .data.departments.rowCount,
    totalEmployees: .data.employees.rowCount,
    totalProjects: .data.projects.rowCount,
    departmentNames: [.data.departments.rows[].name],
    employeeNames: [.data.employees.rows[].name],
    activeProjects: [.data.projects.rows[].name]
  }`
```

### Mixing Middleware Types

Named results work across different middleware:

```wp
GET /mixed/user-profile
  |> fetch: `https://api.example.com/users/1` @result(basicInfo)
  |> pg: `SELECT * FROM user_preferences WHERE user_id = 1` @result(preferences)
  |> fetch: `https://api.example.com/users/1/todos` @result(todos)

  |> jq: `{
    userName: .data.basicInfo.response.name,
    userEmail: .data.basicInfo.response.email,
    theme: .data.preferences.rows[0].theme,
    todoCount: (.data.todos.response | length)
  }`
```

### Variable Auto-Naming with @result Override

You can override auto-naming with the `@result` tag:

```wp
pg getActiveUsers = `SELECT * FROM users WHERE status = 'active'`
pg getAllDepartments = `SELECT * FROM departments`

GET /auto-naming/demo
  # Auto-named: .data.getActiveUsers
  |> pg: getActiveUsers

  # Overridden: .data.departments (not .data.getAllDepartments)
  |> pg: getAllDepartments @result(departments)

  |> jq: `{
    users: .data.getActiveUsers.rows,
    depts: .data.departments.rows
  }`
```

---

## Result Aggregation Patterns

### Pattern: Related Data Loading

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

### Pattern: Parallel Data Sources

```wp
GET /combined-data
  |> fetch: `https://api.external.com/data1` @result(external1)
  |> fetch: `https://api.external.com/data2` @result(external2)
  |> pg: `SELECT * FROM internal_data` @result(internal)

  |> jq: `{
    combined: {
      external: [.data.external1.response, .data.external2.response],
      internal: .data.internal.rows
    }
  }`
```

### Pattern: Conditional Result Accumulation

Combine named results with `@when` or `@guard`:

```wp
GET /conditional-data
  |> lua: `setWhen("is_admin", true); return request`

  |> pg: `SELECT * FROM public_data` @result(public)
  |> pg: `SELECT * FROM admin_data` @result(admin) @when(is_admin)

  |> jq: `{
    public: .data.public.rows,
    admin: (.data.admin.rows // [])
  }`
```

---

## Integration with Concurrency

Named results work seamlessly with async execution:

### Parallel Execution with Named Results

```wp
GET /async-dashboard
  |> fetch: `https://api.example.com/users/1` @async(user) @result(profile)
  |> fetch: `https://api.example.com/posts?userId=1` @async(posts) @result(userPosts)
  |> jq: `{ timestamp: now }`
  |> join: `user,posts`

  |> jq: `{
    loadedAt: .timestamp,
    user: .async.user.data.profile.response,
    posts: .async.posts.data.userPosts.response
  }`
```

**Note:** When using `@async` with `@result`, the named result is stored under `.async.<taskName>.data.<resultName>`.

See [Concurrency](./concurrency.md) for detailed async/join documentation.

---

## Integration with Flow Control

Named results work with all flow control constructs:

### With Dispatch

```wp
GET /dispatch-results
  |> dispatch
    case @env(production):
      |> pg: `SELECT * FROM prod_data` @result(data)
    case @env(development):
      |> pg: `SELECT * FROM dev_data` @result(data)

  |> jq: `{ records: .data.data.rows }`
```

### With Foreach

```wp
GET /batch-processing
  |> pg: `SELECT * FROM items` @result(items)
  |> foreach data.items.rows
    |> jq: `. + { processed: true }`
  end
  |> jq: `{ processedItems: .data.items.rows }`
```

See [Flow Control](./flow-control.md) for detailed documentation.

---

## Best Practices

### 1. Use @result for Clarity

**Prefer:**
```wp
|> pg: `SELECT * FROM users` @result(users)
|> pg: `SELECT * FROM posts` @result(posts)
```

**Over:**
```wp
|> jq: `{ resultName: "users" }`
|> pg: `SELECT * FROM users`
|> jq: `{ resultName: "posts" }`
|> pg: `SELECT * FROM posts`
```

### 2. Name Results Descriptively

Use clear, semantic names:

```wp
# Good
@result(activeUsers)
@result(recentPosts)
@result(userPreferences)

# Avoid
@result(query1)
@result(data)
@result(result)
```

### 3. Check for Existence

When results might be empty or conditional:

```wp
|> jq: `{
  users: (.data.users.rows // []),
  userCount: (.data.users.rowCount // 0)
}`
```

### 4. Document Dependencies

Make data dependencies clear:

```wp
GET /user/:id/profile
  # Load user first (needed for company_id)
  |> pg([.params.id]): `SELECT * FROM users WHERE id = $1` @result(user)

  # Use user.company_id to load company
  |> pg([.data.user.rows[0].company_id]): `SELECT * FROM companies WHERE id = $1` @result(company)
```

### 5. Avoid Naming Conflicts

Each result name should be unique within a route:

```wp
# Avoid - second query overwrites first
|> pg: `SELECT * FROM users` @result(data)
|> pg: `SELECT * FROM posts` @result(data)

# Better - unique names
|> pg: `SELECT * FROM users` @result(users)
|> pg: `SELECT * FROM posts` @result(posts)
```

---

## Common Patterns

### Pattern: Master-Detail Loading

```wp
GET /invoice/:id
  |> pg([.params.id]): `SELECT * FROM invoices WHERE id = $1` @result(invoice)
  |> pg([.params.id]): `SELECT * FROM invoice_items WHERE invoice_id = $1` @result(items)

  |> jq: `{
    invoice: .data.invoice.rows[0],
    items: .data.items.rows,
    total: (.data.items.rows | map(.amount) | add)
  }`
```

### Pattern: Lookup Tables

```wp
GET /report
  |> pg: `SELECT * FROM sales` @result(sales)
  |> pg: `SELECT * FROM products` @result(products)
  |> pg: `SELECT * FROM customers` @result(customers)

  |> jq: `{
    salesData: .data.sales.rows,
    productLookup: (.data.products.rows | map({(.id | tostring): .}) | add),
    customerLookup: (.data.customers.rows | map({(.id | tostring): .}) | add)
  }`
```

### Pattern: Before/After Comparison

```wp
PUT /users/:id
  # Get current state
  |> pg([.params.id]): `SELECT * FROM users WHERE id = $1` @result(before)

  # Update
  |> pg([.body.name, .body.email, .params.id]): `
    UPDATE users SET name = $1, email = $2 WHERE id = $3 RETURNING *
  ` @result(after)

  |> jq: `{
    changed: (.data.before.rows[0] != .data.after.rows[0]),
    before: .data.before.rows[0],
    after: .data.after.rows[0]
  }`
```

---

## See Also

- [DSL Syntax](./dsl-syntax.md) - Complete syntax reference
- [Inline Arguments](./inline-arguments.md) - Passing arguments to middleware
- [Flow Control](./flow-control.md) - Conditional execution and branching
- [Concurrency](./concurrency.md) - Async execution and joining results
- [pg Middleware](./middleware/pg.md) - PostgreSQL queries
- [fetch Middleware](./middleware/fetch.md) - HTTP requests
- [graphql Middleware](./middleware/graphql.md) - GraphQL queries
