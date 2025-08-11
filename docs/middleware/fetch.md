### fetch Middleware

Perform HTTP requests.

Inputs:
- `fetchUrl` (overrides inline URL)
- `fetchMethod` (default GET)
- `fetchHeaders` (object)
- `fetchBody` (JSON for POST/PUT)
- `fetchTimeout` (seconds)
- `resultName` (to place under `.data.<name>`)

Caching:
- Respects `_metadata.cache` set by `cache` middleware.
- Cache key from `keyTemplate` or an automatic hash of method/url/headers/body.

Errors: `networkError`, `timeoutError`, `httpError`.

Example:
```wp
|> jq: `{ fetchUrl: "https://api.github.com/zen", resultName: "api" }`
|> fetch: `_`
|> jq: `{ zen: .data.api.response }`
```


