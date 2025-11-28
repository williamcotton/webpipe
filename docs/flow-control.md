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

```wp
GET /checkout
  |> pipeline: stripeProcessor @flag(use-stripe)
  |> pipeline: paypalProcessor @!flag(use-stripe)
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

