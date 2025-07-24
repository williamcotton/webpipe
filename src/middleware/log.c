#include <jansson.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <stdbool.h>
#include <pthread.h>
#include <sys/time.h>

// Arena allocation function types for middleware
typedef void *(*arena_alloc_func)(void *arena, size_t size);
typedef void (*arena_free_func)(void *arena);

// Log levels
typedef enum {
    LOG_LEVEL_DEBUG = 0,
    LOG_LEVEL_INFO,
    LOG_LEVEL_WARN,
    LOG_LEVEL_ERROR
} LogLevel;

// Log configuration
typedef struct {
    bool enabled;
    char *output;           // "file", "stdout", "stderr"
    char *log_file;         // File path for file output
    char *format;           // "json", "clf", "combined"
    LogLevel level;         // Minimum log level
    bool include_body;      // Log request/response bodies
    bool include_headers;   // Log headers
    int max_body_size;      // Maximum body size to log
    bool timestamp;         // Include timestamps
} LogConfig;

// Step-specific configuration
typedef struct {
    LogLevel level;
    bool include_body;
    bool include_headers;
    bool enabled;
} LogStepConfig;

// Global configuration with defaults
static LogConfig global_config = {
    .enabled = true,
    .output = NULL,
    .log_file = NULL,
    .format = NULL,
    .level = LOG_LEVEL_INFO,
    .include_body = false,
    .include_headers = true,
    .max_body_size = 1024,
    .timestamp = true
};

// Global file handle for log output
static FILE *log_file_handle = NULL;

// Mutex for thread-safe logging
static pthread_mutex_t log_mutex = PTHREAD_MUTEX_INITIALIZER;

// Function prototypes
static LogLevel parse_log_level(const char *level_str);
static const char *log_level_to_string(LogLevel level);
static LogStepConfig parse_step_config(const char *config, void *arena, arena_alloc_func alloc_func);
static void log_request_response(json_t *request, json_t *response, double duration_ms, LogStepConfig step_config);
static double get_current_time_ms(void);
static char *format_timestamp(void);
static int open_log_file(void);
static void close_log_file(void);
static void write_log_entry(const char *log_str);
static char *format_clf_log(json_t *request, json_t *response, double duration_ms);
static char *extract_client_ip(json_t *request);
static char *extract_user_agent(json_t *request);
static json_t *sanitize_headers(json_t *headers);
static bool should_log_request(json_t *request);
static void cleanup_global_config(void);

// Public function prototypes (middleware interface)
int middleware_init(json_t *config);
json_t *middleware_execute(json_t *input, void *arena, arena_alloc_func alloc_func, 
                          arena_free_func free_func, const char *config,
                          json_t *middleware_config, char **contentType, json_t *variables);
void middleware_post_execute(json_t *final_response, void *arena, arena_alloc_func alloc_func, 
                           json_t *middleware_config);
void middleware_cleanup(void); // Optional cleanup function

// Parse log level from string
static LogLevel parse_log_level(const char *level_str) {
    if (!level_str) return LOG_LEVEL_INFO;
    
    if (strcmp(level_str, "debug") == 0) return LOG_LEVEL_DEBUG;
    if (strcmp(level_str, "info") == 0) return LOG_LEVEL_INFO;
    if (strcmp(level_str, "warn") == 0) return LOG_LEVEL_WARN;
    if (strcmp(level_str, "error") == 0) return LOG_LEVEL_ERROR;
    
    return LOG_LEVEL_INFO; // Default
}

// Convert log level to string
static const char *log_level_to_string(LogLevel level) {
    switch (level) {
        case LOG_LEVEL_DEBUG: return "debug";
        case LOG_LEVEL_INFO: return "info";
        case LOG_LEVEL_WARN: return "warn";
        case LOG_LEVEL_ERROR: return "error";
        default: return "info";
    }
}

// Get current time in milliseconds
static double get_current_time_ms(void) {
    struct timeval tv;
    gettimeofday(&tv, NULL);
    return (double)(tv.tv_sec * 1000 + tv.tv_usec / 1000);
}

// Format timestamp as ISO 8601 string
static char *format_timestamp(void) {
    static char timestamp[32];
    struct timeval tv;
    struct tm *tm_info;
    
    gettimeofday(&tv, NULL);
    tm_info = gmtime(&tv.tv_sec);
    
    snprintf(timestamp, sizeof(timestamp), "%04d-%02d-%02dT%02d:%02d:%02d.%03dZ",
             tm_info->tm_year + 1900, tm_info->tm_mon + 1, tm_info->tm_mday,
             tm_info->tm_hour, tm_info->tm_min, tm_info->tm_sec,
             (int)(tv.tv_usec / 1000));
    
    return timestamp;
}

