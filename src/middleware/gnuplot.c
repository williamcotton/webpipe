#include <jansson.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/wait.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <time.h>
#include <stdint.h>

// Arena allocation function types
typedef void* (*arena_alloc_func)(void* arena, size_t size);
typedef void (*arena_free_func)(void* arena);

// Configuration structure
typedef struct {
    char *default_terminal;
    char *default_size;
    char *output_mode;
    int timeout_seconds;
    size_t max_output_size;
    char *temp_directory;
    char *gnuplot_binary;
    int enable_security_checks;
} gnuplot_config_t;

// Forward declaration for middleware function
json_t *middleware_execute(json_t *input, void *arena, 
                          arena_alloc_func alloc_func, 
                          arena_free_func free_func, 
                          const char *config,
                          json_t *middleware_config,
                          char **contentType,
                          json_t *variables);

// Execution result
typedef struct {
    char *output_data;
    size_t output_size;
    char *error_message;
    int exit_code;
    char *content_type;
} gnuplot_result_t;

// Constants
#define MAX_SCRIPT_SIZE 1048576  // 1MB
#define MAX_OUTPUT_SIZE 52428800 // 50MB

// Arena string duplication
static char *arena_strdup(void *arena, arena_alloc_func alloc_func, const char *str) {
    if (!str) return NULL;
    
    size_t len = strlen(str);
    char *copy = alloc_func(arena, len + 1);
    if (copy) {
        memcpy(copy, str, len);
        copy[len] = '\0';
    }
    return copy;
}

// Parse configuration
static gnuplot_config_t *parse_config(json_t *config, void *arena, arena_alloc_func alloc_func) {
    gnuplot_config_t *gp_config = alloc_func(arena, sizeof(gnuplot_config_t));
    if (!gp_config) return NULL;
    
    memset(gp_config, 0, sizeof(gnuplot_config_t));
    
    // Set defaults
    gp_config->default_terminal = arena_strdup(arena, alloc_func, "png");
    gp_config->default_size = arena_strdup(arena, alloc_func, "800,600");
    gp_config->output_mode = arena_strdup(arena, alloc_func, "base64");
    gp_config->timeout_seconds = 30;
    gp_config->max_output_size = MAX_OUTPUT_SIZE;
    gp_config->temp_directory = arena_strdup(arena, alloc_func, "/tmp");
    gp_config->gnuplot_binary = arena_strdup(arena, alloc_func, "gnuplot");
    gp_config->enable_security_checks = 1;
    
    if (!config) return gp_config;
    
    json_t *value;
    
    if ((value = json_object_get(config, "defaultTerminal")) && json_is_string(value)) {
        gp_config->default_terminal = arena_strdup(arena, alloc_func, json_string_value(value));
    }
    
    if ((value = json_object_get(config, "defaultSize")) && json_is_string(value)) {
        gp_config->default_size = arena_strdup(arena, alloc_func, json_string_value(value));
    }
    
    if ((value = json_object_get(config, "outputMode")) && json_is_string(value)) {
        gp_config->output_mode = arena_strdup(arena, alloc_func, json_string_value(value));
    }
    
    if ((value = json_object_get(config, "timeout")) && json_is_integer(value)) {
        gp_config->timeout_seconds = (int)json_integer_value(value);
        if (gp_config->timeout_seconds < 1) gp_config->timeout_seconds = 1;
        if (gp_config->timeout_seconds > 300) gp_config->timeout_seconds = 300;
    }
    
    if ((value = json_object_get(config, "maxOutputSize")) && json_is_integer(value)) {
        gp_config->max_output_size = (size_t)json_integer_value(value);
        if (gp_config->max_output_size > MAX_OUTPUT_SIZE) {
            gp_config->max_output_size = MAX_OUTPUT_SIZE;
        }
    }
    
    return gp_config;
}

// Security validation
static int validate_script(const char *script) {
    if (!script) return 0;
    
    size_t len = strlen(script);
    if (len > MAX_SCRIPT_SIZE) return 0;
    
    // Check for dangerous commands
    const char *forbidden[] = {
        "system ",
        "system(",
        "!",
        "load ",
        "save ",
        "cd ",
        "shell",
        NULL
    };
    
    for (int i = 0; forbidden[i]; i++) {
        if (strstr(script, forbidden[i])) {
            return 0;
        }
    }
    
    return 1;
}

