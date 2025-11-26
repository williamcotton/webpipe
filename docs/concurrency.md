# Concurrency & Async

WebPipe 2.0 introduces parallelism to the DSL, allowing you to execute long-running tasks in the background and join their results.

## Async Steps

You can tag any pipeline step with `@async(taskName)` to spawn it as a background task. The pipeline will immediately proceed to the next step without waiting for the async step to complete.

```wp
GET /dashboard
  # Start fetching user data in background
  |> fetch: `https://api.example.com/users` @async(users)
  
  # Start fetching posts in background
  |> fetch: `https://api.example.com/posts` @async(posts)
  
  # Do some other work while waiting...
  |> log: "Fetching data..."
```

## Joining Results

To retrieve the results of async tasks, use the `join` middleware. This step will suspend execution until the specified tasks are complete.

```wp
  # ... previous async steps ...
  
  # Wait for both tasks to finish
  |> join: `users, posts`
  
  # Results are available in the .async object
  |> jq: `{ 
      userCount: .async.users.data.length, 
      postCount: .async.posts.data.length 
    }`
```

The results of the async tasks are merged into the pipeline input under the `.async` key, keyed by the task name provided in the `@async` tag.

## Example Flow

```wp
GET /summary
  |> jq: `{ status: "processing" }`
  
  # Fork: Fetch external data
  |> fetch: `https://analytics.service/stats` @async(external_stats)
  
  # Fork: Run heavy DB query
  |> pg: `SELECT count(*) FROM huge_table` @async(db_stats)
  
  # Main thread continues...
  
  # Join: Sync both tasks
  |> join: `external_stats, db_stats`
  
  # Merge results
  |> jq: `{ 
      analytics: .async.external_stats.data, 
      db: .async.db_stats.data.rows[0] 
    }`
```

