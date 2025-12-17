# Result Routing

Result routing allows you to shape HTTP responses based on error types, status codes, and custom conditions. Web Pipe provides powerful tools for controlling response status, headers, cookies, and content type.

---

## Result Blocks

Result blocks route execution through different branches based on error types detected in the `.errors` array.

### Basic Syntax

```wp
|> result
  ok(200):
    |> <success-handling>
  errorType(statusCode):
    |> <error-handling>
  default(statusCode):
    |> <fallback-handling>
```

### Simple Example

```wp
GET /example
  |> jq: `{ errors: [ { type: "validationError", field: "email", message: "required" } ] }`
  |> result
    ok(200):
      |> jq: `{ ok: true }`
    validationError(400):
      |> jq: `{ error: "Validation failed", field: .errors[0].field }`
    default(500):
      |> jq: `{ error: "Unhandled", type: .errors[0].type }`
```

### How Result Blocks Work

1. **Check `.errors` array**: If `.errors` is empty or missing, route to `ok()`
2. **Match error type**: Look for a case matching `.errors[0].type`
3. **Fall through to default**: If no match found, use `default()` case
4. **Set status code**: The number in parentheses becomes the HTTP status

### Multiple Error Handlers

```wp
POST /users
  |> validate: `{
    name: string(3..100),
    email: email,
    password: string(8..100)
  }`
  |> auth: "register"
  |> result
    ok(201):
      |> jq: `{ success: true, message: "Registration successful" }`
    validationError(400):
      |> jq: `{ error: "Validation failed", details: .errors }`
    authError(409):
      |> jq: `{ error: "User already exists", email: .errors[0].context }`
    default(500):
      |> jq: `{ error: "Internal server error" }`
```

---

## Manual Response Control

You can manually set response properties using special fields in your JQ output.

### Setting Status Code

Use the `status` field to override the HTTP status:

```wp
GET /custom-status
  |> jq: `{
    status: 403,
    message: "Forbidden",
    reason: "Insufficient permissions"
  }`
```

### Setting Content-Type

Use the `contentType` field to set the response content type:

```wp
GET /xml-response
  |> jq: `{
    contentType: "application/xml",
    body: "<response><message>Hello XML</message></response>"
  }`
```

**Common Content Types:**
- `"application/json"` (default)
- `"text/html"`
- `"text/plain"`
- `"application/xml"`
- `"application/octet-stream"`

### Setting Custom Headers

Use the `headers` field to set response headers:

```wp
GET /with-headers
  |> jq: `{
    headers: {
      "X-Custom-Header": "custom-value",
      "Cache-Control": "no-cache",
      "X-Request-ID": .requestId
    },
    data: { message: "Response with custom headers" }
  }`
```

### Combining Response Properties

```wp
GET /full-control
  |> jq: `{
    status: 201,
    contentType: "application/json",
    headers: {
      "Location": "/users/123",
      "X-Created-At": (now | tostring)
    },
    data: {
      id: 123,
      name: "New Resource",
      created: true
    }
  }`
```

---

## Cookie Management

Web Pipe provides powerful cookie management through the `setCookies` array field.

### Basic Cookie Setting

Use the `setCookies` array to set HTTP cookies:

```wp
GET /set-cookie
  |> jq: `{
    message: "Cookie set",
    setCookies: [
      "sessionId=abc123; Max-Age=3600"
    ]
  }`
```

### Cookie Syntax

Each cookie string follows the standard HTTP Set-Cookie format:

```
name=value; attribute1; attribute2=value
```

### Cookie Attributes

**Max-Age:**
```wp
setCookies: ["sessionId=abc123; Max-Age=3600"]  # Expires in 1 hour
```

**HttpOnly:**
```wp
setCookies: ["token=secret; HttpOnly"]  # Not accessible via JavaScript
```

**Secure:**
```wp
setCookies: ["token=secret; Secure"]  # HTTPS only
```

**SameSite:**
```wp
setCookies: ["session=id; SameSite=Strict"]  # CSRF protection
setCookies: ["tracking=id; SameSite=Lax"]     # Allow some cross-site
setCookies: ["analytics=id; SameSite=None; Secure"]  # Full cross-site (requires Secure)
```

**Path:**
```wp
setCookies: ["theme=dark; Path=/"]       # Available on all paths
setCookies: ["admin=token; Path=/admin"] # Only available under /admin
```

**Domain:**
```wp
setCookies: ["user=id; Domain=.example.com"]  # Available on all subdomains
```

