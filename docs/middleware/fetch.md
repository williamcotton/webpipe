### fetch Middleware

Perform HTTP requests to external APIs.

Inputs:
- `fetchUrl` (overrides inline URL)
- `fetchMethod` (default GET)
- `fetchHeaders` (object)
- `fetchBody` (JSON for POST/PUT)
- `fetchTimeout` (seconds)
- `resultName` (to place under `.data.<name>`)

Caching:
- Use the `cache` middleware before `fetch` to enable response caching.
- Cache key from `keyTemplate` or an automatic hash of method/url/headers/body.

Errors: `networkError`, `timeoutError`, `httpError`.

Basic Example:
```wp
|> jq: `{ fetchUrl: "https://api.github.com/zen", resultName: "api" }`
|> fetch: `_`
|> jq: `{ zen: .data.api.response }`
```

Inline URL Example:
```wp
|> fetch("https://api.example.com/users/" + (.userId | tostring))
```

Using @result Tag:
```wp
|> fetch("https://api.example.com/users/1") @result(profile)
|> fetch("https://api.example.com/posts?userId=1") @result(userPosts)
|> jq: `{
  user: .data.profile.response,
  posts: .data.userPosts.response
}`
```

Async Fetch with @result:
```wp
|> fetch("https://api.example.com/users/1") @async(user) @result(profile)
|> fetch("https://api.example.com/posts?userId=1") @async(posts) @result(userPosts)
|> join: `user, posts`
|> jq: `{
  user: .async.user.data.profile.response,
  posts: .async.posts.data.userPosts.response
}`
```


