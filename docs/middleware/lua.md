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

