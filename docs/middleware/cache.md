### cache Middleware

Caches the **final output of the pipeline the step appears in**: the body, the
content type, and the status code. On a hit, the rest of that pipeline is
skipped and the cached response is replayed.

Because the cached unit is "everything after this step until the end of this
pipeline", the same step works at two levels:

- **Data caching** — inside a named pipeline, caching the pipeline's processed
  result (e.g. an upstream API response after transformation):

  ```wp
  pipeline contentfulArticles =
    |> contentfulAuth
    |> cache: `ttl: 60, keyTemplate: data-articles`
    |> contentfulFetch
    |> lua: `...processing...`
  ```

- **Response caching** — as the first step of a route handler, caching the
  fully rendered response (HTML, content type, and status code):

  ```wp
  GET /articles/:article
    |> cache: `ttl: 120, keyTemplate: route-article-{params.article}`
    |> contentfulByArticle
    |> result
      ok(200):
        |> handlebars: articleLayout
  ```

Both levels compose: a route-level hit skips everything; a route-level miss
can still hit the inner data cache and only pay for rendering.

Config keys:
- `enabled: true|false`
- `ttl: <seconds>` (`ttl: 0` disables caching for the step)
- `keyTemplate: <template-with-{path} placeholders>`

### Cache keys

- **`keyTemplate` keys are a global namespace.** Two steps using the same
  rendered template string share one entry — that is the mechanism for
  intentional sharing (e.g. `/` and `/htmx/` both reading `data-articles`).
  Use prefixes like `route-` and `data-` to avoid accidental collisions, and
  include the distinguishing params (`route-article-{params.article}`).
- **Default keys (no `keyTemplate`) are namespaced per step.** The key hashes
  the step's source location together with the config and the request's
  `method`, `path`, `query`, and `params`, so two distinct `cache` steps can
  never collide even with byte-identical configs.

### What is never cached

- Typed error envelopes (`{ errors: [...] }`) — errors before a `result`
  block shapes them are never pinned for a TTL.
- Responses with a status code **>= 500** — a rendered 500 page from a
  transient upstream outage is not cached. Rendered 4xx pages (e.g. a 404
  page from a `notFound(404)` result branch) **are** cached, and replay with
  their stored status code.

### Stale-while-revalidate

When an entry's TTL expires it enters a grace window (`staleWhileRevalidate`
seconds, default 30) instead of vanishing:

- The **first** request in the window is elected the refresher: it proceeds
  through the pipeline and overwrites the entry with the fresh result.
- All other requests are served the stale value immediately.
- If the refresher fails (error envelope or 5xx), the refresh slot is
  released so the next request can try; a crashed refresher is timed out
  after 30 seconds.

This removes the thundering-herd at TTL expiry: under load, each expiry
costs exactly one upstream call and no request ever waits on it. Set
`staleWhileRevalidate: 0` to disable (entries then expire outright).

Note the total staleness bound is `ttl + staleWhileRevalidate`.

## Cache Configuration

### Global Cache Config

```wp
config cache {
  enabled: true
  defaultTtl: 60                # Default TTL in seconds
  maxCacheSize: 10485760        # Total cache size limit (10MB default)
  maxEntrySize: 1048576         # Per-entry size limit (1MB default)
  staleWhileRevalidate: 30      # Grace window after TTL expiry (seconds)
}
```

### Security Features

#### Per-Entry Size Limit
- **Default**: 1MB (1,048,576 bytes)
- **Purpose**: Prevents a single large entry from consuming all memory
- **Behavior**: Entries exceeding this limit are rejected with a warning

#### Total Cache Size Limit
- **Default**: 10MB (10,485,760 bytes)
- **Purpose**: Bounds total memory usage
- **Behavior**: When limit is reached, LRU (Least Recently Used) entries are evicted

#### Automatic Size Tracking
- Each entry's size is calculated on insertion
- Total cache size is tracked in real-time
- Expired entries are automatically removed and their size reclaimed

### DoS Attack Prevention

The cache system prevents unbounded memory growth from attacks like:

```bash
# Attacker attempt: Fill cache with large entries
for i in {1..1000}; do
  curl -X POST https://app.com/api/endpoint \
    -d '{"data": "'$(python -c 'print("A"*1000000)')'"}'
done
```

**Protection Mechanisms:**
1. Large individual entries (>1MB) are rejected
2. Total cache size is bounded (default 10MB)
3. Old entries are automatically evicted when space is needed
4. All size checks happen before memory allocation

Note that caching rendered pages keyed by request params (e.g.
`route-article-{params.article}`) lets arbitrary missing slugs each create a
404-page entry; this is bounded by the per-entry and total size limits plus
LRU eviction, but prefer tighter TTLs for parameterized 404s if it matters.

### Known limitations

- Don't place `cache` inside an `@async`-tagged branch: pending saves
  registered in spawned tasks are dropped when the task's pipeline context
  ends.
- Inside `foreach`, default keys do not distinguish iteration items; use a
  `keyTemplate` that includes item fields.

### Cache Statistics

Access cache statistics programmatically:

```rust
let stats = cache.stats();
// stats.entry_count: number of cached entries
// stats.total_size_bytes: current total size
// stats.max_entry_size: configured max per entry
// stats.max_total_size: configured max total size
```
