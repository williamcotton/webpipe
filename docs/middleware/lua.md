### lua Middleware

Run Lua scripts with access to `request` JSON and helpers.

Globals:
- `request`: current JSON
- `executeSql(sql, params?) -> (result, err)`
- `getEnv(name) -> string|nil`
- `requireScript(name)`: loads `scripts/<name>.lua` and returns its value

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