// Format JSON data for gnuplot
static char *format_json_for_gnuplot(json_t *value, void *arena, arena_alloc_func alloc_func) {
    if (!value) return arena_strdup(arena, alloc_func, "");
    
    if (json_is_array(value)) {
        size_t array_size = json_array_size(value);
        if (array_size == 0) return arena_strdup(arena, alloc_func, "");
        
        size_t buffer_size = array_size * 100;
        char *buffer = alloc_func(arena, buffer_size);
        if (!buffer) return NULL;
        
        size_t pos = 0;
        
        for (size_t i = 0; i < array_size; i++) {
            json_t *element = json_array_get(value, i);
            
            if (json_is_array(element)) {
                size_t cols = json_array_size(element);
                for (size_t j = 0; j < cols; j++) {
                    json_t *col_val = json_array_get(element, j);
                    
                    if (pos >= buffer_size - 50) break;
                    
                    if (json_is_number(col_val)) {
                        int written = snprintf(buffer + pos, buffer_size - pos, 
                                      "%.6g", json_number_value(col_val));
                        if (written > 0) pos += (size_t)written;
                    } else if (json_is_string(col_val)) {
                        int written = snprintf(buffer + pos, buffer_size - pos,
                                      "\"%s\"", json_string_value(col_val));
                        if (written > 0) pos += (size_t)written;
                    } else {
                        int written = snprintf(buffer + pos, buffer_size - pos, "0");
                        if (written > 0) pos += (size_t)written;
                    }
                    
                    if (j < cols - 1 && pos < buffer_size - 1) {
                        buffer[pos++] = '\t';
                    }
                }
                if (pos < buffer_size - 1) {
                    buffer[pos++] = '\n';
                }
            } else if (json_is_number(element)) {
                if (pos < buffer_size - 20) {
                    int written = snprintf(buffer + pos, buffer_size - pos,
                                  "%.6g\n", json_number_value(element));
                    if (written > 0) pos += (size_t)written;
                }
            }
        }
        
        if (pos < buffer_size) {
            buffer[pos] = '\0';
        }
        return buffer;
    } else if (json_is_string(value)) {
        return arena_strdup(arena, alloc_func, json_string_value(value));
    } else if (json_is_number(value)) {
        char *buffer = alloc_func(arena, 32);
        if (buffer) {
            snprintf(buffer, 32, "%.6g", json_number_value(value));
        }
        return buffer;
    } else if (json_is_true(value)) {
        return arena_strdup(arena, alloc_func, "1");
    } else if (json_is_false(value)) {
        return arena_strdup(arena, alloc_func, "0");
    }
    
    return arena_strdup(arena, alloc_func, "");
}

// Process template variables
static char *process_template(const char *script, json_t *data, void *arena, arena_alloc_func alloc_func) {
    if (!script) return NULL;
    
    size_t script_len = strlen(script);
    size_t result_capacity = script_len * 2;
    char *result = alloc_func(arena, result_capacity);
    if (!result) return NULL;
    
    size_t result_len = 0;
    const char *pos = script;
    
    while (*pos && result_len < result_capacity - 1) {
        if (*pos == '{') {
            const char *end = strchr(pos + 1, '}');
            if (end) {
                size_t var_len = (size_t)(end - pos - 1);
                char *var_name = alloc_func(arena, var_len + 1);
                if (!var_name) break;
                
                strncpy(var_name, pos + 1, var_len);
                var_name[var_len] = '\0';
                
                json_t *value = json_object_get(data, var_name);
                char *substitution = format_json_for_gnuplot(value, arena, alloc_func);
                
                if (substitution) {
                    size_t sub_len = strlen(substitution);
                    if (result_len + sub_len < result_capacity - 1) {
                        memcpy(result + result_len, substitution, sub_len);
                        result_len += sub_len;
                    }
                }
                
                pos = end + 1;
            } else {
                result[result_len++] = *pos++;
            }
        } else {
            result[result_len++] = *pos++;
        }
    }
    
    result[result_len] = '\0';
    return result;
}

// Detect content type from script
static const char *detect_content_type(const char *script) {
    if (!script) return "image/png";
    
    const char *terminal_line = strstr(script, "set terminal");
    if (terminal_line) {
        if (strstr(terminal_line, "png")) return "image/png";
        if (strstr(terminal_line, "svg")) return "image/svg+xml";
        if (strstr(terminal_line, "eps")) return "application/postscript";
        if (strstr(terminal_line, "pdf")) return "application/pdf";
        if (strstr(terminal_line, "gif")) return "image/gif";
        if (strstr(terminal_line, "jpeg")) return "image/jpeg";
        if (strstr(terminal_line, "canvas")) return "text/html";
    }
    
    return "image/png";
}

