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
docker run -p 8090:8090 --env-file .env myapp
```


