# Context & Metadata

The `$context` object provides access to system-level metadata that persists across pipeline steps. Unlike the pipeline state (input data), context contains configuration, flags, and runtime conditions that affect how requests are processed.

## Overview

**Context** is a read-only snapshot of system metadata available in both JQ and Lua middleware:

- **JQ**: Access via `$context.flags`, `$context.conditions`, `$context.env`
- **Lua**: Access via `context.flags`, `context.conditions`, `context.env`

**Pipeline State** is the data flowing through your pipeline:

- **JQ**: Access via `.field` or top-level selectors
- **Lua**: Access via `request.field` or `request` table

---

## Context Structure

The context object contains three main fields:

### `$context.flags` (Feature Flags)

A map of feature flags set via `setFlag()` or the `featureFlags` pipeline.

**Type:** `Object<string, boolean>`

**Access in JQ:**
```wp
$context.flags["experimental-feature"]
$context.flags.newUI
```

**Access in Lua:**
```lua
context.flags["experimental-feature"]
context.flags.newUI
```

### `$context.conditions` (Dynamic Conditions)

A map of runtime conditions set via `setWhen()` in Lua middleware.

**Type:** `Object<string, boolean>`

**Access in JQ:**
```wp
$context.conditions["is_authenticated"]
$context.conditions.is_premium
```

**Access in Lua:**
```lua
context.conditions["is_authenticated"]
context.conditions.is_premium
```

### `$context.env` (Environment)

The current execution environment (e.g., "production", "development", "staging").

**Type:** `string`

**Access in JQ:**
```wp
$context.env == "production"
```

**Access in Lua:**
```lua
context.env == "production"
```

---

## Accessing Context in JQ

Context is available via the special `$context` variable.

### Reading Feature Flags

```wp
GET /feature-check
  |> jq: `{
    experimentalEnabled: $context.flags["experimental-feature"],
    newUIEnabled: $context.flags.newUI,
    allFlags: $context.flags
  }`
```

### Reading Conditions

```wp
GET /test-when-context-jq
  |> lua: `
    setWhen("from_mobile", true)
    setWhen("dark_mode", false)
    return request
  `
  |> jq: `{
    conditions: $context.conditions,
    from_mobile: $context.conditions.from_mobile,
    dark_mode: $context.conditions.dark_mode
  }`
```

### Checking Environment

```wp
GET /env-aware
  |> jq: `{
    env: $context.env,
    isProduction: ($context.env == "production"),
    isDevelopment: ($context.env == "development")
  }`
```

### Conditional Logic Based on Context

```wp
GET /dashboard
  |> jq: `{
    title: "Dashboard",
    showDebug: ($context.env != "production"),
    features: (if $context.flags.experimental then ["ai", "realtime"] else ["basic"] end),
    userAccess: (if $context.conditions.is_admin then "full" else "limited" end)
  }`
```

---

## Accessing Context in Lua

Context is available via the global `context` table.

### Reading Feature Flags

```lua
GET /lua-feature-check
  |> lua: `
    local hasExperimental = context.flags["experimental-feature"] or false
    local hasNewUI = context.flags.newUI or false

    return {
      experimental = hasExperimental,
      newUI = hasNewUI,
      allFlags = context.flags
    }
  `
```

### Reading Conditions

```wp
GET /test-when-context-lua
  |> lua: `setWhen("test_condition", true); return request`
  |> lua: `
    return {
      conditions = context.conditions,
      test_condition = context.conditions.test_condition or false,
      hasCondition = (context.conditions.test_condition ~= nil)
    }
  `
```

### Checking Environment

```lua
GET /lua-env-check
  |> lua: `
    local env = context.env
    local isProduction = (env == "production")
    local isDevelopment = (env == "development")

    return {
      environment = env,
      isProduction = isProduction,
      isDevelopment = isDevelopment
    }
  `
```

### Complex Context-Based Logic

```lua
GET /lua-context-routing
  |> lua: `
    local env = context.env
    local isAdmin = context.conditions.is_admin or false
    local hasExperimental = context.flags["experimental-feature"] or false

    -- Determine access level
    local access = "limited"
    if isAdmin then
      access = "full"
    elseif hasExperimental and env ~= "production" then
      access = "beta"
    end

    -- Determine features
    local features = {"basic"}
    if hasExperimental then
      table.insert(features, "ai")
      table.insert(features, "realtime")
    end

    return {
      access = access,
      features = features,
      environment = env,
      debug = (env ~= "production")
    }
  `
```

---

## Use Cases

### 1. Environment-Aware Logging

Enable verbose logging only in non-production environments:

