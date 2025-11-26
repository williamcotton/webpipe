# Rate Limit Middleware

The `rateLimit` middleware provides Token Bucket based rate limiting to protect your routes from abuse.

## Configuration

Config keys are comma-separated.

*   **`keyTemplate`**: (Required) String template to define the unique bucket key. Supports variable interpolation like `{ip}`, `{method}`, `{path}`, `{params.id}`.
*   **`limit`**: (Required) Number of requests allowed in the window.
*   **`window`**: (Required) Time window (e.g., "60s", "1m", "1h").
*   **`burst`**: (Optional) Additional burst capacity allowed above the rate.
*   **`enabled`**: (Optional) Boolean to conditionally enable/disable.

## Examples

### Basic IP Limiting

Limit each IP to 60 requests per minute.

```wp
GET /api/public
  |> rateLimit: `keyTemplate: ip-{ip}, limit: 60, window: 1m`
  |> pg: ...
```

### Per-User Limiting

Limit based on an authenticated User ID.

```wp
GET /api/private
  |> auth: "required"
  |> rateLimit: `keyTemplate: user-{user.id}, limit: 1000, window: 1h`
  |> ...
```

### Burst Capacity

Allow short bursts of traffic but maintain a long-term average.

```wp
GET /api/search
  |> rateLimit: `keyTemplate: search-{ip}, limit: 10, window: 1m, burst: 5`
  |> ...
```

