#include <jansson.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>
#include <ctype.h>
#include <stdbool.h>

// Arena allocation function types
typedef void* (*arena_alloc_func)(void* arena, size_t size);
typedef void (*arena_free_func)(void* arena);

// Forward declaration for middleware function
json_t *middleware_execute(json_t *input, void *arena, 
                          arena_alloc_func alloc_func, 
                          arena_free_func free_func, 
                          const char *config,
                          json_t *middleware_config,
                          char **contentType,
                          json_t *variables);

// Token types for DSL parsing
typedef enum {
    PLOT_TOKEN_EOF,
    PLOT_TOKEN_PLUS,
    PLOT_TOKEN_LPAREN,
    PLOT_TOKEN_RPAREN,
    PLOT_TOKEN_COMMA,
    PLOT_TOKEN_EQUALS,
    PLOT_TOKEN_DOT,
    PLOT_TOKEN_IDENTIFIER,
    PLOT_TOKEN_STRING,
    PLOT_TOKEN_NUMBER,
    PLOT_TOKEN_DATA,
    PLOT_TOKEN_AES,
    PLOT_TOKEN_GEOM_POINT,
    PLOT_TOKEN_GEOM_LINE,
    PLOT_TOKEN_GEOM_BAR,
    PLOT_TOKEN_GEOM_AREA,
    PLOT_TOKEN_LABS,
    PLOT_TOKEN_THEME
} PlotTokenType;

typedef struct {
    PlotTokenType type;
    char *value;
    size_t start;
    size_t length;
} PlotToken;

typedef struct {
    PlotToken *tokens;
    size_t count;
    size_t capacity;
    size_t current;
    void *arena;
    arena_alloc_func alloc_func;
} PlotLexer;

// Plot data structures
typedef struct {
    double *x_values;
    double *y_values;
    char **categories;
    double *colors;       // Numeric color mapping
    double *sizes;        // Size mapping
    size_t count;
    size_t capacity;
    char *x_field;
    char *y_field;
    char *color_field;
    char *size_field;
} PlotData;

// Aesthetics mapping structure
typedef struct {
    char *x_field;
    char *y_field;
    char *color_field;
    char *size_field;
    char *shape_field;
    char *alpha_field;
} PlotAesthetics;

typedef enum {
    GEOM_POINT,
    GEOM_LINE,
    GEOM_BAR,
    GEOM_AREA
} GeomType;

typedef struct PlotLayer {
    GeomType type;
    PlotData *data;
    PlotAesthetics *aes;  // Aesthetic mappings for this layer
    json_t *params;       // Fixed parameters (color, size, etc.)
    struct PlotLayer *next;
} PlotLayer;

typedef struct {
    PlotData *data;
    PlotAesthetics *global_aes;  // Global aesthetic mappings
    PlotLayer *layers;
    char *title;
    char *x_label;
    char *y_label;
    double width;
    double height;
    double margin_top;
    double margin_right;
    double margin_bottom;
    double margin_left;
} PlotSpec;

// SVG Builder
typedef struct {
    char *buffer;
    size_t size;
    size_t capacity;
    void *arena;
    arena_alloc_func alloc_func;
} SVGBuilder;

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

// Create aesthetics object
static PlotAesthetics *create_aesthetics(void *arena, arena_alloc_func alloc_func) {
    PlotAesthetics *aes = alloc_func(arena, sizeof(PlotAesthetics));
    if (!aes) return NULL;
    
    aes->x_field = NULL;
    aes->y_field = NULL;
    aes->color_field = NULL;
    aes->size_field = NULL;
    aes->shape_field = NULL;
    aes->alpha_field = NULL;
    
    return aes;
}

// Simple color palette (ggplot2-like colors)
static const char *get_color_from_palette(int index) {
    static const char *colors[] = {
        "#1f77b4",  // blue
        "#ff7f0e",  // orange  
        "#2ca02c",  // green
        "#d62728",  // red
        "#9467bd",  // purple
        "#8c564b",  // brown
        "#e377c2",  // pink
        "#7f7f7f",  // gray
        "#bcbd22",  // olive
        "#17becf"   // cyan
    };
    int num_colors = sizeof(colors) / sizeof(colors[0]);
    return colors[index % num_colors];
}

// SVG Builder functions
static SVGBuilder *svg_builder_create(void *arena, arena_alloc_func alloc_func) {
    SVGBuilder *svg = alloc_func(arena, sizeof(SVGBuilder));
    if (!svg) return NULL;
    
    svg->capacity = 4096;
    svg->buffer = alloc_func(arena, svg->capacity);
    if (!svg->buffer) return NULL;
    
    svg->size = 0;
    svg->arena = arena;
    svg->alloc_func = alloc_func;
    svg->buffer[0] = '\0';
    
    return svg;
}