```wp
GET /api/data
  |> jq: `{ sqlParams: [.params.id] }`
  |> pg: `SELECT * FROM data WHERE id = $1`
  |> jq: `
    if $context.env != "production" then
      . + { debug: { query: "SELECT * FROM data", env: $context.env } }
    else
      .
    end
  `
```

### 2. Feature Flag Checks

Conditionally load data based on feature flags:

```wp
GET /dashboard
  |> jq: `{ base: "data" }`
  |> jq: `
    if $context.flags.analytics then
      . + { analytics: "loaded" }
    else
      .
    end
  `
  |> jq: `
    if $context.flags.realtime then
      . + { realtime: "enabled" }
    else
      .
    end
  `
```

### 3. User Classification Routing

Route requests based on runtime conditions:

```wp
GET /content
  |> lua: `
    -- Classify user based on request data
    local isPremium = request.user and request.user.premium == true
    local isVerified = request.user and request.user.verified == true

    setWhen("is_premium", isPremium)
    setWhen("is_verified", isVerified)

    return request
  `
  |> jq: `
    if $context.conditions.is_premium and $context.conditions.is_verified then
      { tier: "premium", access: "full" }
    elif $context.conditions.is_verified then
      { tier: "standard", access: "limited" }
    else
      { tier: "free", access: "basic" }
    end
  `
```

### 4. Multi-Factor Configuration

Combine environment, flags, and conditions:

```wp
GET /advanced-route
  |> lua: `
    local user = request.user
    setWhen("is_authenticated", user ~= nil)
    setWhen("is_admin", user and user.role == "admin")
    return request
  `
  |> jq: `{
    title: "Advanced Features",
    features: (
      if $context.env == "production" and $context.conditions.is_admin then
        ["production-admin"]
      elif $context.flags.experimental and $context.conditions.is_authenticated then
        ["experimental-beta"]
      elif $context.env != "production" then
        ["dev-tools"]
      else
        ["standard"]
      end
    )
  }`
```

### 5. Debugging Context State

Inspect all context values for debugging:

```wp
GET /debug/context
  |> jq: `{
    environment: $context.env,
    flags: $context.flags,
    conditions: $context.conditions,
    flagCount: ($context.flags | keys | length),
    conditionCount: ($context.conditions | keys | length)
  }`
```

---

## Context vs Tags

Understanding when to use context access vs. tags is important for writing clean pipelines.

| Scenario | Use Tags | Use $context |
|----------|----------|--------------|
| Skip/include entire step | `@flag(name)` | No |
| Conditional data transformation | No | `if $context.flags.name then ... else ... end` |
| Dispatch routing | `case @flag(name):` | No |
| Complex multi-condition logic | No | `if $context.flags.a and $context.conditions.b then ...` |
| Logging context values | No | `"Env: " + $context.env` |

### Example: Tags vs Context

**Using Tags (Step-Level):**
```wp
GET /route
  |> jq: `{ data: "base" }` @flag(experimental)
  |> jq: `{ data: "standard" }` @!flag(experimental)
```

**Using Context (Data-Level):**
```wp
GET /route
  |> jq: `{
    data: (if $context.flags.experimental then "experimental" else "standard" end)
  }`
```

**When to use which:**
- Use **tags** when you want to include/exclude entire steps
- Use **$context** when you want to shape data based on flags/conditions

---

## Context vs Pipeline State

Understanding the difference is crucial:

| Aspect | Context ($context) | Pipeline State (.) |
|--------|-------------------|-------------------|
| Purpose | System metadata | Request data |
| Mutability | Read-only | Transformed by each step |
| Scope | Global across steps | Flows through pipeline |
| Contains | Flags, conditions, env | Request body, params, query, custom data |
| Access in JQ | `$context.flags` | `.field` |
| Access in Lua | `context.flags` | `request.field` |

### Example: Using Both

```wp
GET /user/:id/profile
  |> jq: `{ userId: (.params.id | tonumber) }`
  |> pg([.userId]): `SELECT * FROM users WHERE id = $1` @result(user)
  |> jq: `{
    user: .data.user.rows[0],
    environment: $context.env,
    showDebug: ($context.env != "production"),
    features: (
      if $context.conditions.is_premium then
        ["profile", "analytics", "reports"]
      else
        ["profile"]
      end
    )
  }`
```

**Pipeline State:** `.userId`, `.data.user`, `.user`
**Context:** `$context.env`, `$context.conditions.is_premium`

---

## Context Lifetime

### Feature Flags (`$context.flags`)

- **Set by:** `setFlag()` in Lua or `featureFlags` pipeline
- **Lifetime:** Persists across all steps in the request
- **Scope:** Request-scoped (reset per request unless explicitly set)

### Dynamic Conditions (`$context.conditions`)

