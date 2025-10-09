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

## Cache Configuration

The cache system has built-in protection against denial-of-service attacks through memory exhaustion.

### Global Cache Config

```wp
config cache {
  enabled: true
  defaultTtl: 60                # Default TTL in seconds
  maxCacheSize: 10485760        # Total cache size limit (10MB default)
  maxEntrySize: 1048576         # Per-entry size limit (1MB default)
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

### Cache Statistics

Access cache statistics programmatically:

```rust
let stats = cache.stats();
// stats.entry_count: number of cached entries
// stats.total_size_bytes: current total size
// stats.max_entry_size: configured max per entry
// stats.max_total_size: configured max total size
```


