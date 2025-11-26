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

config rateLimit {
  # Default policies can be defined here
  limit: 100
  window: "60s"
  burst: 10
  enabled: true
}
```


