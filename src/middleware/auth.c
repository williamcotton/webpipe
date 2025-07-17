#include "../../include/webpipe-middleware-api.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>
#include <fcntl.h>
#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wundef"
#pragma GCC diagnostic ignored "-Wdocumentation-unknown-command"
#include <argon2.h>
#pragma GCC diagnostic pop

// Global database API (initialized by core server)
WebpipeDatabaseAPI webpipe_db_api = {0};

// Configuration storage
static json_t *current_auth_config = NULL;

// SQL dialect enumeration
typedef enum {
    SQL_DIALECT_POSTGRESQL,
    SQL_DIALECT_MYSQL,
    SQL_DIALECT_SQLITE,
    SQL_DIALECT_UNKNOWN
} SqlDialect;

// SQL query templates for different dialects
typedef struct {
    const char *find_user_by_login;
    const char *create_user;
    const char *create_session;
    const char *find_session_by_token;
    const char *delete_session;
    const char *current_timestamp;
    const char *limit_clause;
} SqlQueries;

// Forward declarations
static SqlDialect detect_sql_dialect(void);
static SqlQueries get_sql_queries(SqlDialect dialect);
static json_t *find_user_by_login_db(const char *login, void *arena, arena_alloc_func alloc_func);
static json_t *create_user_db(const char *login, const char *email, const char *password_hash, void *arena, arena_alloc_func alloc_func);
static json_t *create_session_db(int user_id, const char *token, time_t expires_at, void *arena, arena_alloc_func alloc_func);
static json_t *find_session_by_token_db(const char *token, void *arena, arena_alloc_func alloc_func);
static json_t *delete_session_db(const char *token, void *arena, arena_alloc_func alloc_func);
static char *generate_session_id(void *arena, arena_alloc_func alloc_func);
static char *hash_password(const char *password, void *arena, arena_alloc_func alloc_func);
static int verify_password(const char *password, const char *hash);
static int generate_random_bytes(unsigned char *buffer, size_t length);
static char *create_session_cookie(const char *session_token, void *arena, arena_alloc_func alloc_func);
static char *create_clear_cookie(void *arena, arena_alloc_func alloc_func);
static const char *get_cookie_name(void);
static time_t get_session_ttl(void);

// Authentication handlers
static json_t *handle_login(json_t *input, void *arena, arena_alloc_func alloc_func, json_t *variables);
static json_t *handle_logout(json_t *input, void *arena, arena_alloc_func alloc_func, json_t *variables);
static json_t *handle_register(json_t *input, void *arena, arena_alloc_func alloc_func, json_t *variables);
static json_t *handle_required_auth(json_t *input, void *arena, arena_alloc_func alloc_func, json_t *variables);
static json_t *handle_optional_auth(json_t *input, void *arena, arena_alloc_func alloc_func, json_t *variables);
static json_t *handle_type_check(json_t *input, void *arena, arena_alloc_func alloc_func, json_t *variables, const char *required_type);

// Forward declarations to satisfy -Wmissing-prototypes
int middleware_init(json_t *config);
json_t *middleware_execute(json_t *input, void *arena, arena_alloc_func alloc_func, arena_free_func free_func, const char *config, json_t *middleware_config, char **contentType, json_t *variables);

// Public middleware initialization function called by server at startup
int middleware_init(json_t *config) {
    // Store the configuration for later use
    current_auth_config = config;
    
    if (!config) {
        fprintf(stderr, "auth: No configuration provided\n");
        return 1;
    }
    
    // Validate required configuration
    json_t *session_ttl = json_object_get(config, "sessionTtl");
    if (!session_ttl) {
        fprintf(stderr, "auth: Missing required 'sessionTtl' configuration\n");
        return 1;
    }
    
    printf("auth: Initialized successfully with %lld second session TTL\n", 
           json_integer_value(session_ttl));
    return 0;
}

