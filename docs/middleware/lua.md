### lua Middleware

Run Lua scripts with access to `request` JSON and helpers.

Globals:
- `request`: current JSON
- `executeSql(sql, params?) -> (result, err)`
- `getEnv(name) -> string|nil`
- `requireScript(name)`: loads `scripts/<name>.lua` and returns its value
- `getFlag(name) -> boolean`: read a feature flag (false if not set)
- `setFlag(name, value)`: set a feature flag for downstream steps

#### executeSql(sql, params?)

Executes a SQL query with optional parameterized values for security.

**Parameters:**
- `sql` (string): SQL query with `$1`, `$2`, etc. placeholders
- `params` (table, optional): Array of parameter values to bind

**Returns:** `(result, err)` where result contains `{rows: [...], rowCount: n}` on success

**Examples:**

Parameterized query (recommended):
```wp
|> lua: `
  local id = tonumber(request.params.id)
  local result, err = executeSql("SELECT * FROM teams WHERE id = $1", {id})
  if err then return { errors = { { type = "sqlError", message = err } } } end
  return result
`
```

Multiple parameters:
```wp
|> lua: `
  local userId = request.user.id
  local status = request.query.status or "active"
  local result, err = executeSql(
    "SELECT * FROM tasks WHERE user_id = $1 AND status = $2",
    {userId, status}
  )
  if err then return { errors = { { type = "sqlError", message = err } } } end
  return result
`
```

Simple query without parameters:
```wp
|> lua: `
  local result, err = executeSql("SELECT * FROM teams LIMIT 10")
  if err then return { errors = { { type = "sqlError", message = err } } } end
  return result
`
```

**Security:** Always use parameterized queries when incorporating user input to prevent SQL injection attacks. Dangerous Lua stdlib functions are removed.

#### Feature Flags

Use `getFlag` and `setFlag` to read and write feature flags from Lua scripts.

**Setting flags** (typically in a `featureFlags` pipeline):
```wp
pipeline featureFlags =
  |> lua: ```
    local isBeta = request.headers and request.headers["x-beta-tester"] == "true"
    setFlag("use-stripe", isBeta)
    setFlag("new-ui", true)
    return request
  ```
```

**Reading flags** (in route handlers):
```wp
GET /checkout
  |> lua: ```
    if getFlag("use-stripe") then
      return { processor = "stripe", amount = request.amount }
    else
      return { processor = "paypal", amount = request.amount }
    end
  ```
```

Note: `getFlag` returns a snapshot of flags at script start. Flags set via `setFlag` within the same script won't be visible to `getFlag` in that same execution, but will be available to subsequent pipeline steps.

---

## Dynamic Conditions (@when)

Use `setWhen()` and `getWhen()` to set transient runtime conditions for routing with `@when` tags.

### Available Functions

- `setWhen(name, value)` - Set a dynamic condition for the current request
- `getWhen(name) -> boolean` - Read a dynamic condition value

**Setting conditions** (typically based on request data):
```wp
GET /admin/panel
  |> lua: `
    local user = request.user
    setWhen("is_admin", user and user.role == "admin")
    setWhen("is_authenticated", user ~= nil)
    setWhen("is_premium", user and user.premium == true)
    return request
  `
  |> dispatch
    case @when(is_admin):
      |> jq: `{ access: "granted", panel: "admin" }`
    case @when(is_authenticated):
      |> jq: `{ access: "denied", reason: "insufficient permissions" }`
    default:
      |> jq: `{ access: "denied", reason: "not authenticated" }`
```

**Reading conditions** (in Lua scripts):
```wp
|> lua: `
  local isAdmin = getWhen("is_admin")
  if isAdmin then
    return { userType: "admin", fullAccess: true }
  else
    return { userType: "guest", fullAccess: false }
  end
`
```

**Difference from @flag:**
- `@flag` - Persistent feature flags (set once, available throughout request)
- `@when` - Transient conditions (request-scoped, for runtime classification)

---

## Accessing Context

Lua scripts can access the `context` object to read flags, conditions, and environment:

```wp
|> lua: `
  -- Access feature flags
  local hasExperimental = context.flags["experimental-feature"] or false

  -- Access dynamic conditions
  local isPremium = context.conditions.is_premium or false

  -- Access environment
  local env = context.env

  return {
    experimental: hasExperimental,
    premium: isPremium,
    environment: env,
    isProduction: (env == "production")
  }
`
```

**Context Structure:**
- `context.flags` - Map of feature flags (set via `setFlag()`)
- `context.conditions` - Map of dynamic conditions (set via `setWhen()`)
- `context.env` - Current environment ("production", "development", etc.)

See [Context & Metadata](../context.md) for comprehensive documentation on the context system.

