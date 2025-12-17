# Flow Control

Tag-based flow control mechanisms allowing you to conditionally execute steps based on environments or feature flags.

## Tags

Control logic is implemented via tags appended to pipeline steps.

### Environment Control (`@env`)

Use `@env` to restrict steps to specific environments (defined by the `WEBPIPE_ENV` or equivalent configuration).

*   **`@env(production)`**: Execute only if environment is `production`.
*   **`@!env(dev)`**: Execute only if environment is **NOT** `dev`.

```wp
GET /api/data
  |> log: "Debug trace..." @env(dev)
  |> pipeline: secureHeaders @env(production)
  |> pg: `SELECT * FROM data`
```

### Feature Flags (`@flag`)

Use `@flag` to toggle steps based on dynamic feature flags.

*   **`@flag(new-feature)`**: Execute only if `new-feature` flag is enabled.
*   **`@!flag(legacy-mode)`**: Execute only if `legacy-mode` flag is disabled.
*   **`@flag(beta,staff)`**: Execute only if ALL specified flags are enabled (multiple flags).

```wp
GET /checkout
  |> pipeline: stripeProcessor @flag(use-stripe)
  |> pipeline: paypalProcessor @!flag(use-stripe)
  |> pipeline: betaFeatures @flag(beta,staff)
```

## Feature Flag Pipeline

You can define a special `featureFlags` pipeline in your `.wp` files to dynamically calculate flag states for every request. This pipeline runs before the main route handler.

The pipeline should return a JSON object with a `flags` map at the root level.

```wp
pipeline featureFlags =
  |> jq: `{ 
      flags: {
        "use-stripe": (.headers["x-beta-tester"] == "true"),
        "new-ui": true 
      }
    }`

# Flags are now available for @flag checks
```

Alternatively, you can use Lua to set flags dynamically via `setFlag()`:

```wp
pipeline featureFlags =
  |> lua: ```
    local isBeta = input.headers and input.headers["x-beta-tester"] == "true"
    setFlag("use-stripe", isBeta)
    setFlag("new-ui", true)
    return input
  ```
```

## If/Else Blocks

Use `if/else` blocks for conditional branching based on runtime data evaluation. Unlike tags which use static configuration, if/else blocks evaluate conditions dynamically during request execution.

### Basic Syntax

```wp
|> if
  |> <condition-pipeline>
  then:
    |> <then-branch-steps>
  else:
    |> <else-branch-steps>
  end
```

The `else:` and `end` keywords are optional.

### How It Works

1. **Condition Evaluation**: The condition pipeline runs on a **cloned copy** of the current state
2. **Truthiness Check**: The result is evaluated for truthiness:
   - `false` and `null` are **falsy**
   - Everything else is **truthy** (numbers, strings, objects, arrays, etc.)
3. **Branch Execution**: The appropriate branch executes on the **original state** (not the condition output)

### Simple Example

```wp
GET /api/users/:id
  |> pg: `SELECT * FROM users WHERE id = $1`
  |> if
    |> jq: `.data.rows | length > 0`
    then:
      |> jq: `.data.rows[0]`
    else:
      |> jq: `{ error: "User not found" }`
```

### Optional Else

If you omit the `else:` branch, the state passes through unchanged when the condition is false:

```wp
GET /api/data
  |> jq: `{ value: 10 }`
  |> if
    |> jq: `.value > 5`
    then:
      |> jq: `. + { bonus: "high value" }`
  # If condition is false, the original state continues
```

### Nested If Blocks

You can nest if blocks for complex logic:

```wp
GET /api/classify/:score
  |> jq: `{ score: (.params.score | tonumber) }`
  |> if
    |> jq: `.score > 0`
    then:
      |> if
        |> jq: `.score > 80`
        then:
          |> jq: `. + { grade: "A", tier: "premium" }`
        else:
          |> jq: `. + { grade: "B", tier: "standard" }`
    else:
      |> jq: `. + { grade: "F", tier: "free" }`
```

### Multi-Step Conditions

Conditions can include multiple pipeline steps:

```wp
GET /api/protected
  |> if
    |> jq: `{ sqlParams: [.headers.userId] }`
    |> pg: `SELECT active FROM users WHERE id = $1`
    |> jq: `.data.rows[0].active`
    then:
      |> jq: `{ message: "Access granted" }`
    else:
      |> jq: `{ error: "Access denied", status: 403 }`
```

### Using the `end` Keyword