// Main middleware function
json_t *middleware_execute(json_t *input, void *arena, 
                          arena_alloc_func alloc_func, 
                          arena_free_func free_func, 
                          const char *config,
                          json_t *middleware_config,
                          char **contentType,
                          json_t *variables) {
    // Suppress unused parameter warnings for parameters that are not yet used
    (void)free_func;
    (void)middleware_config;
    (void)contentType;
    (void)variables;
    
    if (!config) {
        return webpipe_create_auth_error("No auth configuration provided", "auth");
    }
    
    // Parse config to determine operation
    if (strcmp(config, "login") == 0) {
        return handle_login(input, arena, alloc_func, variables);
    } else if (strcmp(config, "logout") == 0) {
        return handle_logout(input, arena, alloc_func, variables);
    } else if (strcmp(config, "register") == 0) {
        return handle_register(input, arena, alloc_func, variables);
    } else if (strcmp(config, "required") == 0) {
        return handle_required_auth(input, arena, alloc_func, variables);
    } else if (strcmp(config, "optional") == 0) {
        return handle_optional_auth(input, arena, alloc_func, variables);
    } else if (strncmp(config, "type:", 5) == 0) {
        return handle_type_check(input, arena, alloc_func, variables, config + 5);
    }
    
    return webpipe_create_auth_error("Invalid auth configuration", config);
}

// SQL dialect detection
static SqlDialect detect_sql_dialect(void) {
    if (!webpipe_database_available()) {
        return SQL_DIALECT_UNKNOWN;
    }
    
    const char *provider_name = get_default_database_provider_name();
    if (!provider_name) {
        return SQL_DIALECT_UNKNOWN;
    }
    
    if (strcmp(provider_name, "pg") == 0 || strcmp(provider_name, "postgresql") == 0) {
        return SQL_DIALECT_POSTGRESQL;
    } else if (strcmp(provider_name, "mysql") == 0 || strcmp(provider_name, "mariadb") == 0) {
        return SQL_DIALECT_MYSQL;
    } else if (strcmp(provider_name, "sqlite") == 0) {
        return SQL_DIALECT_SQLITE;
    }
    
    return SQL_DIALECT_UNKNOWN;
}

// Get SQL queries for the current dialect
static SqlQueries get_sql_queries(SqlDialect dialect) {
    switch (dialect) {
        case SQL_DIALECT_POSTGRESQL:
            return (SqlQueries){
                .find_user_by_login = "SELECT id, login, password_hash, email, type, status FROM users WHERE login = $1 AND status = 'active'",
                .create_user = "INSERT INTO users (login, email, password_hash) VALUES ($1, $2, $3) RETURNING id, login, email",
                .create_session = "INSERT INTO sessions (user_id, token, expires_at) VALUES ($1, $2, to_timestamp($3)) RETURNING id",
                .find_session_by_token = "SELECT s.id, s.user_id, s.expires_at, u.login, u.email, u.type FROM sessions s JOIN users u ON s.user_id = u.id WHERE s.token = $1 AND s.expires_at > NOW()",
                .delete_session = "DELETE FROM sessions WHERE token = $1",
                .current_timestamp = "NOW()",
                .limit_clause = "LIMIT 1"
            };
            
        case SQL_DIALECT_MYSQL:
            return (SqlQueries){
                .find_user_by_login = "SELECT id, login, password_hash, email, type, status FROM users WHERE login = ? AND status = 'active'",
                .create_user = "INSERT INTO users (login, email, password_hash) VALUES (?, ?, ?)",
                .create_session = "INSERT INTO sessions (user_id, token, expires_at) VALUES (?, ?, ?)",
                .find_session_by_token = "SELECT s.id, s.user_id, s.expires_at, u.login, u.email, u.type FROM sessions s JOIN users u ON s.user_id = u.id WHERE s.token = ? AND s.expires_at > NOW()",
                .delete_session = "DELETE FROM sessions WHERE token = ?",
                .current_timestamp = "NOW()",
                .limit_clause = "LIMIT 1"
            };
            
        case SQL_DIALECT_SQLITE:
            return (SqlQueries){
                .find_user_by_login = "SELECT id, login, password_hash, email, type, status FROM users WHERE login = ? AND status = 'active'",
                .create_user = "INSERT INTO users (login, email, password_hash) VALUES (?, ?, ?)",
                .create_session = "INSERT INTO sessions (user_id, token, expires_at) VALUES (?, ?, ?)",
                .find_session_by_token = "SELECT s.id, s.user_id, s.expires_at, u.login, u.email, u.type FROM sessions s JOIN users u ON s.user_id = u.id WHERE s.token = ? AND s.expires_at > datetime('now')",
                .delete_session = "DELETE FROM sessions WHERE token = ?",
                .current_timestamp = "datetime('now')",
                .limit_clause = "LIMIT 1"
            };
            
        case SQL_DIALECT_UNKNOWN:
        default:
            // Default to PostgreSQL-style queries as fallback
            return get_sql_queries(SQL_DIALECT_POSTGRESQL);
    }
}