- **Set by:** `setWhen()` in Lua
- **Lifetime:** Transient, per-request only
- **Scope:** Request-scoped (reset per request)

### Environment (`$context.env`)

- **Set by:** Server configuration (`WEBPIPE_ENV` or similar)
- **Lifetime:** Process-scoped (same for all requests)
- **Scope:** Global

---

## Context Functions in Lua

In addition to reading context, Lua has functions to modify it:

### `setFlag(name, value)`

Set a feature flag for the current request:

```lua
setFlag("experimental-feature", true)
setFlag("new-ui", user.betaTester)
```

### `getFlag(name)`

Read a feature flag value:

```lua
local hasExperimental = getFlag("experimental-feature")  -- returns boolean
```

### `setWhen(name, value)`

Set a dynamic condition for the current request:

```lua
setWhen("is_authenticated", user ~= nil)
setWhen("is_premium", user and user.premium == true)
```

### `getWhen(name)`

Read a dynamic condition value:

```lua
local isPremium = getWhen("is_premium")  -- returns boolean or false if not set
```

**Note:** Context snapshots are taken at the start of each Lua script execution, so conditions set in one Lua step won't be visible in `context.conditions` within the same step. Use `getWhen()` to read back values set in the same script, or access them in subsequent steps.

---

## Best Practices

### 1. Use Context for Read-Only Checks

Context is read-only in JQ and should primarily be used for checking values, not storing computed data.

**Good:**
```wp
|> jq: `{ isProd: ($context.env == "production") }`
```

**Avoid:**
```wp
# Context cannot be modified in JQ
|> jq: `$context.flags.new = true`  # This won't work
```

### 2. Set Flags/Conditions Early

Set flags and conditions at the beginning of your pipeline:

```wp
GET /route
  |> lua: `
    setWhen("is_admin", request.user and request.user.role == "admin")
    setFlag("debug-mode", request.query.debug == "true")
    return request
  `
  # Now $context.conditions and $context.flags are available in all subsequent steps
  |> jq: `...`
```

### 3. Document Context Dependencies

Make it clear when routes depend on specific flags or conditions:

```wp
# Requires: @flag(analytics), @when(is_authenticated)
GET /dashboard
  |> jq: `{
    analytics: (if $context.flags.analytics and $context.conditions.is_authenticated then "enabled" else "disabled" end)
  }`
```

### 4. Use Context for Cross-Cutting Concerns

Context is ideal for:
- Environment detection
- Feature flags
- User classification
- Access control metadata

Avoid using context for:
- Passing data between steps (use pipeline state)
- Complex business logic (use explicit data transformations)

### 5. Default Missing Values

Always provide defaults when accessing context to avoid null errors:

**JQ:**
```wp
|> jq: `{
  experimental: ($context.flags.experimental // false),
  isPremium: ($context.conditions.is_premium // false)
}`
```

**Lua:**
```lua
local hasExperimental = context.flags["experimental"] or false
local isPremium = context.conditions.is_premium or false
```

---

## Common Patterns

### Pattern: Environment-Based Configuration

```wp
GET /api/config
  |> jq: `{
    apiUrl: (if $context.env == "production" then "https://api.prod.com" else "https://api.dev.com" end),
    debugMode: ($context.env != "production"),
    cacheEnabled: ($context.env == "production")
  }`
```

### Pattern: Progressive Feature Rollout

```wp
GET /features
  |> lua: `
    -- Enable experimental features for 10% of users
    math.randomseed(os.time())
    local enableExperimental = (math.random() < 0.1)
    setFlag("experimental-rollout", enableExperimental)
    return request
  `
  |> jq: `{
    features: (
      if $context.flags["experimental-rollout"] then
        ["standard", "experimental"]
      else
        ["standard"]
      end
    )
  }`
```

### Pattern: Access Control Decision

```wp
GET /admin/panel
  |> lua: `
    local user = request.user
    setWhen("is_admin", user and user.role == "admin")
    setWhen("is_authenticated", user ~= nil)
    return request
  `
  |> dispatch
    case @when(is_admin):
      |> jq: `{ access: "granted", panel: "admin" }`
    case @when(is_authenticated):
      |> jq: `{ access: "denied", reason: "insufficient permissions" }`
    default:
      |> jq: `{ access: "denied", reason: "not authenticated" }`
```

---

## See Also

- [Flow Control](./flow-control.md) - Using `@flag`, `@when`, `@env`, and `@guard` tags
- [Lua Middleware](./middleware/lua.md) - `setFlag()`, `setWhen()`, `getFlag()`, `getWhen()` functions
- [DSL Syntax](./dsl-syntax.md) - Complete syntax reference
