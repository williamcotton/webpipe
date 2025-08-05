FROM wp:latest

# Copy our demo application
COPY app.wp /app/

# Expose port 80 for production-like setup
EXPOSE 80

# Run the application
CMD ["app.wp", "--port", "80"]