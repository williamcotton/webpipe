# Testing & BDD

Web Pipe provides a comprehensive BDD-style testing framework built directly into `.wp` files. Tests use `describe` and `it` blocks with mocking capabilities, test variables, and powerful assertions including DOM selectors.

---

## Test Structure

### Basic Syntax

```wp
describe "Feature Name"
  let userId = 1

  with mock pg returning `{ "rows": [{ "id": 1 }] }`

  it "does something specific"
    when calling GET /path?query
    with headers `{ "x-user-id": "1" }`
    then status is 200
    and output equals `{ "result": "expected" }`
```

### Components

- **`describe "name"`** - Groups related tests and declares shared mocks and variables
- **`let varName = value`** - Define test variables accessible in all tests
- **`it "does thing"`** - Defines a single test case
- **`with mock ...`** - Declare mocks at describe or test level
- **`when calling/executing`** - Trigger the test
- **`with headers/body/input`** - Provide test data
- **`then`** and **`and`** - Assert expectations

---

## Test Variables with `let`

Define variables in `describe` blocks that are accessible in all assertions:

```wp
describe "User API"
  let userId = 1
  let userName = "Alice"
  let userEmail = "alice@example.com"

  it "returns user data"
    when calling GET /users/{{userId}}
    then output `.name` equals "{{userName}}"
    and output `.email` equals "{{userEmail}}"
```

**Variable Interpolation:**
- Use `{{varName}}` in paths: `GET /users/{{userId}}`
- Use `$varName` in backtick expressions: `` `{ id: $userId }` ``
- Use `"{{varName}}"` in string assertions: `equals "{{userName}}"`

**Example with Multiple Variables:**
```wp
describe "Todo App"
  let userId = 2
  let todoId = 5
  let todoTitle = "New todo"

  it "creates todo"
    when calling POST /todos
    and with body `{ title: $todoTitle }`
    then call mutation createTodo with `{
      userId: $userId,
      title: $todoTitle
    }`
```

---

## Test Types

### 1. Route Calls

Test HTTP endpoints directly:

```wp
when calling GET /path?query
when calling POST /users
when calling PUT /users/:id
when calling DELETE /users/{{userId}}
```

**With Headers:**
```wp
when calling GET /todos
with headers `{
  "x-user-id": ($userId | tostring),
  "authorization": "Bearer token123"
}`
```

**With Body:**
```wp
when calling POST /users
with headers `{ "content-type": "application/json" }`
and with body `{
  name: $userName,
  email: $userEmail
}`
```

**Complete Example:**
```wp
it "creates a user"
  when calling POST /users
  with headers `{
    "content-type": "application/json",
    "authorization": "Bearer token"
  }`
  and with body `{
    name: "Alice",
    email: "alice@example.com"
  }`
  then status is 201
  and output `.id` equals 1
```

### 2. Pipeline Execution

Test pipelines in isolation:

```wp
when executing pipeline getTeams
with input `{ "params": { "id": 2 } }`
```

**Example:**
```wp
describe "getTeams pipeline"
  with mock pg returning `{ "rows": [{ "id": 2, "name": "Growth" }] }`

  it "transforms params and queries database"
    when executing pipeline getTeams
    with input `{ "params": { "id": 2 } }`
    then output `.rows[0].name` equals "Growth"
```

### 3. Variable Execution

Test middleware variables directly:

```wp
when executing variable pg teamsQuery
with input `{ "sqlParams": [] }`
```

---

## Mocking

### Middleware Mocking

**Describe-level (Shared):**
```wp
describe "Feature"
  with mock pg returning `{ "rows": [{ "id": 1 }] }`
```

**Variable-level:**
```wp
with mock pg.teamsQuery returning `{ "rows": [{ "id": 1 }] }`
```

**Pipeline-level:**
```wp
with mock pipeline loadUser returning `{ "user": { "id": 42 } }`
```