// Generate cryptographically secure session ID
static char *generate_session_id(void *arena, arena_alloc_func alloc_func) {
    unsigned char bytes[32];
    
    // Use secure random source
    if (generate_random_bytes(bytes, sizeof(bytes)) != 0) {
        return NULL;
    }
    
    // Convert to hex string (64 characters + null terminator)
    char *session_id = alloc_func(arena, 65);
    if (!session_id) {
        return NULL;
    }
    
    // Convert bytes to hex string
    for (int i = 0; i < 32; i++) {
        snprintf(session_id + (i * 2), 3, "%02x", bytes[i]);
    }
    session_id[64] = '\0';
    
    return session_id;
}

// Hash password using Argon2id
static char *hash_password(const char *password, void *arena, arena_alloc_func alloc_func) {
    // Argon2 parameters
    const uint32_t t_cost = 3;     // Number of iterations
    const uint32_t m_cost = 64 * 1024; // Memory usage in KiB (64 MB)
    const uint32_t parallelism = 1;     // Number of threads
    const uint32_t salt_len_const = 16; // Salt length in bytes (compile-time constant
    const uint32_t hash_len = 32;       // Hash length in bytes
    
    // Generate random salt
    uint8_t salt[16];
    if (generate_random_bytes(salt, salt_len_const) != 0) {
        return NULL;
    }
    
    // Allocate memory for encoded hash (includes salt and parameters)
    const size_t encoded_len = argon2_encodedlen(t_cost, m_cost, parallelism, salt_len_const, hash_len, Argon2_id);
    char *encoded_hash = alloc_func(arena, encoded_len);
    if (!encoded_hash) {
        return NULL;
    }
    
    // Hash the password
    int result = argon2id_hash_encoded(t_cost, m_cost, parallelism,
                                       password, strlen(password),
                                       salt, salt_len_const,
                                       hash_len, encoded_hash, encoded_len);
    
    if (result != ARGON2_OK) {
        return NULL;
    }
    
    return encoded_hash;
}

// Verify password against Argon2 hash
static int verify_password(const char *password, const char *hash) {
    if (!password || !hash) {
        return 0;
    }
    
    int result = argon2id_verify(hash, password, strlen(password));
    return (result == ARGON2_OK) ? 1 : 0;
}

// Generate cryptographically secure random bytes
static int generate_random_bytes(unsigned char *buffer, size_t length) {
    FILE *urandom = fopen("/dev/urandom", "rb");
    if (!urandom) {
        return -1;
    }
    
    size_t bytes_read = fread(buffer, 1, length, urandom);
    fclose(urandom);
    
    return (bytes_read == length) ? 0 : -1;
}

// Database operations using the database registry
static json_t *find_user_by_login_db(const char *login, void *arena, arena_alloc_func alloc_func) {
    WEBPIPE_REQUIRE_DATABASE(NULL);
    
    SqlDialect dialect = detect_sql_dialect();
    SqlQueries queries = get_sql_queries(dialect);
    
    json_t *params = json_array();
    json_array_append_new(params, json_string(login));
    
    return execute_sql(queries.find_user_by_login, params, arena, alloc_func);
}

