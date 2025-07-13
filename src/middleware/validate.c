#include <jansson.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdbool.h>
#include <ctype.h>
#include <regex.h>

// Arena allocation function types for middleware
typedef void* (*arena_alloc_func)(void* arena, size_t size);
typedef void (*arena_free_func)(void* arena);

// Memory arena type (forward declaration)
typedef struct MemoryArena MemoryArena;

// Validation rule types
typedef enum {
    RULE_STRING,
    RULE_NUMBER,
    RULE_EMAIL,
    RULE_BOOLEAN
} ValidationRuleType;

// Validation constraint structure
typedef struct {
    bool has_min;
    bool has_max;
    int min_value;
    int max_value;
} ValidationConstraints;

// Validation rule structure
typedef struct ValidationRule {
    char *field_name;
    ValidationRuleType type;
    bool is_optional;
    ValidationConstraints constraints;
    struct ValidationRule *next;
} ValidationRule;

// Function prototypes
static ValidationRule *parse_validation_dsl(const char *dsl, void *arena, arena_alloc_func alloc_func);
static bool validate_field(json_t *value, ValidationRule *rule, json_t *errors_array, void *arena, arena_alloc_func alloc_func);
static bool validate_string(json_t *value, ValidationRule *rule, json_t *errors_array, void *arena, arena_alloc_func alloc_func);
static bool validate_number(json_t *value, ValidationRule *rule, json_t *errors_array, void *arena, arena_alloc_func alloc_func);
static bool validate_email(json_t *value, ValidationRule *rule, json_t *errors_array, void *arena, arena_alloc_func alloc_func);
static bool validate_boolean(json_t *value, ValidationRule *rule, json_t *errors_array, void *arena, arena_alloc_func alloc_func);
static void add_validation_error(json_t *errors_array, const char *field, const char *rule, const char *message, void *arena, arena_alloc_func alloc_func);
static char *arena_strdup_safe(void *arena, arena_alloc_func alloc_func, const char *str);
static bool is_valid_email(const char *email);
json_t *middleware_execute(json_t *input, void *arena, arena_alloc_func alloc_func, arena_free_func free_func, const char *config, char **contentType, json_t *variables);

// DSL Parser Implementation
static ValidationRule *parse_validation_dsl(const char *dsl, void *arena, arena_alloc_func alloc_func) {
    if (!dsl || !arena || !alloc_func) {
        return NULL;
    }
    
    ValidationRule *rules_head = NULL;
    ValidationRule *rules_tail = NULL;
    
    // Create a copy of the DSL string for parsing
    size_t dsl_len = strlen(dsl);
    char *dsl_copy = alloc_func(arena, dsl_len + 1);
    if (!dsl_copy) {
        return NULL;
    }
    memcpy(dsl_copy, dsl, dsl_len);
    dsl_copy[dsl_len] = '\0';
    
    char *saveptr;
    char *line = strtok_r(dsl_copy, "\n", &saveptr);
    
    while (line) {
        // Skip empty lines and whitespace
        while (*line && isspace(*line)) line++;
        if (!*line) {
            line = strtok_r(NULL, "\n", &saveptr);
            continue;
        }
        
        // Find the colon separator
        char *colon = strchr(line, ':');
        if (!colon) {
            line = strtok_r(NULL, "\n", &saveptr);
            continue;
        }
        
        // Extract field name
        *colon = '\0';
        char *field_start = line;
        char *field_end = colon - 1;
        
        // Trim whitespace from field name
        while (*field_start && isspace(*field_start)) field_start++;
        while (field_end > field_start && isspace(*field_end)) field_end--;
        *(field_end + 1) = '\0';
        
        // Check if field is optional
        bool is_optional = false;
        if (field_end > field_start && *field_end == '?') {
            is_optional = true;
            *field_end = '\0';
            field_end--;
            while (field_end > field_start && isspace(*field_end)) field_end--;
            *(field_end + 1) = '\0';
        }
        
        // Extract type and constraints
        char *type_start = colon + 1;
        while (*type_start && isspace(*type_start)) type_start++;
        
        // Create validation rule
        ValidationRule *rule = alloc_func(arena, sizeof(ValidationRule));
        if (!rule) {
            return rules_head;
        }
        
        rule->field_name = arena_strdup_safe(arena, alloc_func, field_start);
        rule->is_optional = is_optional;
        rule->next = NULL;
        rule->constraints.has_min = false;
        rule->constraints.has_max = false;
        rule->constraints.min_value = 0;
        rule->constraints.max_value = 0;
        
        // Parse type and constraints
        if (strncmp(type_start, "string", 6) == 0) {
            rule->type = RULE_STRING;
            
            // Check for constraints like string(10..100)
            char *paren = strchr(type_start, '(');
            if (paren) {
                char *close_paren = strchr(paren, ')');
                if (close_paren) {
                    *close_paren = '\0';
                    char *constraint_str = paren + 1;
                    
                    char *dot_dot = strstr(constraint_str, "..");
                    if (dot_dot) {
                        *dot_dot = '\0';
                        rule->constraints.has_min = true;
                        rule->constraints.min_value = atoi(constraint_str);
                        rule->constraints.has_max = true;
                        rule->constraints.max_value = atoi(dot_dot + 2);
                    } else {
                        // Single value constraint (exact length)
                        rule->constraints.has_min = true;
                        rule->constraints.has_max = true;
                        rule->constraints.min_value = atoi(constraint_str);
                        rule->constraints.max_value = atoi(constraint_str);
                    }
                }
            }
        } else if (strncmp(type_start, "number", 6) == 0) {
            rule->type = RULE_NUMBER;
            
            // Check for constraints like number(18..120)
            char *paren = strchr(type_start, '(');
            if (paren) {
                char *close_paren = strchr(paren, ')');
                if (close_paren) {
                    *close_paren = '\0';
                    char *constraint_str = paren + 1;
                    
                    char *dot_dot = strstr(constraint_str, "..");
                    if (dot_dot) {
                        *dot_dot = '\0';
                        rule->constraints.has_min = true;
                        rule->constraints.min_value = atoi(constraint_str);
                        rule->constraints.has_max = true;
                        rule->constraints.max_value = atoi(dot_dot + 2);
                    }
                }
            }
        } else if (strncmp(type_start, "email", 5) == 0) {
            rule->type = RULE_EMAIL;
        } else if (strncmp(type_start, "boolean", 7) == 0) {
            rule->type = RULE_BOOLEAN;
        } else {
            // Unknown type, default to string
            rule->type = RULE_STRING;
        }
        
        // Add rule to list
        if (!rules_head) {
            rules_head = rule;
            rules_tail = rule;
        } else {
            rules_tail->next = rule;
            rules_tail = rule;
        }
        
        line = strtok_r(NULL, "\n", &saveptr);
    }
    
    return rules_head;
}