**Test-level (Override):**
```wp
it "overrides describe mock"
  when calling GET /users
  and mock pg returning `{ "rows": [{ "id": 2 }] }`
  then output `.rows[0].id` equals 2
```

### GraphQL Resolver Mocking

Mock GraphQL queries and mutations using `query` and `mutation` keywords:

**Mocking Query Resolvers:**
```wp
describe "Todo App"
  let userId = 1

  with mock query todos returning `[
    {
      id: 1,
      title: "First todo",
      completed: false,
      userId: $userId,
      createdAt: "2024-01-01T00:00:00Z",
      updatedAt: "2024-01-01T00:00:00Z"
    },
    {
      id: 2,
      title: "Second todo",
      completed: true,
      userId: $userId,
      createdAt: "2024-01-02T00:00:00Z",
      updatedAt: "2024-01-02T00:00:00Z"
    }
  ]`

  it "fetches todos"
    when calling GET /todos
    then output `.todos | length` equals 2
```

**Mocking Mutation Resolvers:**
```wp
describe "Todo Mutations"
  let userId = 2
  let newTitle = "New todo from form"

  with mock mutation createTodo returning `{
    id: 10,
    title: $newTitle,
    completed: false,
    userId: $userId,
    createdAt: "2024-01-02T00:00:00Z",
    updatedAt: "2024-01-02T00:00:00Z"
  }`

  it "creates todo"
    when calling POST /todos
    and with body `{ title: $newTitle }`
    then status is 201
```

**Multiple Resolver Mocks:**
```wp
describe "Todo App - Complete Flow"
  with mock mutation createTodo returning `{ id: 10, title: "New" }`
  with mock mutation toggleTodo returning `{ id: 10, completed: true }`
  with mock mutation deleteTodo returning `true`
  with mock query todos returning `[]`

  it "performs all operations"
    # Tests will use the mocked resolvers
```

### Named Result Mocking

If a step uses `resultName: "X"` or `@result(X)`, target the mock:

```wp
with mock pg.users returning `{ "rows": [{ "id": 1 }] }`
```

---

## Assertions

### Status Assertions

```wp
then status is 200
then status in 200..299
then status is 404
```

### Content-Type Assertions

```wp
then contentType is "text/html"
then contentType is "application/json"
```

### Output Assertions

**Exact equality:**
```wp
then output equals `{ "a": 1 }`
```

**Partial matching:**
```wp
then output contains `{ "a": 1 }`
```

**Regex matching:**
```wp
then output matches `^<p>hello, .*</p>$`
```

**JQ path assertions:**
```wp
then output `.rows[0].id` equals 42
then output `.users | length` equals 3
then output `.teams | map(.id)` equals `[1, 2, 3]`
```

**Using Variables:**
```wp
let userId = 1
let userName = "Alice"

then output `.id` equals $userId
and output `.name` equals "{{userName}}"
```

---

## Selector Assertions (HTML/DOM Testing)

Test HTML responses using CSS selectors:

### Basic Selector Existence

```wp
then selector `.todo-list` exists
and selector `form[action="/todos"]` exists
and selector `input[name="title"]` exists
```

### Selector Count

```wp
then selector `.todo-item` count equals 3
and selector `.completed` count equals 1
```

### Selector Text Content

```wp
then selector `.title` text equals "First todo"
and selector `h1` text equals "{{pageTitle}}"
```

### Nth-child Selectors

```wp
then selector `.todo-list > .todo-item:nth-child(1) .title` text equals "First todo"
and selector `.todo-list > .todo-item:nth-child(2) .title` text equals "Second todo"
```

### Complete HTML Testing Example