static json_t *create_user_db(const char *login, const char *email, const char *password_hash, void *arena, arena_alloc_func alloc_func) {
    WEBPIPE_REQUIRE_DATABASE(NULL);
    
    SqlDialect dialect = detect_sql_dialect();
    SqlQueries queries = get_sql_queries(dialect);
    
    json_t *params = json_array();
    json_array_append_new(params, json_string(login));
    json_array_append_new(params, json_string(email));
    json_array_append_new(params, json_string(password_hash));
    
    json_t *result = execute_sql(queries.create_user, params, arena, alloc_func);
    
    // Handle MySQL case where we don't get RETURNING clause
    if (dialect == SQL_DIALECT_MYSQL && result && !webpipe_has_errors(result)) {
        // For MySQL, we need to get the last inserted ID separately
        json_t *last_id_params = json_array();
        json_t *last_id_result = execute_sql("SELECT LAST_INSERT_ID() as id", last_id_params, arena, alloc_func);
        
        if (last_id_result && !webpipe_has_errors(last_id_result)) {
            json_t *rows = json_object_get(last_id_result, "rows");
            if (rows && json_array_size(rows) > 0) {
                json_t *row = json_array_get(rows, 0);
                json_t *user_id = json_object_get(row, "id");
                
                // Create a result that matches standard format
                json_t *new_result = json_object();
                json_t *new_rows = json_array();
                json_t *new_row = json_object();
                    
                json_object_set(new_row, "id", user_id);
                json_object_set_new(new_row, "login", json_string(login));
                json_object_set_new(new_row, "email", json_string(email));
                json_array_append_new(new_rows, new_row);
                json_object_set_new(new_result, "rows", new_rows);
                
                return new_result;
            }
        }
    }

    return result;
}

static json_t *create_session_db(int user_id, const char *token, time_t expires_at, void *arena, arena_alloc_func alloc_func) {
    WEBPIPE_REQUIRE_DATABASE(NULL);
    
    SqlDialect dialect = detect_sql_dialect();
    SqlQueries queries = get_sql_queries(dialect);
    
    json_t *params = json_array();
    json_array_append_new(params, json_integer(user_id));
    json_array_append_new(params, json_string(token));
    json_array_append_new(params, json_integer(expires_at));
    
    json_t *result = execute_sql(queries.create_session, params, arena, alloc_func);
    
    // Handle MySQL case where we don't get RETURNING clause
    if (dialect == SQL_DIALECT_MYSQL && result && !webpipe_has_errors(result)) {
        // For MySQL, we need to get the last inserted ID separately
        json_t *last_id_params = json_array();
        json_t *last_id_result = execute_sql("SELECT LAST_INSERT_ID() as id", last_id_params, arena, alloc_func);
        
        if (last_id_result && !webpipe_has_errors(last_id_result)) {
            json_t *rows = json_object_get(last_id_result, "rows");
            if (rows && json_array_size(rows) > 0) {
                json_t *row = json_array_get(rows, 0);
                json_t *session_id = json_object_get(row, "id");
                
                // Create a result that matches standard format
                json_t *new_result = json_object();
                json_t *new_rows = json_array();
                json_t *new_row = json_object();
                    
                json_object_set(new_row, "id", session_id);
                json_array_append_new(new_rows, new_row);
                json_object_set_new(new_result, "rows", new_rows);
                
                return new_result;
            }
        }
    }    
    return result;
}

static json_t *find_session_by_token_db(const char *token, void *arena, arena_alloc_func alloc_func) {
    WEBPIPE_REQUIRE_DATABASE(NULL);
    
    SqlDialect dialect = detect_sql_dialect();
    SqlQueries queries = get_sql_queries(dialect);
    
    json_t *params = json_array();
    json_array_append_new(params, json_string(token));
    
    return execute_sql(queries.find_session_by_token, params, arena, alloc_func);
}

