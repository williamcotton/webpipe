### Docker & Deployment

Example Dockerfile (uses prebuilt image):

```Dockerfile
FROM ghcr.io/williamcotton/webpipe:latest

COPY app.wp /app/
COPY public /app/public/
COPY scripts /app/scripts/

CMD ["app.wp"]
```

Run:
```bash
docker build -t myapp .
docker run -p 7770:7770 --env-file .env myapp
```