```wp
describe "Todo Page Rendering"
  let userId = 1

  with mock query todos returning `[
    {
      id: 1,
      title: "First todo",
      completed: false,
      userId: $userId,
      createdAt: "2024-01-01T00:00:00Z",
      updatedAt: "2024-01-01T00:00:00Z"
    },
    {
      id: 2,
      title: "Second todo",
      completed: true,
      userId: $userId,
      createdAt: "2024-01-02T00:00:00Z",
      updatedAt: "2024-01-02T00:00:00Z"
    }
  ]`

  it "renders todos with forms"
    when calling GET /todos
    with headers `{
      "x-user-id": ($userId | tostring)
    }`
    then status is 200
    and selector `form[action="/todos"]` exists
    and selector `input[name="title"]` exists
    and selector `.todo-list > .todo-item` count equals 2
    and selector `.todo-list > .todo-item:nth-child(1) .title` text equals "First todo"
    and selector `.todo-list > .todo-item:nth-child(1) form[action="/todos/1/toggle"]` exists
    and selector `.todo-list > .todo-item:nth-child(1) form[action="/todos/1/delete"]` exists
    and selector `.todo-list > .todo-item:nth-child(2) .title` text equals "Second todo"
```

### Testing Handlebars Templates

```wp
describe "todoPageTemplate pipeline"
  let todoTitle = "Pipeline todo"

  it "renders todos into HTML"
    when executing pipeline todoPageTemplate
    with input `{
      todos: [
        { id: 1, title: $todoTitle, completed: false }
      ]
    }`
    then selector `.todo-list` exists
    and selector `.todo-item` exists
    and selector `.todo-item .title` text equals "{{todoTitle}}"
    and selector `form[action="/todos/1/toggle"]` exists
    and selector `form[action="/todos/1/delete"]` exists
```

---

## Verifying GraphQL Calls

Assert that specific GraphQL queries or mutations were called with expected arguments:

### Verifying Query Calls

```wp
and call query todos with `{
  userId: $userId
}`
```

**Example:**
```wp
describe "GraphQL Query Verification"
  let userId = 1

  with mock query todos returning `[]`

  it "calls todos query with correct userId"
    when calling GET /todos
    with headers `{ "x-user-id": ($userId | tostring) }`
    then call query todos with `{
      userId: $userId
    }`
```

### Verifying Mutation Calls

```wp
and call mutation createTodo with `{
  userId: $userId,
  title: $newTitle
}`
```

**Example:**
```wp
describe "GraphQL Mutation Verification"
  let userId = 2
  let newTitle = "New todo"

  with mock mutation createTodo returning `{ id: 10, title: $newTitle }`

  it "calls createTodo mutation with correct args"
    when calling POST /todos
    and with body `{ title: $newTitle }`
    then status is 201
    and call mutation createTodo with `{
      userId: $userId,
      title: $newTitle
    }`
```

### Complete Mutation Flow Example

```wp
describe "Todo Mutations via Forms"
  let userId = 2
  let newTitle = "New todo from form"
  let todoId = 5

  with mock mutation createTodo returning `{
    id: 10,
    title: $newTitle,
    completed: false,
    userId: $userId
  }`

  with mock mutation toggleTodo returning `{
    id: $todoId,
    completed: true,
    userId: $userId
  }`

  with mock mutation deleteTodo returning `true`

  it "creates a todo"
    when calling POST /todos
    with headers `{
      "x-user-id": ($userId | tostring),
      "content-type": "application/json"
    }`
    and with body `{ title: $newTitle }`
    then status is 303
    and call mutation createTodo with `{
      userId: $userId,
      title: $newTitle
    }`

  it "toggles a todo"
    when calling POST /todos/{{todoId}}/toggle
    with headers `{ "x-user-id": ($userId | tostring) }`
    then status is 303
    and call mutation toggleTodo with `{
      userId: $userId,
      id: $todoId
    }`

  it "deletes a todo"
    when calling POST /todos/{{todoId}}/delete
    with headers `{ "x-user-id": ($userId | tostring) }`
    then status is 303
    and call mutation deleteTodo with `{
      userId: $userId,
      id: $todoId
    }`
```

---

## Testing Control Flow

### Testing If/Else Blocks

Test both branches by providing different input:

