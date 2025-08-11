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