### Multiple Cookies

Set multiple cookies in one response:

```wp
GET /cookies
  |> jq: `{
    message: "Multiple cookies set",
    setCookies: [
      "sessionId=abc123; HttpOnly; Secure; Max-Age=3600",
      "userId=john; Max-Age=86400",
      "theme=dark; Path=/"
    ]
  }`
```

### Complete Cookie Example

```wp
GET /login-success
  |> jq: `{
    success: true,
    user: { id: 42, name: "Alice" },
    setCookies: [
      "wp_session=eyJhbGc...; HttpOnly; Secure; SameSite=Lax; Max-Age=604800; Path=/",
      "remember_me=true; Max-Age=2592000; Path=/",
      "preferences={\"theme\":\"dark\"}; Path=/"
    ]
  }`
```

### Security Best Practices

**Production Cookies (Secure):**
```wp
GET /secure-login
  |> jq: `{
    message: "Logged in",
    setCookies: [
      "session=" + .sessionToken + "; HttpOnly; Secure; SameSite=Strict; Max-Age=3600; Path=/"
    ]
  }`
```

**Development Cookies (Less Strict):**
```wp
GET /dev-login
  |> jq: `{
    message: "Logged in (dev)",
    setCookies: [
      "session=" + .sessionToken + "; HttpOnly; SameSite=Lax; Max-Age=3600; Path=/"
    ]
  }`
```

### Cookie Security Matrix

| Attribute | Purpose | XSS Protection | CSRF Protection | MITM Protection |
|-----------|---------|----------------|-----------------|-----------------|
| HttpOnly | JS-inaccessible | ✅ Yes | No | No |
| Secure | HTTPS only | No | No | ✅ Yes |
| SameSite=Strict | Same-site only | No | ✅ Yes | No |
| SameSite=Lax | Some cross-site | No | ⚠️ Partial | No |

**Recommended for session cookies:**
```
sessionId=<token>; HttpOnly; Secure; SameSite=Strict; Max-Age=3600; Path=/
```

### Deleting Cookies

Set `Max-Age=0` to delete a cookie:

```wp
GET /logout
  |> jq: `{
    message: "Logged out",
    setCookies: [
      "wp_session=; Max-Age=0; Path=/",
      "remember_me=; Max-Age=0; Path=/"
    ]
  }`
```

### Dynamic Cookie Values

Build cookie values from request data:

```wp
POST /login
  |> auth: "login"
  |> result
    ok(200):
      |> jq: `{
        success: true,
        user: .user,
        setCookies: [
          "session=" + .sessionToken + "; HttpOnly; Secure; Max-Age=3600; Path=/",
          "userId=" + (.user.id | tostring) + "; Max-Age=86400; Path=/"
        ]
      }`
    authError(401):
      |> jq: `{ error: "Invalid credentials" }`
```

---

## Common Error Types

Web Pipe middleware produces standardized error types that can be routed in result blocks.

### Validation Errors

**Produced by:** `validate` middleware

**Error Structure:**
```json
{
  "type": "validationError",
  "field": "email",
  "rule": "email",
  "message": "must be a valid email",
  "context": "email"
}
```

**Handling:**
```wp
|> result
  validationError(400):
    |> jq: `{
      error: "Validation failed",
      field: .errors[0].field,
      message: .errors[0].message
    }`
```

### Authentication Errors

**Produced by:** `auth` middleware

**Error Structure:**
```json
{
  "type": "authError",
  "message": "Invalid credentials",
  "context": "login"
}
```

**Handling:**
```wp
|> result
  authError(401):
    |> jq: `{
      error: "Authentication failed",
      message: .errors[0].message
    }`
  authRequired(401):
    |> jq: `{
      error: "Authentication required",
      loginUrl: "/login"
    }`
```

### SQL Errors

**Produced by:** `pg` middleware

**Error Structure:**
```json
{
  "type": "sqlError",
  "message": "relation \"nonexistent_table\" does not exist",
  "sqlstate": "42P01",
  "severity": "ERROR",
  "query": "SELECT * FROM nonexistent_table"
}
```

**Handling:**
```wp
|> result
  ok(200):
    |> jq: `{ success: true, data: .data }`
  sqlError(500):
    |> jq: `{
      error: "Database error",
      message: .errors[0].message,
      query: .errors[0].query
    }`
```

### HTTP/Network Errors

**Produced by:** `fetch` middleware

