### Routes & Pipelines

- **Route**: HTTP method + path + pipeline.
- **Pipeline**: ordered middleware steps. Use `pipeline:` to reuse a named pipeline.
- **Named results**: set `resultName` before steps that return `.data` to capture multiple results.
- **Concurrency**: Use `@async` to run steps in background and `join` middleware to sync. See [Concurrency](./concurrency.md).
- **Flow Control**: Use `@env` and `@flag` tags to conditionally execute steps. See [Flow Control](./flow-control.md).

Example:
```wp
pipeline loadUser =
  |> jq: `{ sqlParams: [.params.id], resultName: "user" }`
  |> pg: `SELECT * FROM users WHERE id = $1`

GET /users/:id
  |> pipeline: loadUser
  |> jq: `{ profile: .data.user }`
```


