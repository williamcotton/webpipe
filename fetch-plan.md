# Fetch Middleware Implementation Plan

## 1. Syntax and Usage

### Basic Usage
```wp
GET /proxy/:id
  |> jq: `{ fetchUrl: "https://api.example.com/users/" + .params.id }`
  |> fetch: `https://api.example.com/users`
```

### With Variables
```wp
fetch getUserApi = `https://api.example.com/users`

GET /user/:id
  |> jq: `{ fetchUrl: "https://api.example.com/users/" + .params.id }`
  |> fetch: getUserApi
```

### With Named Results
```wp
GET /user/:id
  |> jq: `{ resultName: "userProfile" }`
  |> fetch: `https://api.example.com/users`
  |> jq: `{ user: .data.userProfile.response }`
```

### Multiple Named Results
```wp
GET /user-data/:id
  |> jq: `{ resultName: "userInfo" }`
  |> fetch: `https://api.example.com/users`
  |> jq: `{ resultName: "userTeams" }`
  |> fetch: `https://api.example.com/teams`
  |> jq: `{ 
    profile: .data.userInfo.response,
    teams: .data.userTeams.response,
    userApiWorked: (.data.userInfo.status == 200),
    teamsApiWorked: (.data.userTeams.status == 200)
  }`
```

## 2. Request Object Parameters

The fetch middleware reads these properties from the input JSON object:

- `fetchUrl`: Override URL in template literal
- `fetchMethod`: HTTP method (defaults to GET)
- `fetchHeaders`: Headers object
- `fetchBody`: Request body for POST/PUT/PATCH
- `fetchQuery`: Query parameters object
- `fetchTimeout`: Per-request timeout override
- `resultName`: Custom name for result in data object

## 3. Response Structure

### Unnamed Fetch
```json
{
  "data": {
    "response": "<response_body>",
    "status": 200,
    "headers": {"content-type": "application/json"}
  }
}
```

### Named Result or Variable
```json
{
  "data": {
    "variableName": {
      "response": "<response_body>",
      "status": 200,
      "headers": {"content-type": "application/json"}
    }
  }
}
```

## 4. JSON Object Flow Examples

### Example 1: Simple GET Request (Unnamed)
```wp
GET /users/:id
  |> jq: `{ fetchUrl: "https://api.example.com/users/" + .params.id }`
  |> fetch: `https://api.example.com/users`
  |> jq: `{ user: .data.response, statusCode: .data.status }`
```

**Flow:**
1. **Initial request object:**
   ```json
   {
     "method": "GET",
     "params": { "id": "123" },
     "query": {},
     "headers": { "host": "localhost:8080" },
     "body": null
   }
   ```

2. **After jq step:**
   ```json
   {
     "method": "GET",
     "params": { "id": "123" },
     "query": {},
     "headers": { "host": "localhost:8080" },
     "body": null,
     "fetchUrl": "https://api.example.com/users/123"
   }
   ```

3. **After fetch step:**
   ```json
   {
     "method": "GET",
     "params": { "id": "123" },
     "query": {},
     "headers": { "host": "localhost:8080" },
     "body": null,
     "fetchUrl": "https://api.example.com/users/123",
     "data": {
       "response": { "id": 123, "name": "John Doe", "email": "john@example.com" },
       "status": 200,
       "headers": { "content-type": "application/json" }
     }
   }
   ```

### Example 2: Named Result
```wp
GET /user/:id
  |> jq: `{ resultName: "userInfo" }`
  |> fetch: `https://api.example.com/users`
  |> jq: `{ 
    user: .data.userInfo.response, 
    wasSuccessful: (.data.userInfo.status < 400)
  }`
```

**After fetch step:**
```json
{
  "method": "GET",
  "params": { "id": "123" },
  "data": {
    "userInfo": {
      "response": { "id": 123, "name": "John Doe" },
      "status": 200,
      "headers": { "content-type": "application/json" }
    }
  }
}
```

### Example 3: Variable Name
```wp
fetch getUserApi = `https://api.example.com/users`

GET /user/:id
  |> fetch: getUserApi
  |> jq: `{ 
    user: .data.getUserApi.response,
    apiStatus: .data.getUserApi.status
  }`
```

**After fetch step:**
```json
{
  "method": "GET",
  "params": { "id": "123" },
  "data": {
    "getUserApi": {
      "response": { "id": 123, "name": "John Doe" },
      "status": 200,
      "headers": { "content-type": "application/json" }
    }
  }
}
```

## 5. Configuration Block

```wp
config fetch {
  timeout: 30
  userAgent: "WebPipe/1.0"
  followRedirects: true
  maxRedirects: 5
  allowedDomains: ["api.example.com", "*.trusted-service.com"]
  blockedDomains: ["internal.company.com"]
}
```

## 6. Implementation Details

- Use libcurl for HTTP requests with connection pooling
- Follow pg.c architecture for configuration and connection management
- Support resultName and variable naming exactly like pg middleware
- Standard error handling with `httpError`, `networkError`, `timeoutError`
- Generate appropriate error objects that work with result step patterns

## 7. Error Handling

The middleware should generate standard error objects:

- `httpError`: For HTTP status codes >= 400  
- `networkError`: For connection failures, DNS resolution issues
- `timeoutError`: For request timeouts
- Follow existing error format with `errors` array

## 8. Security Considerations

- SSRF prevention via allowedDomains/blockedDomains
- Request size limits
- Response size limits
- Timeout enforcement