// Base64 encoding
static char *base64_encode(const unsigned char *data, size_t input_length, void *arena, arena_alloc_func alloc_func) {
    static const char encoding_table[] = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    
    size_t output_length = 4 * ((input_length + 2) / 3);
    char *encoded_data = alloc_func(arena, output_length + 1);
    if (!encoded_data) return NULL;
    
    for (size_t i = 0, j = 0; i < input_length;) {
        uint32_t octet_a = i < input_length ? data[i++] : 0;
        uint32_t octet_b = i < input_length ? data[i++] : 0;
        uint32_t octet_c = i < input_length ? data[i++] : 0;
        
        uint32_t triple = (octet_a << 0x10) + (octet_b << 0x08) + octet_c;
        
        encoded_data[j++] = encoding_table[(triple >> 3 * 6) & 0x3F];
        encoded_data[j++] = encoding_table[(triple >> 2 * 6) & 0x3F];
        encoded_data[j++] = encoding_table[(triple >> 1 * 6) & 0x3F];
        encoded_data[j++] = encoding_table[(triple >> 0 * 6) & 0x3F];
    }
    
    static const int mod_table[] = {0, 2, 1};
    for (size_t i = 0; i < (size_t)mod_table[input_length % 3]; i++) {
        encoded_data[output_length - 1 - i] = '=';
    }
    
    encoded_data[output_length] = '\0';
    return encoded_data;
}

// Execute gnuplot script
static gnuplot_result_t *execute_gnuplot(const char *script, gnuplot_config_t *config, 
                                        void *arena, arena_alloc_func alloc_func) {
    gnuplot_result_t *result = alloc_func(arena, sizeof(gnuplot_result_t));
    if (!result) return NULL;
    
    memset(result, 0, sizeof(gnuplot_result_t));
    result->content_type = arena_strdup(arena, alloc_func, detect_content_type(script));
    
    // Create temporary files
    char temp_script[512];
    char temp_output[512];
    
    snprintf(temp_script, sizeof(temp_script), "%s/wp_gnuplot_%ld_%u.gp", 
             config->temp_directory, (long)time(NULL), arc4random() % 10000);
    snprintf(temp_output, sizeof(temp_output), "%s/wp_gnuplot_%ld_%u.out",
             config->temp_directory, (long)time(NULL), arc4random() % 10000);
    
    // Write script to file
    FILE *script_file = fopen(temp_script, "w");
    if (!script_file) {
        result->error_message = arena_strdup(arena, alloc_func, "Failed to create script file");
        return result;
    }
    
    // Modify script to output to temp file
    size_t script_len = strlen(script);
    char *modified_script = alloc_func(arena, script_len + 512);
    if (!modified_script) {
        fclose(script_file);
        unlink(temp_script);
        result->error_message = arena_strdup(arena, alloc_func, "Memory allocation failed");
        return result;
    }
    
    // Check if script already has "set output" command
    char *output_pos = strstr(script, "set output");
    if (output_pos) {
        // Find the end of the line containing "set output"
        char *line_end = strchr(output_pos, '\n');
        if (!line_end) line_end = output_pos + strlen(output_pos);
        
        // Copy script before "set output"
        size_t before_len = (size_t)(output_pos - script);
        memcpy(modified_script, script, before_len);
        
        // Add our set output command
        int written = snprintf(modified_script + before_len, 512, "set output '%s'\n", temp_output);
        
        // Copy script after the original "set output" line
        if (*line_end == '\n') line_end++; // Skip the newline
        size_t remaining_len = strlen(line_end);
        size_t available_space = script_len + 512 - before_len - (size_t)written;
        if (remaining_len < available_space) {
            memcpy(modified_script + before_len + written, line_end, remaining_len + 1);
        } else {
            // Truncate if necessary to prevent buffer overflow
            memcpy(modified_script + before_len + written, line_end, available_space - 1);
            modified_script[script_len + 512 - 1] = '\0';
        }
    } else {
        // No existing "set output", prepend ours
        snprintf(modified_script, script_len + 512, 
                 "set output '%s'\n%s\n", temp_output, script);
    }
    
    fwrite(modified_script, 1, strlen(modified_script), script_file);
    fclose(script_file);
    
    // Execute gnuplot with timeout
    char command[1024];
    snprintf(command, sizeof(command), "timeout %d %s '%s' 2>&1", 
             config->timeout_seconds, config->gnuplot_binary, temp_script);
    
    FILE *gnuplot_proc = popen(command, "r");
    if (!gnuplot_proc) {
        unlink(temp_script);
        result->error_message = arena_strdup(arena, alloc_func, "Failed to start gnuplot");
        return result;
    }
    
    // Read error output
    char error_buffer[4096] = {0};
    size_t error_len = fread(error_buffer, 1, sizeof(error_buffer) - 1, gnuplot_proc);
    result->exit_code = pclose(gnuplot_proc);
    
    if (result->exit_code != 0) {
        result->error_message = arena_strdup(arena, alloc_func, 
                                           error_len > 0 ? error_buffer : "Gnuplot execution failed");
        unlink(temp_script);
        unlink(temp_output);
        return result;
    }
    
    // Read output file
    FILE *output_file = fopen(temp_output, "rb");
    if (output_file) {
        fseek(output_file, 0, SEEK_END);
        long file_size = ftell(output_file);
        fseek(output_file, 0, SEEK_SET);
        
        if (file_size > 0 && (size_t)file_size <= config->max_output_size) {
            result->output_data = alloc_func(arena, (size_t)file_size);
            if (result->output_data) {
                result->output_size = fread(result->output_data, 1, (size_t)file_size, output_file);
            }
        } else if (file_size > (long)config->max_output_size) {
            result->error_message = arena_strdup(arena, alloc_func, "Output size exceeds limit");
        }
        
        fclose(output_file);
    }
    
    // Cleanup
    unlink(temp_script);
    unlink(temp_output);
    
    return result;
}

