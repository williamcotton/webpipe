### Result Routing

Route structured results to HTTP responses based on error types or conditions.

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

Common error types produced by middleware: `sqlError`, `networkError`, `timeoutError`, `httpError`, `authError`.