```wp
describe "if/else routing"
  it "executes then branch when condition is true"
    when executing pipeline conditionalRoute
    with input `{ "value": 150 }`
    then output `.bonus` equals "high value"

  it "executes else branch when condition is false"
    when executing pipeline conditionalRoute
    with input `{ "value": 50 }`
    then output equals `{ "value": 50 }`
```

### Testing Dispatch Blocks

```wp
describe "Dispatch Feature"
  it "routes based on feature flags"
    when calling GET /feature-test-dispatch
    then status is 200
    and output `.message` contains "feature"

  it "handles environment-based routing"
    when calling GET /env-dispatch
    then status is 200

  it "handles negated tags"
    when calling GET /non-production-dispatch
    then status is 200
```

### Testing Foreach Loops

```wp
describe "Foreach Feature"
  it "transforms each item in an array"
    when calling GET /test-foreach-basic
    then status is 200
    and output `.teams | length` equals 3
    and output `.teams[0].processed` equals true
    and output `.teams[0].uppercase_name` equals "ALPHA TEAM"
```

### Testing @guard Tags

```wp
describe "@guard tag"
  it "executes step when guard condition is true"
    when executing pipeline adminRoute
    with input `{ "user": { "role": "admin" } }`
    then output `.adminData` equals "loaded"

  it "skips step when guard condition is false"
    when executing pipeline adminRoute
    with input `{ "user": { "role": "guest" } }`
    then output `.adminData` equals null
```

### Testing @when Conditions

```wp
describe "@when conditions"
  it "routes to admin branch when is_admin is true"
    when calling GET /admin-check
    then status is 200
    and output `.access` equals "granted"

  it "routes to default when condition is false"
    when calling GET /guest-check
    then status is 200
    and output `.access` equals "denied"
```

---

## Testing Async Routes

### Mocking Async Tasks

```wp
describe "async dashboard"
  with mock fetch.user returning `{
    "response": { "name": "Alice", "email": "alice@example.com" }
  }`
  with mock fetch.posts returning `{
    "response": [
      { "id": 1, "title": "First Post" },
      { "id": 2, "title": "Second Post" }
    ]
  }`

  it "aggregates async results"
    when calling GET /dashboard
    then status is 200
    and output `.user.name` equals "Alice"
    and output `.posts | length` equals 2
```

### Testing Join Middleware

```wp
describe "parallel data fetching"
  with mock fetch.api1 returning `{ "response": { "data": "api1" } }`
  with mock fetch.api2 returning `{ "response": { "data": "api2" } }`

  it "joins multiple async tasks"
    when calling GET /parallel-fetch
    then status is 200
    and output `.api1Data` equals "api1"
    and output `.api2Data` equals "api2"
```

---

## Advanced Mocking Patterns

### Mocking with @result Tags

```wp
describe "named result mocking"
  with mock pg.users returning `{
    "rows": [{ "id": 1, "name": "Alice" }],
    "rowCount": 1
  }`

  it "mocks @result named queries"
    when executing pipeline getUserProfile
    with input `{ "userId": 1 }`
    then output `.data.users.rows[0].name` equals "Alice"
```

### Mocking Multiple Named Results

```wp
describe "multiple result mocking"
  with mock pg.users returning `{ "rows": [{ "id": 1 }] }`
  with mock pg.posts returning `{ "rows": [{ "id": 100 }] }`
  with mock pg.comments returning `{ "rows": [{ "id": 200 }] }`

  it "aggregates multiple named results"
    when calling GET /dashboard
    then output `.data.users.rowCount` equals 1
    and output `.data.posts.rowCount` equals 1
    and output `.data.comments.rowCount` equals 1
```

### Mocking GraphQL Resolvers

```wp
describe "GraphQL resolver mocking"
  with mock query user returning `{
    id: 1,
    name: "Alice",
    email: "alice@example.com"
  }`

  with mock query userPosts returning `[
    { id: 100, title: "Post 1" },
    { id: 101, title: "Post 2" }
  ]`

  it "mocks multiple GraphQL queries"
    when calling GET /user/1/full-profile
    then output `.user.name` equals "Alice"
    and output `.posts | length` equals 2
```