static json_t *delete_session_db(const char *token, void *arena, arena_alloc_func alloc_func) {
    WEBPIPE_REQUIRE_DATABASE(NULL);
    
    SqlDialect dialect = detect_sql_dialect();
    SqlQueries queries = get_sql_queries(dialect);
    
    json_t *params = json_array();
    json_array_append_new(params, json_string(token));
    
    return execute_sql(queries.delete_session, params, arena, alloc_func);
}

// Cookie management functions
static char *create_session_cookie(const char *session_token, void *arena, arena_alloc_func alloc_func) {
    const char *cookie_name = get_cookie_name();
    time_t ttl = get_session_ttl();
    
    // Get security settings from config
    json_t *secure_setting = json_object_get(current_auth_config, "cookieSecure");
    json_t *httponly_setting = json_object_get(current_auth_config, "cookieHttpOnly");
    json_t *samesite_setting = json_object_get(current_auth_config, "cookieSameSite");
    json_t *path_setting = json_object_get(current_auth_config, "cookiePath");
    
    bool is_secure = secure_setting ? json_boolean_value(secure_setting) : false;
    bool is_httponly = httponly_setting ? json_boolean_value(httponly_setting) : true;
    const char *samesite = samesite_setting ? json_string_value(samesite_setting) : "Lax";
    const char *path = path_setting ? json_string_value(path_setting) : "/";
    
    // Create cookie string with configurable security attributes
    size_t cookie_len = strlen(cookie_name) + strlen(session_token) + 150; // Extra space for attributes
    char *cookie = alloc_func(arena, cookie_len);
    if (!cookie) {
        return NULL;
    }
    
    snprintf(cookie, cookie_len, "%s=%s%s%s; SameSite=%s; Path=%s; Max-Age=%ld",
             cookie_name, session_token,
             is_httponly ? "; HttpOnly" : "",
             is_secure ? "; Secure" : "",
             samesite, path, ttl);
    
    return cookie;
}

static char *create_clear_cookie(void *arena, arena_alloc_func alloc_func) {
    const char *cookie_name = get_cookie_name();
    
    // Create cookie string to clear: name=; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=0
    size_t cookie_len = strlen(cookie_name) + 80; // Extra space for attributes
    char *cookie = alloc_func(arena, cookie_len);
    if (!cookie) {
        return NULL;
    }
    
    snprintf(cookie, cookie_len, "%s=; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age=0",
             cookie_name);
    
    return cookie;
}

static const char *get_cookie_name(void) {
    if (current_auth_config) {
        json_t *cookie_name = json_object_get(current_auth_config, "cookieName");
        if (cookie_name && json_is_string(cookie_name)) {
            return json_string_value(cookie_name);
        }
    }
    return "wp_session"; // Default cookie name
}

static time_t get_session_ttl(void) {
    if (current_auth_config) {
        json_t *session_ttl = json_object_get(current_auth_config, "sessionTtl");
        if (session_ttl && json_is_integer(session_ttl)) {
            return json_integer_value(session_ttl);
        }
    }
    return 604800; // Default 7 days
}

