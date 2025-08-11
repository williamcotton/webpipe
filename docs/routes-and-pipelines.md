### Routes & Pipelines

- **Route**: HTTP method + path + pipeline.
- **Pipeline**: ordered middleware steps. Use `pipeline:` to reuse a named pipeline.
- **Named results**: set `resultName` before steps that return `.data` to capture multiple results.

Example:
```wp
pipeline loadUser =
  |> jq: `{ sqlParams: [.params.id], resultName: "user" }`
  |> pg: `SELECT * FROM users WHERE id = $1`

GET /users/:id
  |> pipeline: loadUser
  |> jq: `{ profile: .data.user }`
```


