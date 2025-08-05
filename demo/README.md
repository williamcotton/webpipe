# Web Pipe Demo Application

This is a simple demo application showcasing how to use the Web Pipe (wp) Docker container.

## Quick Start

1. **Build the demo application:**
   ```bash
   cd demo
   docker build -t wp-demo .
   ```

2. **Run the application:**
   ```bash
   docker run -p 8080:80 wp-demo
   ```

3. **Test the endpoints:**
   ```bash
   # Welcome page
   curl http://localhost:8080/

   # Health check
   curl http://localhost:8080/health

   # User API
   curl http://localhost:8080/api/users/123

   # Echo API
   curl -X POST http://localhost:8080/api/echo \
        -H "Content-Type: application/json" \
        -d '{"hello": "world"}'

   # Lua transformation
   curl http://localhost:8080/api/transform/HelloWorld

   # HTML greeting (open in browser)
   open http://localhost:8080/api/greeting/YourName
   ```

## What This Demonstrates

- **Multi-stage Docker build** using the wp base image
- **JSON transformations** with jq middleware
- **Lua scripting** for custom logic
- **HTML templating** with mustache middleware
- **Multiple HTTP methods** (GET, POST)
- **URL parameters** and request body handling
- **Production-ready deployment** on port 80

## Application Structure

```
demo/
├── Dockerfile          # Uses wp:latest as base image
├── app.wp              # Web Pipe application definition
└── README.md           # This file
```

## Base Image

This demo uses the `wp:latest` Docker image which includes:
- Pre-built wp runtime
- All middleware (.so files)
- Ubuntu 22.04 base with all dependencies
- Non-root user setup for security

## Development

To modify the application:

1. Edit `app.wp` with your routes and middleware
2. Rebuild: `docker build -t wp-demo .`
3. Run: `docker run -p 8080:80 wp-demo`

The base wp image handles all the complexity of:
- C compilation and linking
- Dependency management
- Middleware loading
- Security configuration

You just focus on writing your `.wp` application logic!