static void svg_append(SVGBuilder *svg, const char *str) {
    if (!svg || !str) return;
    
    size_t len = strlen(str);
    size_t needed = svg->size + len + 1;
    
    if (needed > svg->capacity) {
        size_t new_capacity = svg->capacity * 2;
        while (new_capacity < needed) new_capacity *= 2;
        
        char *new_buffer = svg->alloc_func(svg->arena, new_capacity);
        if (!new_buffer) return;
        
        memcpy(new_buffer, svg->buffer, svg->size);
        svg->buffer = new_buffer;
        svg->capacity = new_capacity;
    }
    
    memcpy(svg->buffer + svg->size, str, len);
    svg->size += len;
    svg->buffer[svg->size] = '\0';
}

static void svg_begin(SVGBuilder *svg, double width, double height) {
    char header[512];
    snprintf(header, sizeof(header), 
        "<svg width=\"%.0f\" height=\"%.0f\" viewBox=\"0 0 %.0f %.0f\" "
        "xmlns=\"http://www.w3.org/2000/svg\">\n", 
        width, height, width, height);
    svg_append(svg, header);
}

static void svg_end(SVGBuilder *svg) {
    svg_append(svg, "</svg>\n");
}

static void svg_circle(SVGBuilder *svg, double x, double y, double r, 
                      const char *fill, const char *stroke) {
    char circle[256];
    snprintf(circle, sizeof(circle),
        "<circle cx=\"%.2f\" cy=\"%.2f\" r=\"%.2f\" fill=\"%s\" stroke=\"%s\"/>\n",
        x, y, r, fill ? fill : "none", stroke ? stroke : "none");
    svg_append(svg, circle);
}

static void svg_line(SVGBuilder *svg, double x1, double y1, double x2, double y2, 
                    const char *stroke, double width) {
    char line[256];
    snprintf(line, sizeof(line),
        "<line x1=\"%.2f\" y1=\"%.2f\" x2=\"%.2f\" y2=\"%.2f\" "
        "stroke=\"%s\" stroke-width=\"%.2f\"/>\n",
        x1, y1, x2, y2, stroke ? stroke : "black", width);
    svg_append(svg, line);
}

static void svg_text(SVGBuilder *svg, double x, double y, const char *text, 
                    const char *font_family, double font_size) {
    char text_elem[512];
    snprintf(text_elem, sizeof(text_elem),
        "<text x=\"%.2f\" y=\"%.2f\" font-family=\"%s\" font-size=\"%.2f\">%s</text>\n",
        x, y, font_family ? font_family : "Arial", font_size, text ? text : "");
    svg_append(svg, text_elem);
}

static void svg_rect(SVGBuilder *svg, double x, double y, double width, double height, 
                    const char *fill, const char *stroke, double stroke_width) {
    char rect[256];
    snprintf(rect, sizeof(rect),
        "<rect x=\"%.2f\" y=\"%.2f\" width=\"%.2f\" height=\"%.2f\" "
        "fill=\"%s\" stroke=\"%s\" stroke-width=\"%.2f\"/>\n",
        x, y, width, height, 
        fill ? fill : "none", stroke ? stroke : "none", stroke_width);
    svg_append(svg, rect);
}

static void svg_path(SVGBuilder *svg, const char *path_data, const char *fill, 
                    const char *stroke, double stroke_width) {
    char path[1024];
    snprintf(path, sizeof(path),
        "<path d=\"%s\" fill=\"%s\" stroke=\"%s\" stroke-width=\"%.2f\"/>\n",
        path_data, fill ? fill : "none", stroke ? stroke : "none", stroke_width);
    svg_append(svg, path);
}

// Lexer functions
static PlotLexer *plot_lexer_create(void *arena, arena_alloc_func alloc_func) {
    PlotLexer *lexer = alloc_func(arena, sizeof(PlotLexer));
    if (!lexer) return NULL;
    
    lexer->capacity = 64;
    lexer->tokens = alloc_func(arena, sizeof(PlotToken) * lexer->capacity);
    if (!lexer->tokens) return NULL;
    
    lexer->count = 0;
    lexer->current = 0;
    lexer->arena = arena;
    lexer->alloc_func = alloc_func;
    
    return lexer;
}

static void plot_lexer_add_token(PlotLexer *lexer, PlotTokenType type, 
                                const char *value, size_t start, size_t length) {
    if (lexer->count >= lexer->capacity) {
        // For now, just ignore if we run out of space
        return;
    }
    
    PlotToken *token = &lexer->tokens[lexer->count++];
    token->type = type;
    token->start = start;
    token->length = length;
    
    if (value) {
        token->value = arena_strdup(lexer->arena, lexer->alloc_func, value);
    } else {
        token->value = NULL;
    }
}

static int is_alpha(char c) {
    return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || c == '_';
}

static int is_alnum(char c) {
    return is_alpha(c) || (c >= '0' && c <= '9');
}