// Validation Functions
static bool validate_field(json_t *value, ValidationRule *rule, json_t *errors_array, void *arena, arena_alloc_func alloc_func) {
    if (!rule) return true;
    
    // Handle missing field
    if (!value || json_is_null(value)) {
        if (!rule->is_optional) {
            add_validation_error(errors_array, rule->field_name, "required", "Field is required", arena, alloc_func);
            return false;
        }
        return true; // Optional field can be missing
    }
    
    switch (rule->type) {
        case RULE_STRING:
            return validate_string(value, rule, errors_array, arena, alloc_func);
        case RULE_NUMBER:
            return validate_number(value, rule, errors_array, arena, alloc_func);
        case RULE_EMAIL:
            return validate_email(value, rule, errors_array, arena, alloc_func);
        case RULE_BOOLEAN:
            return validate_boolean(value, rule, errors_array, arena, alloc_func);
        default:
            return true;
    }
}

static bool validate_string(json_t *value, ValidationRule *rule, json_t *errors_array, void *arena, arena_alloc_func alloc_func) {
    if (!json_is_string(value)) {
        add_validation_error(errors_array, rule->field_name, "type", "Field must be a string", arena, alloc_func);
        return false;
    }
    
    const char *str_value = json_string_value(value);
    size_t str_len = strlen(str_value);
    
    bool valid = true;
    
    if (rule->constraints.has_min && (int)str_len < rule->constraints.min_value) {
        char message[256];
        snprintf(message, sizeof(message), "String must be at least %d characters long", rule->constraints.min_value);
        add_validation_error(errors_array, rule->field_name, "minLength", message, arena, alloc_func);
        valid = false;
    }
    
    if (rule->constraints.has_max && (int)str_len > rule->constraints.max_value) {
        char message[256];
        snprintf(message, sizeof(message), "String must be at most %d characters long", rule->constraints.max_value);
        add_validation_error(errors_array, rule->field_name, "maxLength", message, arena, alloc_func);
        valid = false;
    }
    
    return valid;
}

static bool validate_number(json_t *value, ValidationRule *rule, json_t *errors_array, void *arena, arena_alloc_func alloc_func) {
    if (!json_is_number(value)) {
        add_validation_error(errors_array, rule->field_name, "type", "Field must be a number", arena, alloc_func);
        return false;
    }
    
    double num_value = json_number_value(value);
    bool valid = true;
    
    if (rule->constraints.has_min && num_value < rule->constraints.min_value) {
        char message[256];
        snprintf(message, sizeof(message), "Number must be at least %d", rule->constraints.min_value);
        add_validation_error(errors_array, rule->field_name, "minimum", message, arena, alloc_func);
        valid = false;
    }
    
    if (rule->constraints.has_max && num_value > rule->constraints.max_value) {
        char message[256];
        snprintf(message, sizeof(message), "Number must be at most %d", rule->constraints.max_value);
        add_validation_error(errors_array, rule->field_name, "maximum", message, arena, alloc_func);
        valid = false;
    }
    
    return valid;
}