---

## Running Tests

Run all tests in a Web Pipe file:

```bash
cargo run app.wp --test
```

Run tests for a specific file:

```bash
cargo run myroutes.wp --test
```

---

## Complete Examples

### Testing HTML Forms with GraphQL Backend

```wp
describe "Simple Todo App"
  let userId = 1
  let todoTitle = "Test todo"

  with mock query todos returning `[
    {
      id: 1,
      title: $todoTitle,
      completed: false,
      userId: $userId,
      createdAt: "2024-01-01T00:00:00Z",
      updatedAt: "2024-01-01T00:00:00Z"
    }
  ]`

  it "renders todo list with forms"
    when calling GET /todos
    with headers `{ "x-user-id": ($userId | tostring) }`
    then status is 200
    and selector `.todo-list` exists
    and selector `.todo-item` count equals 1
    and selector `.todo-item .title` text equals "{{todoTitle}}"
    and call query todos with `{ userId: $userId }`
```

### Testing REST API with Variables

```wp
describe "User API"
  let userId = 42
  let userName = "Alice"
  let userEmail = "alice@example.com"

  with mock pg returning `{
    "rows": [{
      "id": $userId,
      "name": $userName,
      "email": $userEmail
    }]
  }`

  it "fetches user by ID"
    when calling GET /users/{{userId}}
    then status is 200
    and output `.id` equals $userId
    and output `.name` equals "{{userName}}"
    and output `.email` equals "{{userEmail}}"
```

### Testing Mutations with Verification

```wp
describe "User Creation"
  let userName = "Bob"
  let userEmail = "bob@example.com"

  with mock pg returning `{
    "rows": [{ "id": 1, "name": $userName, "email": $userEmail }]
  }`

  it "creates user via POST"
    when calling POST /users
    with headers `{ "content-type": "application/json" }`
    and with body `{
      name: $userName,
      email: $userEmail
    }`
    then status is 201
    and output `.id` equals 1
    and output `.name` equals "{{userName}}"
```

---

## Testing DataLoaders and GraphQL Resolvers

### Testing DataLoader Batching

Test that DataLoaders properly batch queries to prevent N+1 problems:

```wp
describe "DataLoader N+1 Prevention"
  it "batches employee lookups when resolving teams"
    when calling POST /graphql
    with headers `{ "content-type": "application/json" }`
    with body `{ "query": "query { teams { id name employees { id name } } }" }`
    then status is 200
    and output `.data.teams | type` equals `"array"`
    and output `.data.teams[0].employees | type` equals `"array"`
    and output `.data.teams[0].employees[0].name | type` equals `"string"`
```

### Testing Loader Pipelines Directly

Test loader pipelines with batched inputs:

```wp
describe "EmployeesByTeamLoader"
  it "returns employees keyed by team ID"
    when executing pipeline EmployeesByTeamLoader
    with input `{
      keys: [1, 2, 3],
      args: { limit: 5 }
    }`
    then output `.["1"] | type` equals `"array"`
    and output `.["1"][0].teamId` equals `"1"`
    and output `.["2"] | type` equals `"array"`
    and output `.["3"] | type` equals `"array"`
```

### Mocking Nested Resolvers

Mock GraphQL resolvers that use DataLoaders:

```wp
describe "Team with Employees"
  let teamId = 1

  with mock query teams returning `[{
    id: $teamId,
    name: "Engineering"
  }]`

  # Mock the loader result
  with mock pipeline EmployeesByTeamLoader returning `{
    "1": [
      { id: "10", name: "Alice", teamId: "1" },
      { id: "11", name: "Bob", teamId: "1" }
    ]
  }`

  it "resolves nested employees field"
    when calling POST /graphql
    with headers `{ "content-type": "application/json" }`
    and with body `{ "query": "query { teams { id name employees { id name } } }" }`
    then status is 200
    and output `.data.teams[0].employees | length` equals 2
    and output `.data.teams[0].employees[0].name` equals "Alice"
```

