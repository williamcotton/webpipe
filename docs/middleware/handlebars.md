### handlebars Middleware

Render strings using Handlebars templates.

Usage:
```wp
|> jq: `{ name: "World" }`
|> handlebars: `<p>Hello, {{name}}</p>`
```

- Supports inline partials via `{{#*inline "name"}}...{{/inline}}` as seen in `comprehensive_test.wp`.
- Errors surface as `MiddlewareExecutionError` with a message in server logs/tests.