// Open log file for writing
static int open_log_file(void) {
    if (!global_config.log_file) {
        return -1;
    }
    
    if (log_file_handle) {
        fclose(log_file_handle);
    }
    
    log_file_handle = fopen(global_config.log_file, "a");
    if (!log_file_handle) {
        fprintf(stderr, "Log middleware: Failed to open log file: %s\n", global_config.log_file);
        return -1;
    }
    
    return 0;
}

// Close log file
static void close_log_file(void) {
    if (log_file_handle) {
        fclose(log_file_handle);
        log_file_handle = NULL;
    }
}

// Write log entry to configured output
static void write_log_entry(const char *log_str) {
    if (!log_str) return;
    
    if (global_config.output && strcmp(global_config.output, "file") == 0) {
        if (log_file_handle) {
            fprintf(log_file_handle, "%s\n", log_str);
            fflush(log_file_handle);
        } else {
            // Fallback to stderr if file not available
            fprintf(stderr, "%s\n", log_str);
        }
    } else if (global_config.output && strcmp(global_config.output, "stderr") == 0) {
        fprintf(stderr, "%s\n", log_str);
        fflush(stderr);
    } else {
        // Default: stdout
        printf("%s\n", log_str);
        fflush(stdout);
    }
}

// Extract client IP from request headers
static char *extract_client_ip(json_t *request) {
    static char ip_buffer[64];
    strcpy(ip_buffer, "-"); // Default value
    
    if (!request) return ip_buffer;
    
    json_t *headers = json_object_get(request, "headers");
    if (!headers) return ip_buffer;
    
    // Check common headers for client IP
    const char *ip_headers[] = {
        "X-Forwarded-For",
        "X-Real-IP", 
        "X-Client-IP",
        "CF-Connecting-IP",
        "X-Forwarded",
        "Forwarded-For",
        "Forwarded"
    };
    
    for (size_t i = 0; i < sizeof(ip_headers) / sizeof(ip_headers[0]); i++) {
        json_t *ip_json = json_object_get(headers, ip_headers[i]);
        if (ip_json && json_is_string(ip_json)) {
            const char *ip_str = json_string_value(ip_json);
            if (ip_str && strlen(ip_str) > 0 && strcmp(ip_str, "-") != 0) {
                // For X-Forwarded-For, take the first IP (client)
                char *comma = strchr(ip_str, ',');
                if (comma) {
                    size_t len = (size_t)(comma - ip_str);
                    if (len < sizeof(ip_buffer) - 1) {
                        strncpy(ip_buffer, ip_str, len);
                        ip_buffer[len] = '\0';
                        return ip_buffer;
                    }
                } else {
                    strncpy(ip_buffer, ip_str, sizeof(ip_buffer) - 1);
                    ip_buffer[sizeof(ip_buffer) - 1] = '\0';
                    return ip_buffer;
                }
            }
        }
    }
    
    return ip_buffer;
}

// Extract user agent from request headers
static char *extract_user_agent(json_t *request) {
    static char ua_buffer[512];
    strcpy(ua_buffer, "-"); // Default value
    
    if (!request) return ua_buffer;
    
    json_t *headers = json_object_get(request, "headers");
    if (!headers) return ua_buffer;
    
    json_t *ua_json = json_object_get(headers, "User-Agent");
    if (ua_json && json_is_string(ua_json)) {
        const char *ua_str = json_string_value(ua_json);
        if (ua_str && strlen(ua_str) > 0) {
            strncpy(ua_buffer, ua_str, sizeof(ua_buffer) - 1);
            ua_buffer[sizeof(ua_buffer) - 1] = '\0';
        }
    }
    
    return ua_buffer;
}

