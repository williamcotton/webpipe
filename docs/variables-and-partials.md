### Variables & Partials

- **Variables**: name reusable SQL, templates, etc.
- **Partials** (Handlebars): define fragments and reuse with `{{>partialName}}`.

```wp
handlebars baseLayout = `
<!DOCTYPE html>
<html><head><title>{{> title}}</title></head>
<body>{{> content}}</body></html>
`

pg getTeams = `SELECT * FROM teams`
```


