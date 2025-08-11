### cache Middleware

Sets cache metadata used by other middleware (e.g., `fetch`).

Config keys:
- `enabled: true|false`
- `ttl: <seconds>`
- `keyTemplate: <template-with-{path} placeholders>`

Example:
```wp
|> cache: `
  ttl: 30
  enabled: true
  keyTemplate: user-{params.id}
`
```


