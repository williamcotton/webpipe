# DSL Syntax

Complete reference for the Web Pipe DSL (Domain-Specific Language). This guide covers all syntactic elements: config blocks, variables, pipelines, routes, tags, control flow, and comments.

---

## Config Blocks

Config blocks define configuration for specific middleware types.

### Syntax

```wp
config <middleware-name> {
  key: value
  key: $ENV_VAR || "default"
}
```

### Environment Variable Substitution

Use `$ENV_VAR || "default"` to read environment variables with fallback values:

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

### Common Config Blocks

**PostgreSQL:**
```wp
config pg {
  host: $WP_PG_HOST || "localhost"
  port: $WP_PG_PORT || "5432"
  database: $WP_PG_DATABASE || "mydb"
  user: $WP_PG_USER || "postgres"
  password: $WP_PG_PASSWORD || "postgres"
  ssl: false
}
```

**Authentication:**
```wp
config auth {
  sessionTtl: 604800
  cookieName: "wp_session"
  cookieSecure: false
  cookieHttpOnly: true
  cookieSameSite: "Lax"
  cookiePath: "/"
}
```

**Caching:**
```wp
config cache {
  enabled: true
  defaultTtl: 60
  maxCacheSize: 10485760
}
```

**GraphQL:**
```wp
config graphql {
  endpoint: "/graphql"
}
```

**Logging:**
```wp
config log {
  enabled: true
  format: "json"
  level: "info"
  includeBody: false
  includeHeaders: true
  maxBodySize: 1024
  timestamp: true
}
```

---

## Variables

Variables are named code snippets that can be reused across routes and pipelines.

### Syntax

```wp
<middleware-name> <variable-name> = `<code>`
```

### SQL Variables

```wp
pg teamsQuery = `SELECT * FROM teams`
pg userQuery = `SELECT * FROM users WHERE id = $1`
pg insertOrder = `INSERT INTO orders (user_id, product_id) VALUES ($1, $2) RETURNING *`
```

### Handlebars Template Variables

```wp
handlebars header = `<h1>{{title}}</h1>`
handlebars footer = `<footer>{{year}}</footer>`
handlebars card = `
  <div class="card">
    <h3>{{title}}</h3>
    <p>{{description}}</p>
  </div>
`
```

### GraphQL Query Variables

```wp
graphql getUserQuery = `
  query($id: ID!) {
    user(id: $id) {
      name
      email
    }
  }
`

graphql createUserMutation = `
  mutation($name: String!, $email: String!) {
    createUser(name: $name, email: $email) {
      id
      name
    }
  }
`
```

### Using Variables in Routes

```wp
GET /teams
  |> jq: `{ sqlParams: [] }`
  |> pg: teamsQuery

GET /users/:id
  |> jq: `{ sqlParams: [.params.id] }`
  |> pg: userQuery
```

### Auto-Naming with Variables

When you use a variable with middleware, the result is automatically captured under `.data.<variableName>`:

```wp
pg getUserQuery = `SELECT * FROM users WHERE id = $1`

GET /user/:id
  |> jq: `{ sqlParams: [.params.id] }`
  |> pg: getUserQuery
  |> jq: `{ user: .data.getUserQuery.rows[0] }`  # Auto-named!
```

---

## Pipelines

Pipelines are reusable sequences of middleware steps.

### Syntax

```wp
pipeline <name> =
  |> <middleware1>
  |> <middleware2>
  |> <middleware3>
```

### Basic Pipeline

```wp
pipeline getTeams =
  |> jq: `{ sqlParams: [.params.id] }`
  |> pg: `SELECT * FROM teams WHERE id = $1`
```

### Pipeline Invocation

```wp
GET /teams/:id
  |> pipeline: getTeams
  |> jq: `{ team: .data.rows[0] }`
```

### Pipeline Invocation with Arguments

Pass arguments to pipelines:

```wp
pipeline getTeam =
  |> pg: `SELECT * FROM teams WHERE id = $1`

GET /team/:id
  |> pipeline({ sqlParams: [.params.id] }): getTeam
  |> jq: `{ team: .data.rows[0] }`
```