static PlotLexer *plot_tokenize(const char *input, void *arena, arena_alloc_func alloc_func) {
    if (!input) return NULL;
    
    PlotLexer *lexer = plot_lexer_create(arena, alloc_func);
    if (!lexer) return NULL;
    
    size_t i = 0;
    size_t len = strlen(input);
    
    while (i < len) {
        char c = input[i];
        
        // Skip whitespace
        if (isspace(c)) {
            i++;
            continue;
        }
        
        // Single character tokens
        switch (c) {
            case '+':
                plot_lexer_add_token(lexer, PLOT_TOKEN_PLUS, NULL, i, 1);
                i++;
                continue;
            case '(':
                plot_lexer_add_token(lexer, PLOT_TOKEN_LPAREN, NULL, i, 1);
                i++;
                continue;
            case ')':
                plot_lexer_add_token(lexer, PLOT_TOKEN_RPAREN, NULL, i, 1);
                i++;
                continue;
            case ',':
                plot_lexer_add_token(lexer, PLOT_TOKEN_COMMA, NULL, i, 1);
                i++;
                continue;
            case '=':
                plot_lexer_add_token(lexer, PLOT_TOKEN_EQUALS, NULL, i, 1);
                i++;
                continue;
            case '.':
                plot_lexer_add_token(lexer, PLOT_TOKEN_DOT, NULL, i, 1);
                i++;
                continue;
        }
        
        // String literals
        if (c == '"') {
            size_t start = i;
            i++; // Skip opening quote
            while (i < len && input[i] != '"') {
                i++;
            }
            if (i < len) i++; // Skip closing quote
            
            size_t content_len = i - start - 2;
            char *str_value = alloc_func(arena, content_len + 1);
            if (str_value) {
                memcpy(str_value, &input[start + 1], content_len);
                str_value[content_len] = '\0';
            }
            plot_lexer_add_token(lexer, PLOT_TOKEN_STRING, str_value, start, i - start);
            continue;
        }
        
        // Numbers
        if (isdigit(c) || c == '.') {
            size_t start = i;
            while (i < len && (isdigit(input[i]) || input[i] == '.')) {
                i++;
            }
            
            char *num_str = alloc_func(arena, i - start + 1);
            if (num_str) {
                memcpy(num_str, &input[start], i - start);
                num_str[i - start] = '\0';
            }
            plot_lexer_add_token(lexer, PLOT_TOKEN_NUMBER, num_str, start, i - start);
            continue;
        }
        
        // Identifiers and keywords
        if (is_alpha(c)) {
            size_t start = i;
            while (i < len && is_alnum(input[i])) {
                i++;
            }
            
            char *ident = alloc_func(arena, i - start + 1);
            if (ident) {
                memcpy(ident, &input[start], i - start);
                ident[i - start] = '\0';
                
                // Check for keywords - but don't treat field names after dots as keywords
                PlotTokenType type = PLOT_TOKEN_IDENTIFIER;
                
                // Look back to see if this identifier follows a dot
                bool follows_dot = false;
                if (lexer->count > 0 && lexer->tokens[lexer->count - 1].type == PLOT_TOKEN_DOT) {
                    follows_dot = true;
                }
                
                if (!follows_dot) {
                    // Only apply keyword rules if not following a dot
                    if (strcmp(ident, "data") == 0) type = PLOT_TOKEN_DATA;
                    else if (strcmp(ident, "aes") == 0) type = PLOT_TOKEN_AES;
                    else if (strcmp(ident, "geom_point") == 0) type = PLOT_TOKEN_GEOM_POINT;
                    else if (strcmp(ident, "geom_line") == 0) type = PLOT_TOKEN_GEOM_LINE;
                    else if (strcmp(ident, "geom_bar") == 0) type = PLOT_TOKEN_GEOM_BAR;
                    else if (strcmp(ident, "geom_area") == 0) type = PLOT_TOKEN_GEOM_AREA;
                    else if (strcmp(ident, "labs") == 0) type = PLOT_TOKEN_LABS;
                    else if (strncmp(ident, "theme_", 6) == 0) type = PLOT_TOKEN_THEME;
                }
                
                plot_lexer_add_token(lexer, type, ident, start, i - start);
            }
            continue;
        }
        
        // Unknown character, skip it
        i++;
    }
    
    // Add EOF token
    plot_lexer_add_token(lexer, PLOT_TOKEN_EOF, NULL, len, 0);
    
    return lexer;
}

// Create error response
static json_t *create_error(const char *type, const char *message, const char *context) {
    json_t *error_obj = json_object();
    json_t *errors_array = json_array();
    json_t *error_detail = json_object();
    
    json_object_set_new(error_detail, "type", json_string(type));
    json_object_set_new(error_detail, "message", json_string(message));
    if (context) {
        json_object_set_new(error_detail, "context", json_string(context));
    }
    
    json_array_append_new(errors_array, error_detail);
    json_object_set_new(error_obj, "errors", errors_array);
    
    return error_obj;
}

