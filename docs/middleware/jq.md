### jq Middleware

Transform JSON using jq expressions.

Usage:
```wp
|> jq: `{ sqlParams: [.params.id] }`
|> jq: `.data.rows | length`
```

Notes:
- Input is serialized to JSON and fed to jq.
- Result must be valid JSON (string, number, object, array, true/false/null).