When you have steps after an if/else block, use the `end` keyword to explicitly mark where the block ends:

```wp
GET /api/process/:value
  |> jq: `{ value: (.params.value | tonumber) }`
  |> if
    |> jq: `.value > 50`
    then:
      |> jq: `{ category: "high" }`
    else:
      |> jq: `{ category: "low" }`
    end
  |> jq: `{ success: true, data: . }`
```

Without `end`, the parser might include subsequent steps as part of the else branch.

### Truthiness Examples

```wp
# These are FALSY (else branch executes):
|> if
  |> jq: `false`  # boolean false

|> if
  |> jq: `null`   # null value

# These are TRUTHY (then branch executes):
|> if
  |> jq: `true`   # boolean true

|> if
  |> jq: `0`      # number (even zero!)

|> if
  |> jq: `""`     # string (even empty!)

|> if
  |> jq: `[]`     # array (even empty!)

|> if
  |> jq: `{}`     # object (even empty!)
```

### Comparison: Tags vs If/Else

| Feature | Tags (`@flag`, `@env`) | If/Else Blocks |
|---------|------------------------|----------------|
| Evaluation | Static (at route load) | Dynamic (at runtime) |
| Based on | Environment/flags | Request data |
| Granularity | Per-step | Per-pipeline segment |
| Use case | Environment toggling | Data-driven logic |

---

## Dispatch Blocks

Dispatch blocks provide switch-like conditional branching based on tags. Unlike if/else blocks which evaluate runtime data, dispatch evaluates tags (`@flag`, `@env`, `@when`, `@guard`) to route execution through different branches.

### Basic Syntax

```wp
|> dispatch
  case @flag(feature-name):
    |> <steps-when-flag-enabled>
  case @env(production):
    |> <steps-for-production>
  default:
    |> <fallback-steps>
```

The `default:` branch is optional but recommended for handling cases where no conditions match.

### Feature Flag Dispatch

Route based on feature flags:

```wp
GET /feature-test-dispatch
  |> dispatch
    case @flag(experimental-feature):
      |> jq: `{ message: "Experimental feature enabled!" }`
    default:
      |> jq: `{ message: "Standard feature (no flag required)" }`
```

### Environment-Based Dispatch

Different behavior per environment:

```wp
GET /env-dispatch
  |> dispatch
    case @env(production):
      |> jq: `{ env: "production", debug: false, caching: true }`
    case @env(staging):
      |> jq: `{ env: "staging", debug: true, caching: true }`
    case @env(development):
      |> jq: `{ env: "development", debug: true, caching: false }`
    default:
      |> jq: `{ env: "unknown", debug: false, caching: false }`
```

### Negated Tags in Dispatch

Use `@!` to check for absence:

```wp
GET /non-production-dispatch
  |> dispatch
    case @!env(production):
      |> jq: `{
        message: "Non-production environment",
        debugTools: true,
        mockData: true
      }`
    default:
      |> jq: `{
        message: "Production environment",
        debugTools: false,
        mockData: false
      }`
```

### Multi-Step Pipelines in Branches

Each case can contain multiple pipeline steps:

```wp
GET /multi-step-dispatch
  |> dispatch
    case @flag(experimental):
      |> jq: `{ step: 1, version: "experimental" }`
      |> jq: `. + { step: 2, features: ["ai", "realtime"] }`
      |> jq: `. + { step: 3, complete: true }`
    case @flag(beta):
      |> jq: `{ step: 1, version: "beta" }`
      |> jq: `. + { step: 2, features: ["analytics"] }`
    default:
      |> jq: `{ step: 1, version: "stable", features: [] }`
```

### Boolean Expressions in Dispatch

Combine multiple tags with `and` / `or`:

```wp
GET /test-dispatch-and
  |> dispatch
    case @when(authenticated) and @when(admin):
      |> jq: `{ access: "admin_panel" }`
    case @when(authenticated):
      |> jq: `{ access: "user_dashboard" }`
    default:
      |> jq: `{ access: "login_required" }`
```

```wp
GET /test-dispatch-or
  |> dispatch
    case @when(mobile) or @when(tablet):
      |> jq: `{ layout: "compact" }`
    default:
      |> jq: `{ layout: "desktop" }`
```

Mix different tag types:

```wp
GET /test-dispatch-mixed
  |> dispatch
    case @when(premium_user) and @!flag(maintenance):
      |> jq: `{ access: "premium" }`
    default:
      |> jq: `{ access: "basic" }`
```