**Error types:**
- `networkError` - Connection failed
- `timeoutError` - Request timeout
- `httpError` - HTTP error status (4xx, 5xx)

**Handling:**
```wp
|> result
  ok(200):
    |> jq: `{ data: .data.response }`
  networkError(502):
    |> jq: `{ error: "External service unavailable" }`
  timeoutError(504):
    |> jq: `{ error: "Request timeout" }`
  httpError(500):
    |> jq: `{
      error: "External API error",
      status: .errors[0].status
    }`
```

### Custom Error Types

You can create custom error types in your pipelines:

```wp
GET /custom-errors
  |> lua: `
    if not request.user then
      return {
        errors = {{
          type = "customError",
          code = "USER_NOT_FOUND",
          message = "User does not exist"
        }}
      }
    end
    return request
  `
  |> result
    ok(200):
      |> jq: `{ success: true }`
    customError(404):
      |> jq: `{
        error: .errors[0].code,
        message: .errors[0].message
      }`
```

---

## Response Transformation

### JSON Responses (Default)

```wp
GET /json-response
  |> jq: `{
    message: "Hello World",
    data: { users: [1, 2, 3] },
    timestamp: now
  }`
```

### HTML Responses

```wp
GET /html-response
  |> jq: `{
    contentType: "text/html",
    name: "Alice",
    message: "Welcome!"
  }`
  |> handlebars: `
    <!DOCTYPE html>
    <html>
      <head><title>Welcome</title></head>
      <body>
        <h1>{{message}}</h1>
        <p>Hello, {{name}}!</p>
      </body>
    </html>
  `
```

### Plain Text Responses

```wp
GET /text-response
  |> jq: `{
    contentType: "text/plain",
    body: "This is plain text content\nLine 2\nLine 3"
  }`
```

### Binary/File Responses

```wp
GET /download/:id
  |> pg([.params.id]): `SELECT * FROM files WHERE id = $1`
  |> jq: `{
    contentType: .data.rows[0].mime_type,
    headers: {
      "Content-Disposition": "attachment; filename=\"" + .data.rows[0].filename + "\""
    },
    body: .data.rows[0].content
  }`
```

---

## Advanced Patterns

### Pattern: Conditional Response Format

```wp
GET /data
  |> pg: `SELECT * FROM items`
  |> jq: `
    if .query.format == "html" then
      { contentType: "text/html", items: .data.rows }
    else
      { contentType: "application/json", items: .data.rows }
    end
  `
  |> dispatch
    case @guard(`.contentType == "text/html"`):
      |> handlebars: `<ul>{{#each items}}<li>{{name}}</li>{{/each}}</ul>`
    default:
      |> jq: `{ data: .items }`
```

### Pattern: Error with Handlebars

```wp
GET /page
  |> pg: `SELECT * FROM nonexistent_table`
  |> result
    ok(200):
      |> jq: `{ contentType: "text/html", data: .data }`
      |> handlebars: `<p>Success: {{data}}</p>`
    sqlError(500):
      |> jq: `{
        contentType: "text/html",
        error: .errors[0].message
      }`
      |> handlebars: `
        <div class="error">
          <h1>Database Error</h1>
          <p>{{error}}</p>
        </div>
      `
```

### Pattern: Multi-Format API

```wp
GET /api/users
  |> pg: `SELECT * FROM users`
  |> jq: `{
    format: (.query.format // "json"),
    users: .data.rows
  }`
  |> dispatch
    case @guard(`.format == "xml"`):
      |> jq: `{
        contentType: "application/xml",
        body: "<users>" + ([.users[] | "<user><id>" + (.id | tostring) + "</id><name>" + .name + "</name></user>"] | join("")) + "</users>"
      }`
    case @guard(`.format == "csv"`):
      |> jq: `{
        contentType: "text/csv",
        headers: { "Content-Disposition": "attachment; filename=\"users.csv\"" },
        body: "id,name,email\n" + ([.users[] | (.id | tostring) + "," + .name + "," + .email] | join("\n"))
      }`
    default:
      |> jq: `{ users: .users }`
```

---

## See Also

- [Routes & Pipelines](./routes-and-pipelines.md) - Route and pipeline patterns
- [Flow Control](./flow-control.md) - Conditional execution
- [Handlebars Middleware](./middleware/handlebars.md) - HTML templating
- [Auth Middleware](./middleware/auth.md) - Authentication and cookie configuration
- [Error Handling](./error-handling.md) - Comprehensive error handling strategies
