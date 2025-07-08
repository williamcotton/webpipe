#include <jansson.h>
#include <libpq-fe.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

// Arena allocation function types for plugins
typedef void* (*arena_alloc_func)(void* arena, size_t size);
typedef void (*arena_free_func)(void* arena);

// Memory arena type (forward declaration)
typedef struct MemoryArena MemoryArena;

// Global PostgreSQL connection
static PGconn *pg_connection = NULL;

// Connection parameters (should be configurable)
static const char *pg_host = "localhost";
static const char *pg_port = "5432";
static const char *pg_dbname = "wp_db";
static const char *pg_user = "wp_user";
static const char *pg_password = "wp_pass";

// Initialize PostgreSQL connection
void pg_plugin_init() {
    if (!pg_connection) {
        char conninfo[512];
        snprintf(conninfo, sizeof(conninfo), 
                "host=%s port=%s dbname=%s user=%s password=%s",
                pg_host, pg_port, pg_dbname, pg_user, pg_password);
        
        pg_connection = PQconnectdb(conninfo);
        
        if (PQstatus(pg_connection) != CONNECTION_OK) {
            fprintf(stderr, "pg: Connection failed: %s\n", PQerrorMessage(pg_connection));
            PQfinish(pg_connection);
            pg_connection = NULL;
        } else {
            printf("pg: Connected to PostgreSQL database\n");
        }
    }
}

// Cleanup PostgreSQL connection
void pg_plugin_cleanup() {
    if (pg_connection) {
        PQfinish(pg_connection);
        pg_connection = NULL;
    }
}

// Convert PostgreSQL result to JSON
json_t *pg_result_to_json(PGresult *result) {
    if (!result) {
        return json_null();
    }
    
    ExecStatusType status = PQresultStatus(result);
    
    if (status != PGRES_TUPLES_OK && status != PGRES_COMMAND_OK) {
        fprintf(stderr, "pg: Query failed: %s\n", PQresultErrorMessage(result));
        return json_null();
    }
    
    json_t *response = json_object();
    
    if (status == PGRES_TUPLES_OK) {
        int rows = PQntuples(result);
        int cols = PQnfields(result);
        
        json_t *data_array = json_array();
        
        for (int row = 0; row < rows; row++) {
            json_t *row_obj = json_object();
            
            for (int col = 0; col < cols; col++) {
                const char *field_name = PQfname(result, col);
                const char *field_value = PQgetvalue(result, row, col);
                
                if (PQgetisnull(result, row, col)) {
                    json_object_set_new(row_obj, field_name, json_null());
                } else {
                    // Try to determine type based on PostgreSQL type OID
                    Oid field_type = PQftype(result, col);
                    
                    switch (field_type) {
                        case 16: // bool
                            json_object_set_new(row_obj, field_name, 
                                json_boolean(field_value[0] == 't'));
                            break;
                        case 20: // int8
                        case 21: // int2
                        case 23: // int4
                            json_object_set_new(row_obj, field_name, 
                                json_integer(atoll(field_value)));
                            break;
                        case 700: // float4
                        case 701: // float8
                        case 1700: // numeric
                            json_object_set_new(row_obj, field_name, 
                                json_real(atof(field_value)));
                            break;
                        default:
                            json_object_set_new(row_obj, field_name, 
                                json_string(field_value));
                            break;
                    }
                }
            }
            
            json_array_append_new(data_array, row_obj);
        }
        
        json_object_set_new(response, "rows", data_array);
        json_object_set_new(response, "rowCount", json_integer(rows));
    } else {
        // Command completed successfully
        char *affected = PQcmdTuples(result);
        json_object_set_new(response, "rowCount", json_integer(atoi(affected)));
    }
    
    return response;
}

// Execute SQL query with parameters
json_t *execute_sql(const char *sql, json_t *params) {
    if (!pg_connection) {
        pg_plugin_init();
        if (!pg_connection) {
            return json_null();
        }
    }
    
    PGresult *result = NULL;
    
    if (params && json_is_array(params)) {
        // Parameterized query
        int param_count = json_array_size(params);
        const char **param_values = malloc(sizeof(char*) * param_count);
        
        for (int i = 0; i < param_count; i++) {
            json_t *param = json_array_get(params, i);
            if (json_is_string(param)) {
                param_values[i] = json_string_value(param);
            } else if (json_is_integer(param)) {
                // Convert integer to string
                char *str = malloc(32);
                snprintf(str, 32, "%lld", json_integer_value(param));
                param_values[i] = str;
            } else if (json_is_real(param)) {
                // Convert real to string
                char *str = malloc(32);
                snprintf(str, 32, "%f", json_real_value(param));
                param_values[i] = str;
            } else if (json_is_null(param)) {
                param_values[i] = NULL;
            } else {
                param_values[i] = "NULL";
            }
        }
        
        result = PQexecParams(pg_connection, sql, param_count, NULL, 
                             param_values, NULL, NULL, 0);
        
        // Free allocated strings
        for (int i = 0; i < param_count; i++) {
            if (param_values[i] && 
                (json_is_integer(json_array_get(params, i)) || 
                 json_is_real(json_array_get(params, i)))) {
                free((char*)param_values[i]);
            }
        }
        free(param_values);
    } else {
        // Simple query
        result = PQexec(pg_connection, sql);
    }
    
    json_t *response = pg_result_to_json(result);
    PQclear(result);
    
    return response;
}

// Plugin execute function
json_t *plugin_execute(json_t *input, void *arena, arena_alloc_func alloc_func, arena_free_func free_func, const char *sql_query) {
    // Use the arena for any per-request allocations if needed (e.g., copying sql_query)
    size_t len = strlen(sql_query);
    char *arena_query = alloc_func(arena, len + 1);
    if (!arena_query) {
        json_t *result = json_object();
        json_object_set_new(result, "error", json_string("Arena OOM for SQL query"));
        return result;
    }
    memcpy(arena_query, sql_query, len);
    arena_query[len] = '\0';
    
    // Look for sqlParams in input
    json_t *sql_params = json_object_get(input, "sqlParams");
    
    // Execute query
    json_t *result = execute_sql(arena_query, sql_params);
    
    // Create response by copying input and adding data
    json_t *response = json_deep_copy(input);
    json_object_set_new(response, "data", result);
    
    // Free the arena_query allocated in this function
    free_func(arena);
    
    return response;
}

// Plugin cleanup function called when plugin is unloaded
__attribute__((destructor))
void plugin_destructor() {
    pg_plugin_cleanup();
}