Explicit grouping with parentheses:

```wp
GET /test-dispatch-grouping
  |> dispatch
    case (@when(admin) or @when(staff)) and @when(on_duty):
      |> jq: `{ status: "authorized" }`
    default:
      |> jq: `{ status: "unauthorized" }`
```

**Precedence Rules:**
- `and` binds tighter than `or`
- `@when(a) or @when(b) and @when(c)` is evaluated as `@when(a) or (@when(b) and @when(c))`
- Use parentheses for explicit grouping: `(@when(a) or @when(b)) and @when(c)`

---

## Foreach Loops

Foreach loops transform each element in an array in-place. The syntax allows you to apply a pipeline of transformations to every item in a specified array.

### Basic Syntax

```wp
|> foreach <selector>
  |> <transformation-steps>
end
```

The `<selector>` is a JQ-style path to the array (e.g., `data.rows`, `items`, `users`).

### Simple Example

Transform each team name to uppercase:

```wp
GET /test-foreach-basic
  |> jq: `{
    data: {
      rows: [
        { id: 1, name: "Alpha Team" },
        { id: 2, name: "Beta Team" },
        { id: 3, name: "Gamma Team" }
      ]
    }
  }`
  |> foreach data.rows
    |> jq: `. + { processed: true, uppercase_name: (.name | ascii_upcase) }`
  end
  |> jq: `{ teams: .data.rows }`
```

**Result:**
```json
{
  "teams": [
    { "id": 1, "name": "Alpha Team", "processed": true, "uppercase_name": "ALPHA TEAM" },
    { "id": 2, "name": "Beta Team", "processed": true, "uppercase_name": "BETA TEAM" },
    { "id": 3, "name": "Gamma Team", "processed": true, "uppercase_name": "GAMMA TEAM" }
  ]
}
```

### How Foreach Works

1. The selector identifies the array to iterate over
2. Each element is processed through the inner pipeline steps
3. The transformed element replaces the original in the array
4. The modified array remains at the same path in the request state

### Use Cases

**Batch Data Enrichment:**
```wp
|> foreach users
  |> jq: `. + { fullName: .firstName + " " + .lastName }`
  |> jq: `. + { age: ((now - .birthdate) / 31536000) | floor }`
end
```

**Data Normalization:**
```wp
|> foreach products
  |> jq: `. + { price: (.price | tonumber), inStock: (.quantity > 0) }`
end
```

**Multi-Step Transformations:**
```wp
|> foreach items
  |> jq: `. + { priceWithTax: .price * 1.2 }`
  |> jq: `. + { discount: (if .priceWithTax > 100 then 0.1 else 0) }`
  |> jq: `. + { finalPrice: .priceWithTax * (1 - .discount) }`
end
```

---

## @guard Tag

The `@guard` tag enables conditional step execution based on JQ expressions evaluated against the current request state. Steps with guards are only executed if their expression evaluates to truthy.

### Basic Syntax

*   **`@guard(\`jq_expression\`)`**: Execute step only if expression is truthy
*   **`@!guard(\`jq_expression\`)`**: Execute step only if expression is falsy

```wp
|> jq: `. + { message: "Admin only" }` @guard(`.user.role == "admin"`)
|> jq: `. + { debugInfo: "..." }` @!guard(`.env == "production"`)
```

### Simple Guards

Execute steps based on user roles:

```wp
GET /admin/dashboard
  |> jq: `{ user: { role: "admin", name: "Alice" } }`
  |> jq: `. + { message: "Admin dashboard - restricted access" }` @guard(`.user.role == "admin"`)
  |> jq: `. + { message: "Access denied" }` @guard(`.user.role != "admin"`)
```

### Negated Guards

Skip steps in certain environments:

```wp
GET /debug/info
  |> jq: `{ env: "development", debug: true }`
  |> log: `level: debug` @!guard(`.env == "production"`)
  |> jq: `{ debugInfo: "Verbose logging enabled" }` @guard(`.debug == true`)
```

### Complex Conditions

Combine multiple checks with `and` / `or`:

```wp
GET /premium/features
  |> jq: `{
    user: {
      premium: true,
      verified: true,
      credits: 100
    }
  }`
  |> jq: `. + { premiumData: "loaded" }` @guard(`.user.premium and .user.verified`)
  |> jq: `. + { bonus: "10 credits" }` @guard(`.user.credits > 50`)
```

### Array and Nested Property Guards

Check array lengths:

