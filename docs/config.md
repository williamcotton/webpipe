### Configuration

- **Global configs**: `pg`, `auth`, `cache`, `log`.
- Values can reference env vars: `$NAME || "default"`.

Examples:

```wp
config auth {
  sessionTtl: 604800
  cookieName: "wp_session"
  cookieSecure: false
  cookieHttpOnly: true
  cookieSameSite: "Lax"
  cookiePath: "/"
}

config log {
  enabled: true
  format: "json"
  level: "debug"
  includeHeaders: true
}
```


