### Error Handling

- Middleware returns structured errors under `errors: [ { type, ... } ]`.
- Map them with `result` (see `result-routing.md`).
- Examples emitted:
  - **pg**: `sqlError { message, sqlstate?, severity?, query }`
  - **fetch**: `networkError | timeoutError | httpError { status?, message, url }`
  - **auth**: `authError { message }`
  - **validate**: `validationError { field, message, rule? }`