```wp
GET /process/items
  |> jq: `{ items: [1, 2, 3, 4, 5] }`
  |> jq: `{ processed: .items | map(. * 2) }` @guard(`(.items | length) >= 3`)
```

Check nested properties:

```wp
GET /api/user/preferences
  |> jq: `{
    user: {
      settings: {
        notifications: {
          email: true,
          push: false
        }
      }
    }
  }`
  |> jq: `. + { sendEmail: true }` @guard(`.user.settings.notifications.email == true`)
  |> jq: `. + { sendPush: true }` @guard(`.user.settings.notifications.push == true`)
```

### Null Checks

Handle optional fields:

```wp
GET /api/optional/feature
  |> jq: `{ feature: null, fallback: "default" }`
  |> jq: `{ source: "feature" }` @guard(`.feature != null`)
  |> jq: `{ source: "fallback" }` @guard(`.feature == null`)
```

### Multiple Guards in Pipeline

Chain conditional steps:

```wp
pipeline conditionalProcessing =
  |> jq: `{ stage: 1 }` @guard(`.enabled != false`)
  |> jq: `. + { stage: 2 }` @guard(`.stage == 1`)
  |> jq: `. + { stage: 3 }` @guard(`.stage == 2`)
  |> jq: `. + { complete: true }`
```

### Guards with Dispatch

Use guards as dispatch case conditions:

```wp
GET /dispatch/conditional
  |> jq: `{ tier: "premium", verified: true }`
  |> dispatch
    case @guard(`.tier == "premium"`):
      |> jq: `{ message: "Premium tier", features: ["all"] }`
    case @guard(`.tier == "standard"`):
      |> jq: `{ message: "Standard tier", features: ["basic"] }`
    default:
      |> jq: `{ message: "Free tier", features: [] }`
```

### Real-World Example: ETL Pipeline

Conditional data transformation with validation, transformation, caching, and notification steps:

