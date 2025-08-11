### pg Middleware

Execute SQL against Postgres using sqlx.

Inputs:
- `sqlParams`: array of parameters bound in order
- optional `resultName`: capture under `.data.<name>`

Behavior:
- SELECT/RETURNING → `.data = { rows: [...], rowCount: N }`
- Non-SELECT → `.data = { rows: [], rowCount: <affected> }`
- On error → `errors: [ { type: "sqlError", message, sqlstate?, severity?, query } ]`

Examples:
```wp
|> jq: `{ sqlParams: [.params.id], resultName: "team" }`
|> pg: `SELECT * FROM teams WHERE id = $1`
```