// Create error response
static json_t *create_error(const char *type, const char *message) {
    json_t *error_obj = json_object();
    json_t *errors_array = json_array();
    json_t *error_detail = json_object();
    
    json_object_set_new(error_detail, "type", json_string(type));
    json_object_set_new(error_detail, "message", json_string(message));
    
    json_array_append_new(errors_array, error_detail);
    json_object_set_new(error_obj, "errors", errors_array);
    
    return error_obj;
}

// Main middleware execute function
json_t *middleware_execute(json_t *input, void *arena, 
                          arena_alloc_func alloc_func, 
                          arena_free_func free_func, 
                          const char *gnuplot_script,
                          json_t *middleware_config,
                          char **contentType,
                          json_t *variables) {
    
    (void)free_func;
    (void)variables;
    
    if (!gnuplot_script || strlen(gnuplot_script) == 0) {
        return create_error("gnuplotError", "No gnuplot script provided");
    }
    
    // Parse configuration
    gnuplot_config_t *config = parse_config(middleware_config, arena, alloc_func);
    if (!config) {
        return create_error("gnuplotError", "Failed to parse configuration");
    }
    
    // Security validation
    if (config->enable_security_checks && !validate_script(gnuplot_script)) {
        return create_error("securityError", "Script contains forbidden commands");
    }
    
    // Process template variables
    char *processed_script = process_template(gnuplot_script, input, arena, alloc_func);
    if (!processed_script) {
        return create_error("templateError", "Failed to process template variables");
    }
    
    // Execute gnuplot
    gnuplot_result_t *result = execute_gnuplot(processed_script, config, arena, alloc_func);
    if (!result) {
        return create_error("gnuplotError", "Failed to execute gnuplot");
    }
    
    if (result->exit_code != 0 || result->error_message) {
        return create_error("executionError", 
                           result->error_message ? result->error_message : "Gnuplot execution failed");
    }
    
    if (!result->output_data || result->output_size == 0) {
        return create_error("outputError", "No output generated");
    }
    
    // Check for contentType override in input JSON
    json_t *input_content_type = json_object_get(input, "contentType");
    const char *desired_content_type = NULL;
    if (input_content_type && json_is_string(input_content_type)) {
        desired_content_type = json_string_value(input_content_type);
    }
    
    // Determine output mode - input contentType overrides config
    const char *effective_output_mode = config->output_mode;
    if (desired_content_type) {
        // Override output mode based on desired content type
        if (strstr(desired_content_type, "text/") == desired_content_type ||
            strcmp(desired_content_type, "image/svg+xml") == 0 ||
            strcmp(desired_content_type, "application/postscript") == 0) {
            effective_output_mode = "text";
        } else if (strstr(desired_content_type, "image/") == desired_content_type ||
                   strcmp(desired_content_type, "application/pdf") == 0) {
            effective_output_mode = "binary";
        }
    }
    
    // Format response based on effective output mode
    if (strcmp(effective_output_mode, "binary") == 0) {
        // Set content type and return binary data
        const char *content_type_to_set = desired_content_type ? desired_content_type : result->content_type;
        *contentType = arena_strdup(arena, alloc_func, content_type_to_set);
        return json_stringn(result->output_data, result->output_size);
    } else if (strcmp(effective_output_mode, "text") == 0) {
        // Return as text (for SVG, EPS, HTML canvas, etc.)
        const char *content_type_to_set = desired_content_type ? desired_content_type : result->content_type;
        *contentType = arena_strdup(arena, alloc_func, content_type_to_set);
        return json_string(result->output_data);
    } else {
        // Default: return base64-encoded data in JSON
        char *base64_data = base64_encode((unsigned char*)result->output_data, 
                                         result->output_size, arena, alloc_func);
        if (!base64_data) {
            return create_error("encodingError", "Failed to encode output data");
        }
        
        json_t *response = json_object();
        json_object_set_new(response, "data", json_string(base64_data));
        json_object_set_new(response, "contentType", json_string(result->content_type));
        json_object_set_new(response, "size", json_integer((json_int_t)result->output_size));
        
        return response;
    }
}
