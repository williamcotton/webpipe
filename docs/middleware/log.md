### log Middleware

Adds logging metadata and prints a JSON log line in `post_execute`.

Config keys:
- `level`
- `includeBody`
- `includeHeaders`
- `enabled`

Example:
```wp
|> log: `level: debug, includeBody: false, includeHeaders: true`
```