// Format log entry in Common Log Format (CLF)
static char *format_clf_log(json_t *request, json_t *response, double duration_ms) {
    static char clf_buffer[2048];
    
    // CLF format: host ident authuser [timestamp] "request" status size
    // Extended with response time: host ident authuser [timestamp] "request" status size response_time
    
    char *client_ip = extract_client_ip(request);
    char *timestamp = format_timestamp();
    char *user_agent = extract_user_agent(request);
    
    const char *method = "-";
    const char *url = "-";
    int status = 200;
    const char *size = "-";
    
    if (request) {
        json_t *method_json = json_object_get(request, "method");
        if (method_json && json_is_string(method_json)) {
            method = json_string_value(method_json);
        }
        
        json_t *url_json = json_object_get(request, "url");
        if (url_json && json_is_string(url_json)) {
            url = json_string_value(url_json);
        }
    }
    
    if (response) {
        json_t *status_json = json_object_get(response, "status");
        if (status_json && json_is_integer(status_json)) {
            status = (int)json_integer_value(status_json);
        }
    }
    
    // Format: IP - - [timestamp] "METHOD URL HTTP/1.1" status size response_time "user_agent"
    snprintf(clf_buffer, sizeof(clf_buffer),
             "%s - - [%s] \"%s %s HTTP/1.1\" %d %s %.0f \"%s\"",
             client_ip, timestamp, method, url, status, size, duration_ms, user_agent);
    
    return clf_buffer;
}

// Sanitize headers by removing sensitive information
static json_t *sanitize_headers(json_t *headers) {
    if (!headers || !json_is_object(headers)) {
        return json_object();
    }
    
    json_t *sanitized = json_object();
    const char *key;
    json_t *value;
    
    // List of headers that should be sanitized or excluded
    const char *sensitive_headers[] = {
        "Authorization",
        "Cookie",
        "X-API-Key",
        "X-Auth-Token",
        "X-Access-Token",
        "Bearer",
        "JWT",
        "X-Session-ID",
        "Set-Cookie"
    };
    
    json_object_foreach(headers, key, value) {
        bool is_sensitive = false;
        
        // Check if this header should be sanitized
        for (size_t i = 0; i < sizeof(sensitive_headers) / sizeof(sensitive_headers[0]); i++) {
            if (strcasecmp(key, sensitive_headers[i]) == 0) {
                is_sensitive = true;
                break;
            }
        }
        
        if (is_sensitive) {
            json_object_set_new(sanitized, key, json_string("[REDACTED]"));
        } else {
            json_object_set(sanitized, key, value);
        }
    }
    
    return sanitized;
}

// Check if request should be logged (filtering for health checks, etc.)
static bool should_log_request(json_t *request) {
    if (!request) {
        return true; // Default to logging
    }
    
    json_t *url_json = json_object_get(request, "url");
    if (!url_json || !json_is_string(url_json)) {
        return true;
    }
    
    const char *url = json_string_value(url_json);
    if (!url) {
        return true;
    }
    
    // Skip common health check endpoints
    const char *skip_paths[] = {
        "/health",
        "/healthz",
        "/ping",
        "/status",
        "/metrics",
        "/favicon.ico"
    };
    
    for (size_t i = 0; i < sizeof(skip_paths) / sizeof(skip_paths[0]); i++) {
        if (strcmp(url, skip_paths[i]) == 0) {
            return false;
        }
    }
    
    return true;
}

// Cleanup global configuration
static void cleanup_global_config(void) {
    if (global_config.output) {
        free(global_config.output);
        global_config.output = NULL;
    }
    if (global_config.log_file) {
        free(global_config.log_file);
        global_config.log_file = NULL;
    }
    if (global_config.format) {
        free(global_config.format);
        global_config.format = NULL;
    }
    close_log_file();
}

// Parse step-specific configuration (WebPipe native format)
static LogStepConfig parse_step_config(const char *config, void *arena, arena_alloc_func alloc_func) {
    LogStepConfig step_config = {
        .level = global_config.level,
        .include_body = global_config.include_body,
        .include_headers = global_config.include_headers,
        .enabled = global_config.enabled
    };
    
    if (!config || strlen(config) == 0) {
        return step_config;
    }
    
    // Create a copy of the config string for parsing
    size_t config_len = strlen(config);
    char *config_copy = alloc_func(arena, config_len + 1);
    if (!config_copy) {
        return step_config;
    }
    memcpy(config_copy, config, config_len);
    config_copy[config_len] = '\0';
    
    // Parse line by line
    char *saveptr = NULL;
    char *line = strtok_r(config_copy, "\n", &saveptr);
    
    while (line) {
        // Skip leading whitespace
        while (*line && (*line == ' ' || *line == '\t')) {
            line++;
        }
        
        // Skip empty lines and comments
        if (*line == '\0' || *line == '#') {
            line = strtok_r(NULL, "\n", &saveptr);
            continue;
        }
        
        // Find the colon separator
        char *colon = strchr(line, ':');
        if (colon) {
            *colon = '\0';
            char *key = line;
            char *value = colon + 1;
            
            // Trim key
            char *key_end = key + strlen(key) - 1;
            while (key_end > key && (*key_end == ' ' || *key_end == '\t')) {
                *key_end = '\0';
                key_end--;
            }
            
            // Trim value
            while (*value && (*value == ' ' || *value == '\t')) {
                value++;
            }
            char *value_end = value + strlen(value) - 1;
            while (value_end > value && (*value_end == ' ' || *value_end == '\t')) {
                *value_end = '\0';
                value_end--;
            }
            
            // Parse specific keys
            if (strcmp(key, "level") == 0) {
                step_config.level = parse_log_level(value);
            } else if (strcmp(key, "includeBody") == 0) {
                step_config.include_body = (strcmp(value, "true") == 0);
            } else if (strcmp(key, "includeHeaders") == 0) {
                step_config.include_headers = (strcmp(value, "true") == 0);
            } else if (strcmp(key, "enabled") == 0) {
                step_config.enabled = (strcmp(value, "true") == 0);
            }
        }
        
        line = strtok_r(NULL, "\n", &saveptr);
    }
    
    return step_config;
}

