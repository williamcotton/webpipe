### validate Middleware

Validates fields in the current JSON.

Example:
```wp
|> validate: `{
  login: string(3..50),
  email: email,
  password: string(8..100)
}`
```

On failure, produces `errors: [ { type: "validationError", field, message, rule? } ]`.