**Syntax:**
```wp
|> pipeline(<object-expression>): <pipeline-name>
```

The object is merged into the pipeline input.

### Complex Pipelines

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
    authError(401):
      |> jq: `{ success: false, error: "Invalid credentials" }`
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

## Routes

Routes define HTTP endpoints and their processing pipelines.

### Syntax

```wp
<METHOD> <path>
  |> <middleware1>
  |> <middleware2>
  |> <middleware3>
```

### HTTP Methods

```wp
GET /resource
POST /resource
PUT /resource/:id
DELETE /resource/:id
PATCH /resource/:id
```

### Path Parameters

Use `:paramName` for dynamic URL segments:

```wp
GET /users/:id
  |> jq: `{ userId: .params.id }`

GET /posts/:postId/comments/:commentId
  |> jq: `{
    postId: .params.postId,
    commentId: .params.commentId
  }`
```

### Query String Access

Access query parameters via `.query`:

```wp
GET /search
  |> jq: `{
    term: .query.q,
    limit: (.query.limit // 10 | tonumber),
    offset: (.query.offset // 0 | tonumber)
  }`
```

### Request Body Access

Access POST/PUT body via `.body`:

```wp
POST /users
  |> jq: `{
    name: .body.name,
    email: .body.email,
    age: .body.age
  }`
```

### Headers Access

Access request headers via `.headers`:

```wp
GET /api/data
  |> jq: `{
    authorization: .headers.authorization,
    userAgent: .headers["user-agent"]
  }`
```

---

## Tags

Tags modify the behavior of individual pipeline steps.

### Environment Tags (`@env`)

Execute steps only in specific environments:

```wp
|> log: `level: debug` @env(development)
|> cache: `ttl: 3600` @env(production)
|> pipeline: secureHeaders @env(production)
```

**Negated:**
```wp
|> log: `level: trace` @!env(production)
```

### Feature Flag Tags (`@flag`)

Execute steps based on feature flags:

```wp
|> pipeline: newFeature @flag(experimental)
|> pipeline: oldFeature @!flag(experimental)
```

**Multiple Flags (AND logic):**
```wp
|> pipeline: betaStaffFeature @flag(beta,staff)
```

### Dynamic Condition Tags (`@when`)

Execute steps based on runtime conditions set by `setWhen()`:

```wp
|> lua: `setWhen("is_admin", request.user and request.user.role == "admin"); return request`
|> jq: `{ message: "Admin panel" }` @when(is_admin)
|> jq: `{ message: "Regular user" }` @!when(is_admin)
```

**Multiple Conditions (AND logic):**
```wp
|> jq: `{ access: "granted" }` @when(authenticated,verified)
```

### Guard Tags (`@guard`)

Execute steps based on JQ expression evaluation:

```wp
|> jq: `{ adminData: "loaded" }` @guard(`.user.role == "admin"`)
|> jq: `{ debugInfo: "..." }` @!guard(`.env == "production"`)
|> jq: `{ premium: true }` @guard(`.user.premium and .user.verified`)
```

### Async Tags (`@async`)

Mark steps for parallel execution:

```wp
|> fetch: `https://api.example.com/users` @async(users)
|> fetch: `https://api.example.com/posts` @async(posts)
|> join: `users,posts`
```

### Result Naming Tags (`@result`)

Capture middleware output under a specific name:

```wp
|> pg: `SELECT * FROM users` @result(users)
|> pg: `SELECT * FROM posts` @result(posts)
|> jq: `{ users: .data.users.rows, posts: .data.posts.rows }`
```

### Tag Combinations

Multiple tags can be combined on one step:

```wp
|> pg: `SELECT * FROM data` @env(production) @result(prodData)
|> fetch: `https://api.example.com/data` @async(apiData) @flag(external-api)
```

### Boolean Expressions in Tags

Tags support `and` / `or` / parentheses:

```wp
case @when(authenticated) and @when(admin):
case @when(mobile) or @when(tablet):
case (@when(admin) or @when(staff)) and @when(on_duty):
case @when(authenticated) and @!when(banned):
```

**Precedence:**
- `and` binds tighter than `or`
- Use parentheses for explicit grouping

---

## Control Flow Blocks

### If/Else Blocks

Conditional execution based on runtime data:

```wp
|> if
  |> jq: `.user.role == "admin"`
  then:
    |> jq: `{ access: "full" }`
  else:
    |> jq: `{ access: "limited" }`
  end
```

**Optional `else`:**
```wp
|> if
  |> jq: `.value > 100`
  then:
    |> jq: `. + { bonus: "high value" }`
  # No else - state passes through unchanged
```

### Dispatch Blocks

Switch-like branching based on tags:

```wp
|> dispatch
  case @flag(experimental):
    |> jq: `{ version: "experimental" }`
  case @env(production):
    |> jq: `{ version: "stable", env: "production" }`
  default:
    |> jq: `{ version: "standard" }`
```

**Boolean expressions in cases:**
```wp
|> dispatch
  case @when(authenticated) and @when(admin):
    |> jq: `{ access: "admin_panel" }`
  case @when(authenticated):
    |> jq: `{ access: "user_dashboard" }`
  default:
    |> jq: `{ access: "login_required" }`
```

### Foreach Loops

Iterate over arrays, transforming each element:

```wp
|> foreach data.rows
  |> jq: `. + { processed: true }`
  |> jq: `. + { uppercase: (.name | ascii_upcase) }`
end
```

### Result Blocks

Route execution based on error types:

```wp
|> result
  ok(200):
    |> jq: `{ success: true, data: . }`
  validationError(400):
    |> jq: `{ error: "Validation failed", details: .errors }`
  authRequired(401):
    |> jq: `{ error: "Authentication required" }`
  default(500):
    |> jq: `{ error: "Internal server error" }`
```

---

## Inline Arguments

Pass dynamic data directly to middleware calls.

### Array Parameters (pg)

```wp
|> pg([.userId, .status]): `SELECT * FROM users WHERE id = $1 AND status = $2`
```

### Object Parameters (graphql)

```wp
|> graphql({ userId: .targetId, limit: 10 }): `query($userId: ID!, $limit: Int) { ... }`
```

### String Arguments (fetch, log)

```wp
|> fetch("https://api.example.com/users/" + (.userId | tostring))
|> log("Processing request for user " + .userId)
```

### Loader Arguments (GraphQL DataLoaders)

Pass parent field value to DataLoader pipelines:

```wp
resolver Team.employees =
  |> loader(.parent.id): EmployeesByTeamLoader
```

**With field arguments:**
```wp
resolver Team.employees =
  |> loader(.parent.id, { limit: .args.limit }): EmployeesByTeamLoader
```

See [Inline Arguments](./inline-arguments.md) for detailed documentation.

---

## Comments

Use `#` for line comments:

```wp
# This is a comment
GET /users/:id
  # Fetch user data
  |> pg: `SELECT * FROM users WHERE id = $1`
  # Transform response
  |> jq: `{ user: .data.rows[0] }`
```

**Multi-line strings** use backticks:

```wp
handlebars template = `
  <html>
    <head><title>{{title}}</title></head>
    <body>{{content}}</body>
  </html>
`
```

---

## Complete Example

Putting it all together:

```wp
# Configuration
config pg {
  host: $WP_PG_HOST || "localhost"
  database: $WP_PG_DATABASE || "mydb"
}

config auth {
  sessionTtl: 604800
  cookieName: "wp_session"
}

# Variables
pg usersQuery = `SELECT * FROM users WHERE status = $1`
handlebars userCard = `
  <div class="user-card">
    <h3>{{name}}</h3>
    <p>{{email}}</p>
  </div>
`

# Feature flags pipeline
pipeline featureFlags =
  |> lua: `
    setFlag("experimental-ui", math.random() < 0.5)
    return request
  `

# Reusable pipeline
pipeline authenticateAdmin =
  |> auth: "required"
  |> lua: `
    setWhen("is_admin", request.user and request.user.role == "admin")
    return request
  `
  |> result
    ok(200):
      |> dispatch
        case @when(is_admin):
          |> jq: `{ access: "granted" }`
        default:
          |> jq: `{ error: "Admin access required", status: 403 }`
    authRequired(401):
      |> jq: `{ error: "Authentication required" }`

# Routes
GET /users
  |> pg(["active"]): usersQuery @result(users) @env(production)
  |> pg(["all"]): usersQuery @result(users) @!env(production)
  |> foreach data.users.rows
    |> jq: `. + { fullName: .firstName + " " + .lastName }`
  end
  |> jq: `{ users: .data.users.rows }`

GET /admin/dashboard
  |> pipeline: authenticateAdmin
  |> pg: `SELECT COUNT(*) as total FROM users` @result(stats)
  |> jq: `{
    message: "Admin Dashboard",
    userCount: .data.stats.rows[0].total,
    env: $context.env
  }`

POST /users
  |> validate: `{
    name: string(3..100),
    email: email,
    age: number
  }`
  |> pg([.body.name, .body.email, .body.age]): `
    INSERT INTO users (name, email, age)
    VALUES ($1, $2, $3)
    RETURNING *
  `
  |> result
    ok(201):
      |> jq: `{ success: true, user: .data.rows[0] }`
    validationError(400):
      |> jq: `{ error: "Validation failed", details: .errors }`

# Async/parallel example
GET /dashboard
  |> fetch: `https://api.example.com/users/1` @async(user)
  |> fetch: `https://api.example.com/posts?userId=1` @async(posts)
  |> jq: `{ title: "Dashboard", timestamp: now }`
  |> join: `user,posts`
  |> jq: `{
    title: .title,
    loadedAt: .timestamp,
    user: .async.user.data.response,
    posts: .async.posts.data.response
  }`
  |> handlebars: `
    <html>
      <head><title>{{title}}</title></head>
      <body>
        <h1>Welcome, {{user.name}}</h1>
        <ul>
          {{#each posts}}
            <li>{{this.title}}</li>
          {{/each}}
        </ul>
      </body>
    </html>
  `
```

---

## Syntax Summary

### Top-Level Constructs

| Construct | Syntax |
|-----------|--------|
| Config | `config <name> { key: value }` |
| Variable | `<middleware> <name> = \`code\`` |
| Pipeline | `pipeline <name> = \|> ...` |
| Route | `<METHOD> <path> \|> ...` |
| Comment | `# comment` |

### Tags

| Tag | Purpose |
|-----|---------|
| `@env(name)` | Environment-specific execution |
| `@!env(name)` | Negated environment check |
| `@flag(name)` | Feature flag check |
| `@flag(f1,f2)` | Multiple flags (AND) |
| `@!flag(name)` | Negated flag check |
| `@when(name)` | Dynamic condition check |
| `@!when(name)` | Negated condition check |
| `@guard(\`expr\`)` | JQ expression guard |
| `@!guard(\`expr\`)` | Negated guard |
| `@async(name)` | Parallel execution |
| `@result(name)` | Name result |

### Control Flow

| Construct | Syntax |
|-----------|--------|
| If/Else | `\|> if ... then: ... else: ... end` |
| Dispatch | `\|> dispatch case @flag(): ... default: ...` |
| Foreach | `\|> foreach <path> ... end` |
| Result | `\|> result ok(200): ... default(500): ...` |

### Access Patterns

| Pattern | Example |
|---------|---------|
| Path param | `.params.id` |
| Query param | `.query.q` |
| Request body | `.body.field` |
| Headers | `.headers.authorization` |
| Context flags | `$context.flags.name` |
| Context conditions | `$context.conditions.name` |
| Context env | `$context.env` |

---

## See Also

- [Flow Control](./flow-control.md) - If/else, dispatch, foreach, tags
- [Inline Arguments](./inline-arguments.md) - Passing arguments to middleware
- [Context & Metadata](./context.md) - Accessing $context
- [Routes & Pipelines](./routes-and-pipelines.md) - Route and pipeline patterns
- [Testing & BDD](./testing-bdd.md) - Testing syntax
