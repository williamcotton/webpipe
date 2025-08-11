### lua Middleware

Run Lua scripts with access to `request` JSON and helpers.

Globals:
- `request`: current JSON
- `executeSql(sql) -> (result, err)`
- `getEnv(name) -> string|nil`
- `requireScript(name)`: loads `scripts/<name>.lua` and returns its value

Example:
```wp
|> lua: `
  local id = request.params.id
  local result, err = executeSql("SELECT * FROM teams WHERE id = " .. id)
  if err then return { errors = { { type = "sqlError", message = err } } } end
  return result
`
```

Security: dangerous Lua stdlib functions are removed.