// Authentication handlers implementation
static json_t *handle_login(json_t *input, void *arena, arena_alloc_func alloc_func, json_t *variables) {
    (void)variables;
    // Extract login/username and password from input body
    json_t *body = json_object_get(input, "body");
    if (!body) {
        return webpipe_create_auth_error("Missing request body", "login");
    }
    
    json_t *login_json = json_object_get(body, "login");
    if (!login_json) {
        // Also try "username" for compatibility
        login_json = json_object_get(body, "username");
    }
    json_t *password_json = json_object_get(body, "password");
    
    if (!login_json || !password_json) {
        return webpipe_create_auth_error("Missing login or password", "login");
    }
    
    const char *login = json_string_value(login_json);
    const char *password = json_string_value(password_json);
    
    if (!login || !password) {
        return webpipe_create_auth_error("Invalid login or password format", "login");
    }
    
    // Find user by login
    json_t *user_result = find_user_by_login_db(login, arena, alloc_func);
    if (!user_result) {
        return webpipe_create_auth_error("Database query failed", "login");
    }
    
    // Check for errors in user lookup
    if (webpipe_has_errors(user_result)) {
        return user_result;
    }
    
    // Extract user data from result
    json_t *data = json_object_get(user_result, "data");
    json_t *rows;
    if (data) {
        // Old format with data.rows
        rows = json_object_get(data, "rows");
    } else {
        // New format with direct rows
        rows = json_object_get(user_result, "rows");
    }
    
    if (!rows) {
        return webpipe_create_auth_error("Invalid credentials", "login");
    }
    
    size_t row_count = json_array_size(rows);
    
    if (row_count == 0) {
        return webpipe_create_auth_error("Invalid credentials", "login");
    }
    
    json_t *user = json_array_get(rows, 0);
    
    // Check user status (active/inactive)
    json_t *status_json = json_object_get(user, "status");
    const char *status = json_string_value(status_json);
    if (!status || strcmp(status, "active") != 0) {
        return webpipe_create_auth_error("Account is not active", "login");
    }
    
    // Verify password
    json_t *password_hash_json = json_object_get(user, "password_hash");
    const char *password_hash = json_string_value(password_hash_json);
    
    if (!password_hash) {
        return webpipe_create_auth_error("Invalid credentials", "login");
    }
    
    int verify_result = verify_password(password, password_hash);
    
    if (!verify_result) {
        return webpipe_create_auth_error("Invalid credentials", "login");
    }
    
    // Create session
    char *session_token = generate_session_id(arena, alloc_func);
    if (!session_token) {
        return webpipe_create_auth_error("Session ID generation failed", "login");
    }
    
    json_t *user_id_json = json_object_get(user, "id");
    int user_id = (int)json_integer_value(user_id_json);
    time_t expires_at = time(NULL) + get_session_ttl();
    
    // Store session
    json_t *session_result = create_session_db(user_id, session_token, expires_at, arena, alloc_func);
    if (!session_result) {
        return webpipe_create_auth_error("Session storage failed", "login");
    }
    
    // Check for errors in session creation
    if (webpipe_has_errors(session_result)) {
        // Print the error details
        json_t *errors = json_object_get(session_result, "errors");
        if (errors && json_is_array(errors)) {
            size_t error_count = json_array_size(errors);
            for (size_t i = 0; i < error_count; i++) {
                json_t *error = json_array_get(errors, i);
                json_t *message = json_object_get(error, "message");
                if (message && json_is_string(message)) {
                }
            }
        }
        return session_result;
    }
    
    // Create response with user data and cookie
    json_t *result = json_deep_copy(input);
    json_t *user_obj = json_object();
    json_object_set_new(user_obj, "id", json_copy(json_object_get(user, "id")));
    json_object_set_new(user_obj, "login", json_copy(json_object_get(user, "login")));
    json_object_set_new(user_obj, "email", json_copy(json_object_get(user, "email")));
    json_object_set_new(user_obj, "type", json_copy(json_object_get(user, "type")));
    json_object_set_new(user_obj, "status", json_copy(json_object_get(user, "status")));
    json_object_set_new(result, "user", user_obj);
    
    // Set cookie
    char *cookie_value = create_session_cookie(session_token, arena, alloc_func);
    if (cookie_value) {
        json_t *set_cookies = json_object_get(result, "setCookies");
        if (!set_cookies) {
            set_cookies = json_array();
            json_object_set_new(result, "setCookies", set_cookies);
        }
        json_array_append_new(set_cookies, json_string(cookie_value));
    }
    
    return result;
}

