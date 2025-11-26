# Join Middleware

The `join` middleware is the synchronization primitive for WebPipe's concurrency model. It waits for specified async tasks to complete and merges their results.

## Usage

The configuration string is a comma-separated list of task names that were previously spawned using the `@async(name)` tag.

```wp
|> join: `task1, task2`
```

## Behavior

1.  **Waits**: Suspends the current pipeline until all listed tasks (Tokio handles) have completed.
2.  **Merges**: Inserts the results of the tasks into the current pipeline input (JSON object) under the `.async` key.
3.  **Error Handling**: If an async task fails, its error is captured in the result object (e.g., `.async.task1.error`).

## Example

See [Concurrency & Async](../concurrency.md) for full examples.

