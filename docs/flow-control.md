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

The pipeline must return a JSON object with a `_metadata` field containing a `flags` map.

```wp
pipeline featureFlags =
  |> jq: `{ 
      _metadata: {
        flags: {
          "use-stripe": (.headers["x-beta-tester"] == "true"),
          "new-ui": true 
        }
      }
    }`

# Flags are now available for @flag checks
```