static json_t *handle_logout(json_t *input, void *arena, arena_alloc_func alloc_func, json_t *variables) {
    (void)variables;
    // Get session token from cookies
    json_t *cookies = json_object_get(input, "cookies");
    if (!cookies) {
        return webpipe_create_auth_error("No cookies found", "logout");
    }
    
    json_t *session_cookie = json_object_get(cookies, get_cookie_name());
    if (!session_cookie) {
        return webpipe_create_auth_error("No session cookie found", "logout");
    }
    
    const char *session_token = json_string_value(session_cookie);
    if (!session_token) {
        return webpipe_create_auth_error("Invalid session cookie", "logout");
    }
    
    // Delete session
    json_t *delete_result = delete_session_db(session_token, arena, alloc_func);
    if (!delete_result) {
        return webpipe_create_auth_error("Session deletion failed", "logout");
    }
    
    // Check for errors in session deletion (optional - logout can continue even if deletion fails)
    if (webpipe_has_errors(delete_result)) {
        // Log error but don't fail logout
        fprintf(stderr, "Warning: Session deletion failed during logout\n");
    }
    
    // Create response with clear cookie (modify input in place)
    json_t *result = input;
    char *clear_cookie = create_clear_cookie(arena, alloc_func);
    if (clear_cookie) {
        json_t *set_cookies = json_object_get(result, "setCookies");
        if (!set_cookies) {
            set_cookies = json_array();
            json_object_set_new(result, "setCookies", set_cookies);
        }
        json_array_append_new(set_cookies, json_string(clear_cookie));
    }
    
    return result;
}

static json_t *handle_register(json_t *input, void *arena, arena_alloc_func alloc_func, json_t *variables) {
    (void)variables;
    // Extract user data from input body
    json_t *body = json_object_get(input, "body");
    if (!body) {
        return webpipe_create_auth_error("Missing request body", "register");
    }
    
    json_t *login_json = json_object_get(body, "login");
    json_t *email_json = json_object_get(body, "email");
    json_t *password_json = json_object_get(body, "password");
    
    if (!login_json || !email_json || !password_json) {
        return webpipe_create_auth_error("Missing required fields: login, email, password", "register");
    }
    
    const char *login = json_string_value(login_json);
    const char *email = json_string_value(email_json);
    const char *password = json_string_value(password_json);
    
    if (!login || !email || !password) {
        return webpipe_create_auth_error("Invalid field formats", "register");
    }
    
    // Check if user already exists
    json_t *existing_user = find_user_by_login_db(login, arena, alloc_func);
    if (existing_user && !webpipe_has_errors(existing_user)) {
        json_t *rows = json_object_get(existing_user, "rows");
        if (rows && json_array_size(rows) > 0) {
            return webpipe_create_auth_error("User already exists", "register");
        }
    }
    
    // Hash password
    char *password_hash = hash_password(password, arena, alloc_func);
    if (!password_hash) {
        return webpipe_create_auth_error("Password hashing failed", "register");
    }
    
    // Create user in database
    json_t *create_result = create_user_db(login, email, password_hash, arena, alloc_func);
    if (!create_result) {
        return webpipe_create_auth_error("User creation failed", "register");
    }
    
    // Check for errors in user creation
    if (webpipe_has_errors(create_result)) {
        return create_result;
    }
    
    // Extract user data from result
    json_t *rows = json_object_get(create_result, "rows");
    if (!rows) {
        return webpipe_create_auth_error("User creation failed - no rows object in result", "register");
    }
    
    if (json_array_size(rows) == 0) {
        return webpipe_create_auth_error("User creation failed - empty rows array", "register");
    }
    
    json_t *created_user = json_array_get(rows, 0);
    
    // Create response with user data (modify input in place)
    json_t *result = input;
    json_t *user_obj = json_object();
    json_object_set_new(user_obj, "id", json_copy(json_object_get(created_user, "id")));
    json_object_set_new(user_obj, "login", json_copy(json_object_get(created_user, "login")));
    json_object_set_new(user_obj, "email", json_copy(json_object_get(created_user, "email")));
    json_object_set_new(user_obj, "type", json_string("local"));
    json_object_set_new(user_obj, "status", json_string("active"));
    json_object_set_new(result, "user", user_obj);
    json_object_set_new(result, "message", json_string("User registration successful"));
    
    return result;
}

