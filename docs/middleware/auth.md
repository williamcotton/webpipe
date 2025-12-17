### auth Middleware

Flows:
- `login`: validates credentials, creates session, sets cookie (`setCookies`), attaches `.user`
- `register`: creates user, attaches `.user`
- `logout`: clears session, returns `setCookies` with expired cookie
- `required`: ensures valid session and attaches `.user` or emits `authError`
- `optional`: attaches `.user` if valid session, otherwise passes through
- `type:<role>`: requires `.user.type == role` else `authError`

Cookies:
- Name and flags from auth config (`cookieName`, `HttpOnly`, `Secure`, `SameSite`, `Path`, `Max-Age`).

Errors: `authError { message }`.

## Security Configuration

**IMPORTANT: Production builds enforce secure cookie settings.**

### Required Settings for Production

```wp
config auth {
  sessionTtl: 604800
  cookieName: "wp_session"
  cookieSecure: true        # REQUIRED in production (enforced)
  cookieHttpOnly: true      # Prevents XSS attacks
  cookieSameSite: "Strict"  # Recommended: prevents CSRF attacks
  cookiePath: "/"
}
```

### Cookie Security Flags

- **`cookieSecure: true`** - Cookies only sent over HTTPS (ENFORCED in production builds)
  - Production builds will ERROR if this is false
  - Development builds will WARN but allow for local testing

- **`cookieHttpOnly: true`** - Prevents JavaScript access to cookies (protects against XSS)
  - Should always be enabled (default: true)

- **`cookieSameSite`** - CSRF protection:
  - `"Strict"` - Most secure, prevents all cross-site requests (recommended)
  - `"Lax"` - Allows top-level GET navigation (default)
  - `"None"` - Allows all cross-site requests (requires `cookieSecure: true`)

### Attack Prevention

| Attack Type | Protection | Configuration |
|-------------|-----------|---------------|
| Session Hijacking (MITM) | HTTPS + Secure flag | `cookieSecure: true` |
| XSS Cookie Theft | HttpOnly flag | `cookieHttpOnly: true` |
| CSRF | SameSite flag | `cookieSameSite: "Strict"` |

### Development vs Production

**Development (debug builds):**
- Insecure settings trigger warnings
- Allows HTTP for local testing
- `cookieSecure: false` permitted

**Production (release builds):**
- Insecure settings cause errors
- `cookieSecure: false` will prevent server from issuing session cookies
- Always deploy with HTTPS

---

## Manual Cookie Management

For advanced cookie management beyond auth sessions, you can manually set cookies in your response JSON:

```wp
GET /custom-cookies
  |> jq: `{
    message: "Response",
    setCookies: [
      "customId=abc123; HttpOnly; Secure; Max-Age=3600; Path=/",
      "preference=darkMode; Max-Age=86400; Path=/"
    ]
  }`
```

See [Result Routing - Cookie Management](../result-routing.md#cookie-management) for comprehensive cookie documentation including:
- Cookie syntax and attributes
- Multiple cookies
- Cookie security best practices
- Deleting cookies with Max-Age=0