static bool validate_email(json_t *value, ValidationRule *rule, json_t *errors_array, void *arena, arena_alloc_func alloc_func) {
    if (!json_is_string(value)) {
        add_validation_error(errors_array, rule->field_name, "type", "Email must be a string", arena, alloc_func);
        return false;
    }
    
    const char *email_value = json_string_value(value);
    
    if (!is_valid_email(email_value)) {
        add_validation_error(errors_array, rule->field_name, "format", "Invalid email format", arena, alloc_func);
        return false;
    }
    
    return true;
}

static bool validate_boolean(json_t *value, ValidationRule *rule, json_t *errors_array, void *arena, arena_alloc_func alloc_func) {
    if (!json_is_boolean(value)) {
        add_validation_error(errors_array, rule->field_name, "type", "Field must be a boolean", arena, alloc_func);
        return false;
    }
    
    return true;
}

// Email validation helper (basic implementation)
static bool is_valid_email(const char *email) {
    if (!email) return false;
    
    // Basic email validation - must contain @ and a dot after @
    const char *at_sign = strchr(email, '@');
    if (!at_sign || at_sign == email) return false; // No @ or @ at start
    
    const char *dot = strchr(at_sign, '.');
    if (!dot || dot == at_sign + 1) return false; // No . after @ or . immediately after @
    
    // Check for valid characters (basic check)
    size_t len = strlen(email);
    if (len < 5 || len > 254) return false; // RFC 5321 limit
    
    // Must not end with dot
    if (email[len - 1] == '.') return false;
    
    return true;
}

// Error handling helpers
static void add_validation_error(json_t *errors_array, const char *field, const char *rule, const char *message, void *arena __attribute__((unused)), arena_alloc_func alloc_func __attribute__((unused))) {
    json_t *error = json_object();
    
    json_object_set_new(error, "type", json_string("validationError"));
    json_object_set_new(error, "field", json_string(field));
    json_object_set_new(error, "rule", json_string(rule));
    json_object_set_new(error, "message", json_string(message));
    
    json_array_append_new(errors_array, error);
}

static char *arena_strdup_safe(void *arena, arena_alloc_func alloc_func, const char *str) {
    if (!str || !arena || !alloc_func) return NULL;
    
    size_t len = strlen(str);
    char *copy = alloc_func(arena, len + 1);
    if (copy) {
        memcpy(copy, str, len);
        copy[len] = '\0';
    }
    return copy;
}

// Main middleware function
json_t *middleware_execute(json_t *input, void *arena, arena_alloc_func alloc_func, arena_free_func free_func, const char *config, char **contentType, json_t *variables) {
    (void)free_func; // Suppress unused parameter warning
    (void)contentType; // Validate middleware produces JSON output
    (void)variables; // Unused parameter
    
    // Handle null or empty config
    if (!config || strlen(config) == 0) {
        // No validation rules, return input as-is
        return input ? json_incref(input) : json_null();
    }
    
    // Handle null input
    if (!input) {
        json_t *error_result = json_object();
        json_t *errors_array = json_array();
        add_validation_error(errors_array, "input", "required", "Input is required", arena, alloc_func);
        json_object_set_new(error_result, "errors", errors_array);
        return error_result;
    }
    
    // Parse validation rules from DSL
    ValidationRule *rules = parse_validation_dsl(config, arena, alloc_func);
    if (!rules) {
        json_t *error_result = json_object();
        json_t *errors_array = json_array();
        add_validation_error(errors_array, "config", "parse", "Failed to parse validation rules", arena, alloc_func);
        json_object_set_new(error_result, "errors", errors_array);
        return error_result;
    }
    
    // Get the body object from the request
    json_t *body = json_object_get(input, "body");
    if (!body || json_is_null(body)) {
        // Check if any required fields exist
        ValidationRule *rule = rules;
        bool has_required = false;
        while (rule) {
            if (!rule->is_optional) {
                has_required = true;
                break;
            }
            rule = rule->next;
        }
        
        if (has_required) {
            json_t *error_result = json_object();
            json_t *errors_array = json_array();
            add_validation_error(errors_array, "body", "required", "Request body is required", arena, alloc_func);
            json_object_set_new(error_result, "errors", errors_array);
            return error_result;
        }
        
        // No required fields, return input as-is
        return json_incref(input);
    }
    
    // Validate each field according to rules
    json_t *errors_array = json_array();
    bool has_errors = false;
    
    ValidationRule *rule = rules;
    while (rule) {
        json_t *field_value = json_object_get(body, rule->field_name);
        if (!validate_field(field_value, rule, errors_array, arena, alloc_func)) {
            has_errors = true;
        }
        rule = rule->next;
    }
    
    if (has_errors) {
        json_t *error_result = json_object();
        json_object_set_new(error_result, "errors", errors_array);
        return error_result;
    }
    
    // Validation passed, clean up errors array and return input
    json_decref(errors_array);
    return json_incref(input);
}