```wp
GET /pipeline/etl/:dataset
  |> jq: `{
    dataset: .params.dataset,
    config: {
      validate: true,
      transform: true,
      cache: true,
      notify: false
    },
    rawData: [...]
  }`
  |> jq: `. + { validationErrors: (...) }` @guard(`.config.validate == true`)
  |> jq: `. + { transformed: ... }` @guard(`.config.transform == true and (.validationErrors // 0) < 2`)
  |> jq: `. + { cached: true }` @guard(`.config.cache == true`)
  |> jq: `. + { notificationSent: true }` @guard(`.config.notify == true`)
  |> jq: `{ dataset: .dataset, status: "complete" }`
```

### Comparison: @guard vs @flag/@env

| Feature | @guard | @flag / @env |
|---------|--------|-------------|
| Evaluation | Runtime data | Static configuration |
| Syntax | JQ expression | Tag name |
| Based on | Request state | Environment/flags |
| Flexibility | Any condition | Predefined flags |
| Use case | Data-driven logic | Feature toggles |

---

## @when Tag and Dynamic Conditions

The `@when` tag enables conditional execution based on transient runtime conditions set dynamically via Lua's `setWhen()` function. Unlike `@flag` (persistent configuration), `@when` is designed for request-specific classification and routing.

### Basic Syntax

*   **`@when(condition-name)`**: Execute only if condition is true
*   **`@!when(condition-name)`**: Execute only if condition is false

Conditions are set in Lua:
```lua
setWhen("is_admin", true)
setWhen("is_premium", user.premium)
```

### Setting Conditions with setWhen()

Use Lua middleware to classify the request:

```wp
GET /test-when-basic
  |> lua: `setWhen("is_admin", true); return request`
  |> dispatch
    case @when(is_admin):
      |> jq: `{ role: "admin", access: "full" }`
    default:
      |> jq: `{ role: "guest", access: "limited" }`
```

### @when vs @flag: Independent Namespaces

Feature flags and dynamic conditions are separate:

```wp
GET /test-when-vs-flag-isolation
  |> lua: `setWhen("beta", true); return request`
  |> dispatch
    case @flag(beta):
      |> jq: `{ error: "BUG: beta is a condition, not a flag" }`
    case @when(beta):
      |> jq: `{ success: true, message: "Correctly matched @when(beta)" }`
```

### Negated @when

Check for absence of condition:

```wp
GET /test-when-negated
  |> lua: `setWhen("is_premium", false); return request`
  |> dispatch
    case @!when(is_premium):
      |> jq: `{ message: "Free tier - upgrade for more features" }`
    case @when(is_premium):
      |> jq: `{ message: "Premium tier - all features unlocked" }`
```

### Multiple Conditions (AND Logic)

Multiple condition names in one `@when` tag require ALL to be true:

```wp
GET /test-when-multiple
  |> lua: `
    setWhen("is_authenticated", true)
    setWhen("is_admin", true)
    return request
  `
  |> dispatch
    case @when(is_authenticated,is_admin):
      |> jq: `{ access: "admin_panel", level: "superuser" }`
    case @when(is_authenticated):
      |> jq: `{ access: "dashboard", level: "user" }`
    default:
      |> jq: `{ access: "login", level: "anonymous" }`
```

### Accessing Conditions in JQ

Conditions are available via `$context.conditions`:

```wp
GET /test-when-context-jq
  |> lua: `setWhen("from_mobile", true); setWhen("dark_mode", false); return request`
  |> jq: `{
    conditions: $context.conditions,
    from_mobile: $context.conditions.from_mobile,
    dark_mode: $context.conditions.dark_mode
  }`
```

### Accessing Conditions in Lua

Use `context.conditions` or `getWhen()`:

```wp
GET /test-when-context-lua
  |> lua: `setWhen("test_condition", true); return request`
  |> lua: `
    return {
      conditions = context.conditions,
      test_condition = context.conditions.test_condition or false
    }
  `
```

Or with `getWhen()`:

```wp
GET /test-get-when
  |> lua: `setWhen("feature_x", true); return request`
  |> lua: `
    local hasFeatureX = getWhen("feature_x")
    local hasFeatureY = getWhen("feature_y")  -- not set, returns false
    return {
      feature_x = hasFeatureX,
      feature_y = hasFeatureY
    }
  `
```

### @when on Individual Steps

Use `@when` tags on any pipeline step, not just in dispatch:

```wp
GET /test-when-on-step
  |> jq: `{ base: "data" }`
  |> lua: `setWhen("add_metadata", true); return request`
  |> jq: `. + { metadata: { timestamp: "2024-01-01" } }` @when(add_metadata)
  |> jq: `. + { skipped: true }` @when(never_set_condition)
```

### Boolean Expressions with @when

Combine with `and` / `or`:

```wp
GET /test-dispatch-and
  |> lua: `
    setWhen("authenticated", true)
    setWhen("admin", true)
    return request
  `
  |> dispatch
    case @when(authenticated) and @when(admin):
      |> jq: `{ access: "admin_panel" }`
    default:
      |> jq: `{ access: "denied" }`
```

Mix with negated conditions:

```wp
GET /test-dispatch-negated-expr
  |> lua: `
    setWhen("authenticated", true)
    setWhen("banned", false)
    return request
  `
  |> dispatch
    case @when(authenticated) and @!when(banned):
      |> jq: `{ status: "welcome" }`
    default:
      |> jq: `{ status: "blocked" }`
```

### Use Cases

**Request Classification:**
```lua
-- Classify device type
local userAgent = request.headers["user-agent"] or ""
setWhen("mobile", string.find(userAgent:lower(), "mobile") ~= nil)
setWhen("tablet", string.find(userAgent:lower(), "tablet") ~= nil)
```

**Access Control:**
```lua
-- Check user permissions
setWhen("is_authenticated", request.user ~= nil)
setWhen("is_admin", request.user and request.user.role == "admin")
setWhen("is_premium", request.user and request.user.premium == true)
```

**Feature Eligibility:**
```lua
-- Determine feature access
local score = calculateUserScore(request.user)
setWhen("high_quality_user", score > 80)
setWhen("eligible_for_beta", score > 50 and request.user.verified)
```

### Comparison: @when vs @flag vs @guard

| Feature | @when | @flag | @guard |
|---------|-------|-------|--------|
| Set by | `setWhen()` in Lua | Config/`setFlag()` | N/A (inline JQ) |
| Lifetime | Request-scoped | Persistent | N/A (evaluated per-step) |
| Evaluation | Tag evaluation | Tag evaluation | Runtime JQ eval |
| Namespace | Conditions | Flags | N/A |
| Use case | Request classification | Feature toggles | Data-driven logic |

---

## See Also

- [Context & Metadata](./context.md) - Accessing `$context.conditions` and `$context.flags`
- [DSL Syntax](./dsl-syntax.md) - Complete tag reference and syntax guide
- [Testing & BDD](./testing-bdd.md) - Testing dispatch, foreach, guards, and conditions