// Log request and response with configurable output and formatting
static void log_request_response(json_t *request, json_t *response, double duration_ms, LogStepConfig step_config) {
    if (!step_config.enabled) {
        return;
    }
    
    // Filter by log level
    if (step_config.level < global_config.level) {
        return;
    }
    
    // Skip logging for filtered requests (health checks, etc.)
    if (!should_log_request(request)) {
        return;
    }
    
    // Thread-safe logging
    pthread_mutex_lock(&log_mutex);
    
    char *log_str = NULL;
    
    // Format based on global configuration
    if (global_config.format && strcmp(global_config.format, "clf") == 0) {
        // Common Log Format - use static buffer, no need to strdup
        log_str = format_clf_log(request, response, duration_ms);
    } else if (global_config.format && strcmp(global_config.format, "combined") == 0) {
        // Combined Log Format (Apache-style) - use static buffer, no need to strdup
        log_str = format_clf_log(request, response, duration_ms);
    } else {
        // JSON format (default)
        json_t *log_entry = json_object();
        
        // Add timestamp
        if (global_config.timestamp) {
            json_object_set_new(log_entry, "timestamp", json_string(format_timestamp()));
        }
        
        // Add log level
        json_object_set_new(log_entry, "level", json_string(log_level_to_string(step_config.level)));
        
        // Add request information
        json_t *req_log = json_object();
        if (request) {
            json_t *method = json_object_get(request, "method");
            if (method && json_is_string(method)) {
                json_object_set(req_log, "method", method);
            }
            
            json_t *url = json_object_get(request, "url");
            if (url && json_is_string(url)) {
                json_object_set(req_log, "url", url);
            }
            
            json_t *params = json_object_get(request, "params");
            if (params && json_object_size(params) > 0) {
                json_object_set(req_log, "params", params);
            }
            
            json_t *query = json_object_get(request, "query");
            if (query && json_object_size(query) > 0) {
                json_object_set(req_log, "query", query);
            }
            
            // Headers (if enabled, with sanitization)
            if (step_config.include_headers) {
                json_t *headers = json_object_get(request, "headers");
                if (headers && json_object_size(headers) > 0) {
                    json_t *sanitized_headers = sanitize_headers(headers);
                    json_object_set_new(req_log, "headers", sanitized_headers);
                }
            }
            
            // Body (if enabled and within size limit)
            if (step_config.include_body) {
                json_t *body = json_object_get(request, "body");
                if (body) {
                    // Check body size if it's a string
                    if (json_is_string(body)) {
                        const char *body_str = json_string_value(body);
                        if (body_str && (int)strlen(body_str) <= global_config.max_body_size) {
                            json_object_set(req_log, "body", body);
                        } else {
                            json_object_set_new(req_log, "body", json_string("[body too large]"));
                        }
                    } else {
                        // For non-string bodies, include if reasonable size
                        char *body_str = json_dumps(body, JSON_COMPACT);
                        if (body_str) {
                            if ((int)strlen(body_str) <= global_config.max_body_size) {
                                json_object_set(req_log, "body", body);
                            } else {
                                json_object_set_new(req_log, "body", json_string("[body too large]"));
                            }
                            free(body_str);
                        }
                    }
                }
            }
        }
        json_object_set_new(log_entry, "request", req_log);
        
        // Add response information
        json_t *resp_log = json_object();
        if (response) {
            // Add status (default 200 if not present)
            json_object_set_new(resp_log, "status", json_integer(200));
            
            // Include response body if enabled and within size limit
            if (step_config.include_body) {
                // Remove internal fields before logging
                json_t *clean_response = json_deep_copy(response);
                if (clean_response) {
                    json_object_del(clean_response, "_log_metadata");
                    json_object_del(clean_response, "_cache_metadata");
                    json_object_del(clean_response, "originalRequest");
                    json_object_del(clean_response, "setCookies");
                    
                    // Check response size
                    char *resp_str = json_dumps(clean_response, JSON_COMPACT);
                    if (resp_str) {
                        if ((int)strlen(resp_str) <= global_config.max_body_size) {
                            json_object_set_new(resp_log, "body", clean_response);
                        } else {
                            json_object_set_new(resp_log, "body", json_string("[response too large]"));
                            json_decref(clean_response);
                        }
                        free(resp_str);
                    } else {
                        json_decref(clean_response);
                    }
                }
            }
        }
        json_object_set_new(log_entry, "response", resp_log);
        
        // Add duration
        json_object_set_new(log_entry, "duration_ms", json_real(duration_ms));
        
        // Add client IP and user agent for better observability
        if (request) {
            json_object_set_new(log_entry, "client_ip", json_string(extract_client_ip(request)));
            json_object_set_new(log_entry, "user_agent", json_string(extract_user_agent(request)));
        }
        
        // Convert to string
        log_str = json_dumps(log_entry, JSON_COMPACT);
        json_decref(log_entry);
    }
    
    // Write to configured output
    if (log_str) {
        write_log_entry(log_str);
        // Don't free log_str:
        // - For JSON format: it's arena-allocated memory during request processing
        // - For CLF format: it's a static buffer that shouldn't be freed
    }
    
    pthread_mutex_unlock(&log_mutex);
}