// Enhanced plot data extraction with aesthetics support and debugging
static PlotData *extract_plot_data(json_t *input, const char *data_field, PlotAesthetics *aes,
                                  void *arena, arena_alloc_func alloc_func) {
    
    if (!input || !data_field) {
        return NULL;
    }
    
    // Handle .field syntax - skip the '.' prefix
    const char *field_name = data_field;
    if (field_name[0] == '.') {
        field_name = field_name + 1;
    }
    
    // Extract the data field from input JSON
    json_t *data_json = json_object_get(input, field_name);
    if (!data_json) {
        return NULL;
    }
    
    if (!json_is_array(data_json)) {
        return NULL;
    }
    
    size_t array_size = json_array_size(data_json);
    
    if (array_size == 0) {
        return NULL;
    }
    
    PlotData *data = alloc_func(arena, sizeof(PlotData));
    if (!data) {
        return NULL;
    }
    
    data->x_values = alloc_func(arena, sizeof(double) * array_size);
    data->y_values = alloc_func(arena, sizeof(double) * array_size);
    data->colors = alloc_func(arena, sizeof(double) * array_size);
    data->sizes = alloc_func(arena, sizeof(double) * array_size);
    if (!data->x_values || !data->y_values || !data->colors || !data->sizes) {
        return NULL;
    }
    
    data->count = 0;
    data->capacity = array_size;
    data->categories = NULL;
    
    // Use aesthetics to determine field mappings, fallback to defaults
    if (aes && aes->x_field) {
        data->x_field = arena_strdup(arena, alloc_func, aes->x_field);
    } else {
        data->x_field = arena_strdup(arena, alloc_func, "x");
    }
    
    if (aes && aes->y_field) {
        data->y_field = arena_strdup(arena, alloc_func, aes->y_field);
    } else {
        data->y_field = arena_strdup(arena, alloc_func, "y");
    }
    
    data->color_field = (aes && aes->color_field) ? arena_strdup(arena, alloc_func, aes->color_field) : NULL;
    data->size_field = (aes && aes->size_field) ? arena_strdup(arena, alloc_func, aes->size_field) : NULL;
    
    // Extract data points
    for (size_t i = 0; i < array_size; i++) {
        json_t *point = json_array_get(data_json, i);
        if (!point) continue;
        
        if (json_is_array(point) && json_array_size(point) >= 2) {
            // Array format: [x, y]
            json_t *x_val = json_array_get(point, 0);
            json_t *y_val = json_array_get(point, 1);
            
            if (json_is_number(x_val) && json_is_number(y_val)) {
                data->x_values[data->count] = json_number_value(x_val);
                data->y_values[data->count] = json_number_value(y_val);
                
                // Set default values for array format
                data->colors[data->count] = 1.0; // Default color index
                data->sizes[data->count] = 3.0;  // Default size
                
                data->count++;
            }
        } else if (json_is_object(point)) {
            // Object format: {x: 1, y: 10, ...}
            json_t *x_val = json_object_get(point, data->x_field);
            json_t *y_val = json_object_get(point, data->y_field);
            
            double x_numeric = 0.0;
            double y_numeric = 0.0;
            bool x_valid = false;
            bool y_valid = false;
            
            // Handle x value - support both numeric and string values
            if (x_val) {
                if (json_is_number(x_val)) {
                    x_numeric = json_number_value(x_val);
                    x_valid = true;
                } else if (json_is_string(x_val)) {
                    // Convert string to numeric index (categorical data)
                    // Simple mapping: assign sequential indices to unique string values
                    const char *x_str = json_string_value(x_val);
                    if (x_str) {
                        // For categorical data, create a simple hash-based index
                        // This is a basic approach - could be improved with proper categorical mapping
                        size_t hash = 0;
                        for (const char *c = x_str; *c; c++) {
                            hash = hash * 31 + (unsigned char)*c;
                        }
                        x_numeric = (double)(hash % 100); // Use modulo to keep values reasonable
                        x_valid = true;
                    }
                }
            }
            
            // Handle y value - must be numeric
            if (y_val && json_is_number(y_val)) {
                y_numeric = json_number_value(y_val);
                y_valid = true;
            }
            
            if (x_valid && y_valid) {
                data->x_values[data->count] = x_numeric;
                data->y_values[data->count] = y_numeric;
                
                // Extract color and size mappings if specified
                data->colors[data->count] = 1.0; // Default color index
                data->sizes[data->count] = 3.0;  // Default size
                
                if (data->color_field) {
                    json_t *color_val = json_object_get(point, data->color_field);
                    if (color_val && json_is_number(color_val)) {
                        data->colors[data->count] = json_number_value(color_val);
                    }
                }
                
                if (data->size_field) {
                    json_t *size_val = json_object_get(point, data->size_field);
                    if (size_val && json_is_number(size_val)) {
                        data->sizes[data->count] = json_number_value(size_val);
                    }
                }
                
                data->count++;
            }
        }
    }
    
    return data->count > 0 ? data : NULL;
}