### Testing Resolver with Field Arguments

Test GraphQL field resolvers that accept arguments:

```wp
describe "Team Employees with Limit"
  with mock query teams returning `[{ id: "1", name: "Engineering" }]`

  with mock pipeline EmployeesByTeamLoader returning `{
    "1": [
      { id: "10", name: "Alice", teamId: "1" }
    ]
  }`

  it "passes limit argument to loader"
    when calling POST /graphql
    with headers `{ "content-type": "application/json" }`
    and with body `{ "query": "query { teams { employees(limit: 1) { name } } }" }`
    then status is 200
    and call pipeline EmployeesByTeamLoader with `{
      keys: ["1"],
      args: { limit: 1 }
    }`
```

### Testing Multi-Level Nested Resolvers

Test deeply nested DataLoader chains:

```wp
describe "Users -> Posts -> Comments"
  with mock query users returning `[{ id: "1", name: "Alice" }]`

  with mock pipeline PostsByUserLoader returning `{
    "1": [{ id: "100", title: "Post 1", userId: "1" }]
  }`

  with mock pipeline CommentsByPostLoader returning `{
    "100": [{ id: "500", text: "Comment 1", postId: "100" }]
  }`

  it "resolves 3 levels of nesting efficiently"
    when calling POST /graphql
    with headers `{ "content-type": "application/json" }`
    and with body `{ "query": "query { users { posts { comments { text } } } }" }`
    then status is 200
    and output `.data.users[0].posts[0].comments[0].text` equals "Comment 1"
    # Verify batching occurred
    and call pipeline PostsByUserLoader with `{ keys: ["1"] }`
    and call pipeline CommentsByPostLoader with `{ keys: ["100"] }`
```

### Testing Loader Error Handling

Test how loaders handle missing keys:

```wp
describe "UserLoader with Missing Keys"
  it "returns null for missing users"
    when executing pipeline UserLoader
    with input `{
      keys: [999, 1000, 1001]
    }`
    then output `.["999"]` equals null
    and output `.["1000"]` equals null
    and output `.["1001"]` equals null
```

---

## Best Practices

### 1. Use `let` for Reusable Test Data

Define variables once, use everywhere:

```wp
describe "User Tests"
  let userId = 1
  let userName = "Alice"

  it "test 1"
    # Use $userId and "{{userName}}"
```

### 2. Test Each Branch

For dispatch/if-else, test all cases:

```wp
describe "feature dispatch"
  it "tests experimental case" ...
  it "tests production case" ...
  it "tests default case" ...
```

### 3. Mock All Dependencies

Mock every external dependency:

```wp
with mock pg returning `{ "rows": [] }`
with mock fetch returning `{ "response": {} }`
with mock query todos returning `[]`
```

### 4. Use Descriptive Test Names

```wp
it "returns 404 when user not found"
it "transforms array elements with uppercase names"
it "routes to admin panel when user is admin"
```

### 5. Test HTML with Selectors

For HTML responses, verify structure:

```wp
then selector `form[action="/submit"]` exists
and selector `.error-message` count equals 0
and selector `h1` text equals "{{expectedTitle}}"
```

### 6. Verify GraphQL Calls

Assert resolvers were called correctly:

```wp
then call query todos with `{ userId: $userId }`
and call mutation createTodo with `{ title: $newTitle }`
```

---

## See Also

- [Flow Control](./flow-control.md) - If/else, dispatch, foreach syntax
- [Routes & Pipelines](./routes-and-pipelines.md) - Named results and @result tags
- [GraphQL](./graphql.md) - GraphQL schema and resolvers
- [Concurrency](./concurrency.md) - Async execution and join middleware
- [DSL Syntax](./dsl-syntax.md) - Complete syntax reference