// Middleware initialization function
int middleware_init(json_t *config) {
    if (config) {
        // Parse global log configuration
        json_t *enabled_json = json_object_get(config, "enabled");
        if (enabled_json && json_is_boolean(enabled_json)) {
            global_config.enabled = json_boolean_value(enabled_json);
        }
        
        json_t *output_json = json_object_get(config, "output");
        if (output_json && json_is_string(output_json)) {
            const char *output = json_string_value(output_json);
            if (global_config.output) free(global_config.output);
            global_config.output = strdup(output);
        } else {
            if (global_config.output) free(global_config.output);
            global_config.output = strdup("stdout");
        }
        
        json_t *log_file_json = json_object_get(config, "logFile");
        if (log_file_json && json_is_string(log_file_json)) {
            const char *log_file = json_string_value(log_file_json);
            if (global_config.log_file) free(global_config.log_file);
            global_config.log_file = strdup(log_file);
        }
        
        json_t *format_json = json_object_get(config, "format");
        if (format_json && json_is_string(format_json)) {
            const char *format = json_string_value(format_json);
            if (global_config.format) free(global_config.format);
            global_config.format = strdup(format);
        } else {
            if (global_config.format) free(global_config.format);
            global_config.format = strdup("json");
        }
        
        json_t *level_json = json_object_get(config, "level");
        if (level_json && json_is_string(level_json)) {
            global_config.level = parse_log_level(json_string_value(level_json));
        }
        
        json_t *include_body_json = json_object_get(config, "includeBody");
        if (include_body_json && json_is_boolean(include_body_json)) {
            global_config.include_body = json_boolean_value(include_body_json);
        }
        
        json_t *include_headers_json = json_object_get(config, "includeHeaders");
        if (include_headers_json && json_is_boolean(include_headers_json)) {
            global_config.include_headers = json_boolean_value(include_headers_json);
        }
        
        json_t *max_body_size_json = json_object_get(config, "maxBodySize");
        if (max_body_size_json && json_is_integer(max_body_size_json)) {
            global_config.max_body_size = (int)json_integer_value(max_body_size_json);
        }
        
        json_t *timestamp_json = json_object_get(config, "timestamp");
        if (timestamp_json && json_is_boolean(timestamp_json)) {
            global_config.timestamp = json_boolean_value(timestamp_json);
        }
    } else {
        // Set defaults if no config provided
        if (global_config.output) free(global_config.output);
        global_config.output = strdup("stdout");
        if (global_config.format) free(global_config.format);
        global_config.format = strdup("json");
    }
    
    // Open log file if using file output
    if (global_config.output && strcmp(global_config.output, "file") == 0) {
        if (open_log_file() != 0) {
            fprintf(stderr, "Log middleware: Warning - falling back to stderr output\n");
            if (global_config.output) free(global_config.output);
            global_config.output = strdup("stderr");
        }
    }
    
    printf("Log middleware initialized: enabled=%s, output=%s, level=%s\n",
           global_config.enabled ? "true" : "false",
           global_config.output ? global_config.output : "stdout",
           log_level_to_string(global_config.level));
    
    return 0; // Success
}