// Render basic plot
static void render_plot(SVGBuilder *svg, PlotSpec *spec) {
    if (!svg || !spec) return;
    
    svg_begin(svg, spec->width, spec->height);
    
    // Calculate plot area
    double plot_x = spec->margin_left;
    double plot_y = spec->margin_top;
    double plot_w = spec->width - spec->margin_left - spec->margin_right;
    double plot_h = spec->height - spec->margin_top - spec->margin_bottom;
    
    // Find data ranges
    double min_x = HUGE_VAL, max_x = -HUGE_VAL;
    double min_y = HUGE_VAL, max_y = -HUGE_VAL;
    
    if (spec->data && spec->data->count > 0) {
        for (size_t i = 0; i < spec->data->count; i++) {
            double x = spec->data->x_values[i];
            double y = spec->data->y_values[i];
            if (x < min_x) min_x = x;
            if (x > max_x) max_x = x;
            if (y < min_y) min_y = y;
            if (y > max_y) max_y = y;
        }
    }
    
    // Add some padding to ranges
    double x_range = max_x - min_x;
    double y_range = max_y - min_y;
    if (x_range < 1e-10) x_range = 1.0;
    if (y_range < 1e-10) y_range = 1.0;
    
    min_x -= x_range * 0.05;
    max_x += x_range * 0.05;
    min_y -= y_range * 0.05;
    max_y += y_range * 0.05;
    
    // Draw background
    char bg[256];
    snprintf(bg, sizeof(bg), 
        "<rect x=\"%.2f\" y=\"%.2f\" width=\"%.2f\" height=\"%.2f\" "
        "fill=\"white\" stroke=\"#e5e5e5\"/>\n",
        plot_x, plot_y, plot_w, plot_h);
    svg_append(svg, bg);
    
    // Draw axes
    svg_line(svg, plot_x, plot_y + plot_h, plot_x + plot_w, plot_y + plot_h, "#333", 1.0);
    svg_line(svg, plot_x, plot_y, plot_x, plot_y + plot_h, "#333", 1.0);
    
    // Render data layers
    PlotLayer *layer = spec->layers;
    while (layer) {
        if (layer->data && layer->data->count > 0) {
            for (size_t i = 0; i < layer->data->count; i++) {
                double x = layer->data->x_values[i];
                double y = layer->data->y_values[i];
                
                // Scale to plot coordinates
                double px = plot_x + ((x - min_x) / (max_x - min_x)) * plot_w;
                double py = plot_y + plot_h - ((y - min_y) / (max_y - min_y)) * plot_h;
                
                if (layer->type == GEOM_POINT) {
                    // Use aesthetics for color and size
                    double point_size = layer->data->sizes[i];
                    const char *color = "steelblue"; // Default color
                    
                    // Use color mapping if available
                    if (layer->aes && layer->aes->color_field && layer->data->color_field) {
                        int color_index = (int)layer->data->colors[i];
                        color = get_color_from_palette(color_index);
                    }
                    svg_circle(svg, px, py, point_size, color, color);
                }
            }
            
            // Draw lines for line geom
            if (layer->type == GEOM_LINE && layer->data->count > 1) {
                const char *line_color = "steelblue"; // Default color
                
                // Use first point's color for the entire line
                if (layer->aes && layer->aes->color_field && layer->data->color_field) {
                    int color_index = (int)layer->data->colors[0];
                    line_color = get_color_from_palette(color_index);
                }
                
                for (size_t i = 0; i < layer->data->count - 1; i++) {
                    double x1 = layer->data->x_values[i];
                    double y1 = layer->data->y_values[i];
                    double x2 = layer->data->x_values[i + 1];
                    double y2 = layer->data->y_values[i + 1];
                    
                    double px1 = plot_x + ((x1 - min_x) / (max_x - min_x)) * plot_w;
                    double py1 = plot_y + plot_h - ((y1 - min_y) / (max_y - min_y)) * plot_h;
                    double px2 = plot_x + ((x2 - min_x) / (max_x - min_x)) * plot_w;
                    double py2 = plot_y + plot_h - ((y2 - min_y) / (max_y - min_y)) * plot_h;
                    
                    svg_line(svg, px1, py1, px2, py2, line_color, 2.0);
                }
            }
            
            // Draw bars for bar geom with proper grouping
            if (layer->type == GEOM_BAR && layer->data->count > 0) {
                // First, identify unique x-values (quarters) and count bars per group
                double unique_x[100]; // Max 100 unique x-values
                size_t bars_per_group[100]; // Count of bars for each x-value
                size_t unique_count = 0;
                
                // Find unique x-values and count bars per group
                for (size_t i = 0; i < layer->data->count; i++) {
                    double x = layer->data->x_values[i];
                    bool found = false;
                    
                    for (size_t j = 0; j < unique_count; j++) {
                        if (fabs(unique_x[j] - x) < 1e-9) { // Found existing group
                            bars_per_group[j]++;
                            found = true;
                            break;
                        }
                    }
                    
                    if (!found && unique_count < 100) { // New group
                        unique_x[unique_count] = x;
                        bars_per_group[unique_count] = 1;
                        unique_count++;
                    }
                }
                
                // Calculate spacing: 70% for bars, 30% for spacing between quarters
                double total_bar_space = plot_w * 0.7;
                double total_quarter_spacing = plot_w * 0.3;
                double quarter_width = total_bar_space / unique_count;
                double quarter_gap = (unique_count > 1) ? total_quarter_spacing / (unique_count - 1) : 0;
                
                // Track position within each quarter group
                size_t group_positions[100] = {0}; // Current position within each group
                
                for (size_t i = 0; i < layer->data->count; i++) {
                    double x = layer->data->x_values[i];
                    double y = layer->data->y_values[i];
                    
                    // Find which quarter group this bar belongs to
                    size_t group_index = 0;
                    for (size_t j = 0; j < unique_count; j++) {
                        if (fabs(unique_x[j] - x) < 1e-9) {
                            group_index = j;
                            break;
                        }
                    }
                    
                    // Calculate bar dimensions
                    double bar_width = quarter_width / bars_per_group[group_index] * 0.8; // 80% of available space within group
                    double bar_spacing_within_group = quarter_width / bars_per_group[group_index] * 0.2; // 20% for spacing within group
                    
                    // Calculate bar position
                    double quarter_start_x = plot_x + (group_index * (quarter_width + quarter_gap));
                    double bar_x = quarter_start_x + 
                                  (group_positions[group_index] * (bar_width + bar_spacing_within_group / bars_per_group[group_index])) + 
                                  bar_spacing_within_group / 2.0;
                    
                    double bar_height = ((y - min_y) / (max_y - min_y)) * plot_h;
                    double bar_y = plot_y + plot_h - bar_height;
                    
                    // Ensure bars don't have negative height
                    if (bar_height < 0) {
                        bar_height = 0;
                        bar_y = plot_y + plot_h;
                    }
                    
                    // Use color mapping for bars
                    const char *bar_color = "steelblue"; // Default color
                    if (layer->aes && layer->aes->color_field && layer->data->color_field) {
                        int color_index = (int)layer->data->colors[i];
                        bar_color = get_color_from_palette(color_index);
                    }
                    
                    svg_rect(svg, bar_x, bar_y, bar_width, bar_height, bar_color, bar_color, 1.0);
                    
                    // Increment position within this quarter group
                    group_positions[group_index]++;
                }
            }
            
            // Draw area for area geom
            if (layer->type == GEOM_AREA && layer->data->count > 1) {
                // Build path data for area fill
                char path_data[2048];
                size_t path_pos = 0;
                
                // Start from bottom-left of first point
                double first_x = layer->data->x_values[0];
                double first_px = plot_x + ((first_x - min_x) / (max_x - min_x)) * plot_w;
                double baseline_py = plot_y + plot_h;
                
                path_pos += (size_t)snprintf(path_data + path_pos, sizeof(path_data) - path_pos,
                                   "M %.2f %.2f", first_px, baseline_py);
                
                // Draw line to each data point
                for (size_t i = 0; i < layer->data->count; i++) {
                    double x = layer->data->x_values[i];
                    double y = layer->data->y_values[i];
                    
                    double px = plot_x + ((x - min_x) / (max_x - min_x)) * plot_w;
                    double py = plot_y + plot_h - ((y - min_y) / (max_y - min_y)) * plot_h;
                    
                    path_pos += (size_t)snprintf(path_data + path_pos, sizeof(path_data) - path_pos,
                                       " L %.2f %.2f", px, py);
                }
                
                // Close the path back to baseline
                double last_x = layer->data->x_values[layer->data->count - 1];
                double last_px = plot_x + ((last_x - min_x) / (max_x - min_x)) * plot_w;
                
                snprintf(path_data + path_pos, sizeof(path_data) - path_pos,
                        " L %.2f %.2f Z", last_px, baseline_py);
                
                // Render the filled area
                svg_path(svg, path_data, "lightsteelblue", "steelblue", 1.0);
            }
        }
        layer = layer->next;
    }
    
    // Add title if present
    if (spec->title) {
        svg_text(svg, spec->width / 2, 30, spec->title, "Arial", 16);
    }
    
    // Add axis labels if present
    if (spec->x_label) {
        // X-axis label at bottom center
        svg_text(svg, spec->width / 2, spec->height - 10, spec->x_label, "Arial", 12);
    }
    
    if (spec->y_label) {
        // Y-axis label on left side, rotated (for now, just positioned vertically)
        // TODO: Add text rotation support for proper Y-axis label
        svg_text(svg, 15, spec->height / 2, spec->y_label, "Arial", 12);
    }
    
    svg_end(svg);
}