static json_t *handle_required_auth(json_t *input, void *arena, arena_alloc_func alloc_func, json_t *variables) {
    (void)variables;
    // Get session token from cookies
    json_t *cookies = json_object_get(input, "cookies");
    if (!cookies) {
        return webpipe_create_auth_error("Authentication required", "required");
    }
    
    json_t *session_cookie = json_object_get(cookies, get_cookie_name());
    if (!session_cookie) {
        return webpipe_create_auth_error("Authentication required", "required");
    }
    
    const char *session_token = json_string_value(session_cookie);
    if (!session_token) {
        return webpipe_create_auth_error("Authentication required", "required");
    }
    
    // Get session by token
    json_t *session_result = find_session_by_token_db(session_token, arena, alloc_func);
    if (!session_result) {
        return webpipe_create_auth_error("Session lookup failed", "required");
    }
    
    // Check for errors in session lookup
    if (webpipe_has_errors(session_result)) {
        return webpipe_create_auth_error("Invalid session", "required");
    }
    
    // Extract session data from result
    json_t *rows = json_object_get(session_result, "rows");
    if (!rows || json_array_size(rows) == 0) {
        return webpipe_create_auth_error("Invalid session", "required");
    }
    
    json_t *session_data = json_array_get(rows, 0);
    
    // Add user to request (modify input in place)
    json_t *result = input;
    json_t *user_obj = json_object();
    json_object_set_new(user_obj, "id", json_copy(json_object_get(session_data, "user_id")));
    json_object_set_new(user_obj, "login", json_copy(json_object_get(session_data, "login")));
    json_object_set_new(user_obj, "email", json_copy(json_object_get(session_data, "email")));
    json_object_set_new(user_obj, "type", json_copy(json_object_get(session_data, "type")));
    json_object_set_new(result, "user", user_obj);
    
    return result;
}

static json_t *handle_optional_auth(json_t *input, void *arena, arena_alloc_func alloc_func, json_t *variables) {
    (void)variables;
    // Try to get session token from cookies
    json_t *cookies = json_object_get(input, "cookies");
    if (!cookies) {
        // No cookies, return input as-is
        return input;
    }
    
    json_t *session_cookie = json_object_get(cookies, get_cookie_name());
    if (!session_cookie) {
        // No session cookie, return input as-is
        return input;
    }
    
    const char *session_token = json_string_value(session_cookie);
    if (!session_token) {
        // Invalid session cookie, return input as-is
        return input;
    }
    
    // Try to get session by token
    json_t *session_result = find_session_by_token_db(session_token, arena, alloc_func);
    if (!session_result || webpipe_has_errors(session_result)) {
        // Session lookup failed, return input as-is
        return input;
    }
    
    // Extract session data from result
    json_t *rows = json_object_get(session_result, "rows");
    if (!rows || json_array_size(rows) == 0) {
        // No valid session, return input as-is
        return input;
    }
    
    json_t *session_data = json_array_get(rows, 0);
    
    // Add user to request (modify input in place)
    json_t *result = input;
    json_t *user_obj = json_object();
    json_object_set_new(user_obj, "id", json_copy(json_object_get(session_data, "user_id")));
    json_object_set_new(user_obj, "login", json_copy(json_object_get(session_data, "login")));
    json_object_set_new(user_obj, "email", json_copy(json_object_get(session_data, "email")));
    json_object_set_new(user_obj, "type", json_copy(json_object_get(session_data, "type")));
    json_object_set_new(result, "user", user_obj);
    
    return result;
}

static json_t *handle_type_check(json_t *input, void *arena, arena_alloc_func alloc_func, json_t *variables, const char *required_type) {
    // First ensure authentication
    json_t *auth_result = handle_required_auth(input, arena, alloc_func, variables);
    if (webpipe_has_errors(auth_result)) {
        return auth_result;
    }
    
    // Check user type
    json_t *user = json_object_get(auth_result, "user");
    if (!user) {
        return webpipe_create_auth_error("User information not found", "type_check");
    }
    
    json_t *user_type_json = json_object_get(user, "type");
    const char *user_type = json_string_value(user_type_json);
    
    if (!user_type || strcmp(user_type, required_type) != 0) {
        return webpipe_create_auth_error("Insufficient permissions", "type_check");
    }
    
    return auth_result;
}