// Middleware execute function (log request and store metadata)
json_t *middleware_execute(json_t *input, void *arena, 
                          arena_alloc_func alloc_func,
                          arena_free_func free_func,
                          const char *config,
                          json_t *middleware_config,
                          char **contentType,
                          json_t *variables) {
    (void)free_func;    // Not used
    (void)contentType;  // Log middleware doesn't change content type
    (void)variables;    // Not used in basic implementation
    (void)middleware_config; // Global config not used for step-specific logic
    
    // Check if logging is globally disabled
    if (!global_config.enabled) {
        return input; // Continue pipeline
    }
    
    // Parse step-specific configuration
    LogStepConfig step_config = parse_step_config(config, arena, alloc_func);
    
    // Check if logging is disabled for this step
    if (!step_config.enabled) {
        return input; // Continue pipeline
    }
    
    // Store log metadata in the request object for post_execute
    json_t *log_metadata = json_object();
    json_object_set_new(log_metadata, "start_time", json_real(get_current_time_ms()));
    json_object_set_new(log_metadata, "level", json_string(log_level_to_string(step_config.level)));
    json_object_set_new(log_metadata, "include_body", json_boolean(step_config.include_body));
    json_object_set_new(log_metadata, "include_headers", json_boolean(step_config.include_headers));
    json_object_set_new(log_metadata, "enabled", json_boolean(true));
    
    // Add log metadata to the request object
    json_object_set_new(input, "_log_metadata", log_metadata);
    
    return input; // Continue pipeline
}

// Post-execute function (log response)
void middleware_post_execute(json_t *final_response, void *arena,
                           arena_alloc_func alloc_func,
                           json_t *middleware_config) {
    (void)arena;          // Not used in basic logging
    (void)alloc_func;     // Not used in basic logging
    (void)middleware_config; // Not needed with our metadata approach
    
    if (!global_config.enabled) {
        return;
    }
    
    // Look for log metadata in the final response
    json_t *log_metadata = json_object_get(final_response, "_log_metadata");
    if (!log_metadata) {
        // No log metadata, nothing to log
        return;
    }
    
    // Check if logging is enabled for this request
    json_t *log_enabled = json_object_get(log_metadata, "enabled");
    if (!log_enabled || !json_is_boolean(log_enabled) || !json_boolean_value(log_enabled)) {
        return;
    }
    
    // Extract metadata
    json_t *start_time_json = json_object_get(log_metadata, "start_time");
    json_t *level_json = json_object_get(log_metadata, "level");
    json_t *include_body_json = json_object_get(log_metadata, "include_body");
    json_t *include_headers_json = json_object_get(log_metadata, "include_headers");
    
    double start_time = 0.0;
    if (start_time_json && json_is_real(start_time_json)) {
        start_time = json_real_value(start_time_json);
    }
    
    // Calculate duration
    double current_time = get_current_time_ms();
    double duration_ms = current_time - start_time;
    
    // Build step config from metadata
    LogStepConfig step_config = {
        .level = LOG_LEVEL_INFO,
        .include_body = false,
        .include_headers = true,
        .enabled = true
    };
    
    if (level_json && json_is_string(level_json)) {
        step_config.level = parse_log_level(json_string_value(level_json));
    }
    if (include_body_json && json_is_boolean(include_body_json)) {
        step_config.include_body = json_boolean_value(include_body_json);
    }
    if (include_headers_json && json_is_boolean(include_headers_json)) {
        step_config.include_headers = json_boolean_value(include_headers_json);
    }
    
    // Get original request from response
    json_t *original_request = json_object_get(final_response, "originalRequest");
    
    // Log the request and response
    log_request_response(original_request, final_response, duration_ms, step_config);
    
    // Clean up - remove log metadata from response
    json_object_del(final_response, "_log_metadata");
}

// Optional cleanup function (called on shutdown)
void middleware_cleanup(void) {
    cleanup_global_config();
    printf("Log middleware cleaned up\n");
}