// Main middleware execute function
json_t *middleware_execute(json_t *input, void *arena, 
                          arena_alloc_func alloc_func, 
                          arena_free_func free_func, 
                          const char *plot_spec,
                          json_t *middleware_config,
                          char **contentType,
                          json_t *variables) {
    
    (void)free_func;
    (void)variables;
    
    if (!plot_spec || strlen(plot_spec) == 0) {
        return create_error("plotError", "No plot specification provided", NULL);
    }
    
    // Set content type to SVG
    *contentType = arena_strdup(arena, alloc_func, "image/svg+xml");
    
    // Parse default config
    double width = 800;
    double height = 600;
    if (middleware_config) {
        json_t *w = json_object_get(middleware_config, "width");
        json_t *h = json_object_get(middleware_config, "height");
        if (w && json_is_number(w)) width = json_number_value(w);
        if (h && json_is_number(h)) height = json_number_value(h);
    }
    
    // Tokenize the plot specification
    PlotLexer *lexer = plot_tokenize(plot_spec, arena, alloc_func);
    if (!lexer) {
        return create_error("plotError", "Failed to parse plot specification", plot_spec);
    }
    
    // Create plot spec
    PlotSpec *spec = alloc_func(arena, sizeof(PlotSpec));
    if (!spec) {
        return create_error("plotError", "Memory allocation failed", NULL);
    }
    
    spec->data = NULL;
    spec->global_aes = create_aesthetics(arena, alloc_func);
    spec->layers = NULL;
    spec->title = NULL;
    spec->x_label = NULL;
    spec->y_label = NULL;
    spec->width = width;
    spec->height = height;
    spec->margin_top = 50;
    spec->margin_right = 50;
    spec->margin_bottom = 50;
    spec->margin_left = 60;
    
    // Step 1: Find the data source specification  
    const char *data_field = NULL;
    for (size_t i = 0; i < lexer->count; i++) {
        if (lexer->tokens[i].type == PLOT_TOKEN_DATA) {
            // Expect: data ( .field )
            if (i + 4 < lexer->count &&
                lexer->tokens[i + 1].type == PLOT_TOKEN_LPAREN &&
                lexer->tokens[i + 2].type == PLOT_TOKEN_DOT &&
                lexer->tokens[i + 3].type == PLOT_TOKEN_IDENTIFIER &&
                lexer->tokens[i + 4].type == PLOT_TOKEN_RPAREN) {
                
                data_field = lexer->tokens[i + 3].value;
                break;
            }
        }
    }
    
    if (!data_field) {
        return create_error("plotError", "No data specification found", "Add data(.field)");
    }
    
    // Step 2: Parse aesthetics to get field mappings
    for (size_t i = 0; i < lexer->count; i++) {
        if (lexer->tokens[i].type == PLOT_TOKEN_AES) {
            // Expect: aes ( ... )
            if (i + 1 < lexer->count && lexer->tokens[i + 1].type == PLOT_TOKEN_LPAREN) {
                // Find the matching closing parenthesis
                size_t j = i + 2;
                int paren_count = 1;
                while (j < lexer->count && paren_count > 0) {
                    if (lexer->tokens[j].type == PLOT_TOKEN_LPAREN) paren_count++;
                    if (lexer->tokens[j].type == PLOT_TOKEN_RPAREN) paren_count--;
                    j++;
                }
                
                // Parse aesthetic mappings between parentheses
                for (size_t k = i + 2; k < j - 1; k++) {
                    if (lexer->tokens[k].type == PLOT_TOKEN_IDENTIFIER) {
                        const char *aes_name = lexer->tokens[k].value;
                        if (aes_name && k + 2 < j - 1 && 
                            lexer->tokens[k + 1].type == PLOT_TOKEN_EQUALS &&
                            lexer->tokens[k + 2].type == PLOT_TOKEN_IDENTIFIER) {
                            
                            const char *field_name = lexer->tokens[k + 2].value;
                            
                            if (strcmp(aes_name, "x") == 0) {
                                spec->global_aes->x_field = arena_strdup(arena, alloc_func, field_name);
                            } else if (strcmp(aes_name, "y") == 0) {
                                spec->global_aes->y_field = arena_strdup(arena, alloc_func, field_name);
                            } else if (strcmp(aes_name, "color") == 0 || strcmp(aes_name, "colour") == 0) {
                                spec->global_aes->color_field = arena_strdup(arena, alloc_func, field_name);
                            } else if (strcmp(aes_name, "size") == 0) {
                                spec->global_aes->size_field = arena_strdup(arena, alloc_func, field_name);
                            } else if (strcmp(aes_name, "shape") == 0) {
                                spec->global_aes->shape_field = arena_strdup(arena, alloc_func, field_name);
                            } else if (strcmp(aes_name, "alpha") == 0) {
                                spec->global_aes->alpha_field = arena_strdup(arena, alloc_func, field_name);
                            }
                            k += 2; // Skip the = and field tokens
                        }
                    }
                }
                break;
            }
        }
    }
    
    // Step 3: Now extract data using the data source + aesthetics  
    spec->data = extract_plot_data(input, data_field, spec->global_aes, arena, alloc_func);
    
    if (!spec->data) {
        return create_error("plotError", "Failed to extract data from input", "Check data(.field) specification");
    }
    
    // Look for labs() and extract labels
    for (size_t i = 0; i < lexer->count; i++) {
        if (lexer->tokens[i].type == PLOT_TOKEN_LABS) {
            // Expect: labs ( ... )
            if (i + 1 < lexer->count && lexer->tokens[i + 1].type == PLOT_TOKEN_LPAREN) {
                // Find the matching closing parenthesis
                size_t j = i + 2;
                int paren_count = 1;
                while (j < lexer->count && paren_count > 0) {
                    if (lexer->tokens[j].type == PLOT_TOKEN_LPAREN) paren_count++;
                    if (lexer->tokens[j].type == PLOT_TOKEN_RPAREN) paren_count--;
                    j++;
                }
                
                // Parse parameters between parentheses
                for (size_t k = i + 2; k < j - 1; k++) {
                    if (lexer->tokens[k].type == PLOT_TOKEN_IDENTIFIER) {
                        const char *param_name = lexer->tokens[k].value;
                        if (param_name && k + 2 < j - 1 && 
                            lexer->tokens[k + 1].type == PLOT_TOKEN_EQUALS) {
                            
                            if (lexer->tokens[k + 2].type == PLOT_TOKEN_STRING) {
                                // String literal value
                                const char *value = lexer->tokens[k + 2].value;
                                if (strcmp(param_name, "title") == 0) {
                                    spec->title = arena_strdup(arena, alloc_func, value);
                                } else if (strcmp(param_name, "x") == 0) {
                                    spec->x_label = arena_strdup(arena, alloc_func, value);
                                } else if (strcmp(param_name, "y") == 0) {
                                    spec->y_label = arena_strdup(arena, alloc_func, value);
                                }
                            } else if (lexer->tokens[k + 2].type == PLOT_TOKEN_IDENTIFIER) {
                                // Field reference - extract from input JSON
                                const char *field_name = lexer->tokens[k + 2].value;
                                json_t *field_value = json_object_get(input, field_name);
                                if (field_value && json_is_string(field_value)) {
                                    const char *value = json_string_value(field_value);
                                    if (strcmp(param_name, "title") == 0) {
                                        spec->title = arena_strdup(arena, alloc_func, value);
                                    } else if (strcmp(param_name, "x") == 0) {
                                        spec->x_label = arena_strdup(arena, alloc_func, value);
                                    } else if (strcmp(param_name, "y") == 0) {
                                        spec->y_label = arena_strdup(arena, alloc_func, value);
                                    }
                                }
                            }
                            k += 2; // Skip the = and value tokens
                        }
                    }
                }
                break;
            }
        }
    }
    
    // Look for geometries and create layers
    PlotLayer *last_layer = NULL;
    for (size_t i = 0; i < lexer->count; i++) {
        if (lexer->tokens[i].type == PLOT_TOKEN_GEOM_POINT || 
            lexer->tokens[i].type == PLOT_TOKEN_GEOM_LINE ||
            lexer->tokens[i].type == PLOT_TOKEN_GEOM_BAR ||
            lexer->tokens[i].type == PLOT_TOKEN_GEOM_AREA) {
            
            PlotLayer *layer = alloc_func(arena, sizeof(PlotLayer));
            if (!layer) continue;
            
            if (lexer->tokens[i].type == PLOT_TOKEN_GEOM_POINT) {
                layer->type = GEOM_POINT;
            } else if (lexer->tokens[i].type == PLOT_TOKEN_GEOM_LINE) {
                layer->type = GEOM_LINE;
            } else if (lexer->tokens[i].type == PLOT_TOKEN_GEOM_BAR) {
                layer->type = GEOM_BAR;
            } else if (lexer->tokens[i].type == PLOT_TOKEN_GEOM_AREA) {
                layer->type = GEOM_AREA;
            }
            
            layer->data = spec->data; // Use the same data (already re-extracted with aesthetics if needed)
            layer->aes = spec->global_aes; // Use global aesthetics
            layer->params = NULL; // No parameters needed for basic geometries
            layer->next = NULL;
            
            if (!spec->layers) {
                spec->layers = layer;
            } else {
                last_layer->next = layer;
            }
            last_layer = layer;
        }
    }
    
    if (!spec->layers) {
        return create_error("plotError", "No geometry layers specified", "Add geom_point() or geom_line()");
    }
    
    // Generate SVG
    SVGBuilder *svg = svg_builder_create(arena, alloc_func);
    if (!svg) {
        return create_error("plotError", "Failed to create SVG builder", NULL);
    }
    
    render_plot(svg, spec);
    
    // Return SVG as string
    return json_string(svg->buffer);
}
