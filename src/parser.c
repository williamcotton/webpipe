#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdbool.h>
#include "wp.h"

// Parser functions
Parser *parser_new(Token *tokens, int token_count) {
  Parser *parser = malloc(sizeof(Parser));
  parser->tokens = tokens;
  parser->token_count = token_count;
  parser->current = 0;
  parser->ctx = NULL;  // No arena context for backward compatibility
  return parser;
}

Parser *parser_new_with_context(Token *tokens, int token_count, ParseContext *ctx) {
  Parser *parser = malloc(sizeof(Parser));
  parser->tokens = tokens;
  parser->token_count = token_count;
  parser->current = 0;
  parser->ctx = ctx;
  return parser;
}

void parser_free(Parser *parser) { free(parser); }

bool parser_is_at_end(Parser *parser) {
  return parser->current >= parser->token_count ||
         parser->tokens[parser->current].type == TOKEN_EOF;
}

Token *parser_peek(Parser *parser) {
  if (parser_is_at_end(parser))
    return NULL;
  return &parser->tokens[parser->current];
}

Token *parser_advance(Parser *parser) {
  if (!parser_is_at_end(parser))
    parser->current++;
  return &parser->tokens[parser->current - 1];
}

bool parser_check(Parser *parser, TokenType type) {
  if (parser_is_at_end(parser))
    return false;
  return parser_peek(parser)->type == type;
}

bool parser_match(Parser *parser, TokenType type) {
  if (parser_check(parser, type)) {
    parser_advance(parser);
    return true;
  }
  return false;
}

void parser_consume_newlines(Parser *parser) {
  while (parser_match(parser, TOKEN_NEWLINE) || parser_match(parser, TOKEN_COMMENT)) {
  }
}

// Error recovery function to skip invalid tokens
static void parser_skip_to_next_statement(Parser *parser) {
  while (!parser_is_at_end(parser) &&
         !parser_check(parser, TOKEN_HTTP_METHOD) &&
         !parser_check(parser, TOKEN_IDENTIFIER) &&
         !parser_check(parser, TOKEN_EOF)) {
    parser_advance(parser);
  }
}

PipelineStep *parser_parse_pipeline(Parser *parser) {
  PipelineStep *head = NULL;
  PipelineStep *tail = NULL;

  while (parser_match(parser, TOKEN_PIPE)) {
    if (!parser_check(parser, TOKEN_IDENTIFIER)) {
      fprintf(stderr, "Expected middleware name after |>\n");
      parser_skip_to_next_statement(parser);
      return head;
    }

    Token *middleware = parser_advance(parser);

    // Check for result step
    if (strcmp(middleware->value, "result") == 0) {
      // This is a result step - no colon needed, parse result conditions
      parser_consume_newlines(parser);
      
      ASTNode *result_node = parser_parse_result_step(parser);
      
      // Create a special pipeline step for result
      PipelineStep *step;
      if (parser->ctx && parser->ctx->parse_arena) {
        step = arena_alloc(parser->ctx->parse_arena, sizeof(PipelineStep));
        step->middleware = arena_strdup(parser->ctx->parse_arena, "result");
      } else {
        step = malloc(sizeof(PipelineStep));
        step->middleware = strdup_safe("result");
      }
      step->value = (char*)result_node; // Store result node as value
      step->is_variable = false;
      step->next = NULL;

      if (tail) {
        tail->next = step;
      } else {
        head = step;
      }
      tail = step;

      parser_consume_newlines(parser);
      continue;
    }

    if (!parser_match(parser, TOKEN_COLON)) {
      fprintf(stderr, "Expected : after middleware name\n");
      parser_skip_to_next_statement(parser);
      return head;
    }

    char *value = NULL;
    bool is_variable = false;

    if (parser_check(parser, TOKEN_STRING)) {
      if (parser->ctx && parser->ctx->parse_arena) {
        value = arena_strdup(parser->ctx->parse_arena, parser_advance(parser)->value);
      } else {
        value = strdup_safe(parser_advance(parser)->value);
      }
      is_variable = false;
    } else if (parser_check(parser, TOKEN_IDENTIFIER)) {
      if (parser->ctx && parser->ctx->parse_arena) {
        value = arena_strdup(parser->ctx->parse_arena, parser_advance(parser)->value);
      } else {
        value = strdup_safe(parser_advance(parser)->value);
      }
      is_variable = true;
    }

    PipelineStep *step;
    if (parser->ctx && parser->ctx->parse_arena) {
      step = arena_alloc(parser->ctx->parse_arena, sizeof(PipelineStep));
      step->middleware = arena_strdup(parser->ctx->parse_arena, middleware->value);
      step->value = value; // Already allocated in arena above
    } else {
      step = malloc(sizeof(PipelineStep));
      step->middleware = strdup_safe(middleware->value);
      step->value = value;
    }
    step->is_variable = is_variable;
    step->next = NULL;

    if (tail) {
      tail->next = step;
    } else {
      head = step;
    }
    tail = step;

    parser_consume_newlines(parser);
  }

  return head;
}

ASTNode *parser_parse_result_step(Parser *parser) {
  ASTNode *node;
  if (parser->ctx && parser->ctx->parse_arena) {
    node = arena_alloc(parser->ctx->parse_arena, sizeof(ASTNode));
  } else {
    node = malloc(sizeof(ASTNode));
  }
  node->type = AST_RESULT_STEP;
  node->data.result_step.conditions = NULL;

  ResultCondition *head = NULL;
  ResultCondition *tail = NULL;

  while (!parser_is_at_end(parser)) {
    // Skip any leading newlines between result conditions or before next statement
    parser_consume_newlines(parser);

    // Detect the start of a variable assignment (e.g. "mustache myPartial = `...`")
    // A variable assignment starts with IDENT IDENT =
    if (parser_check(parser, TOKEN_IDENTIFIER)) {
      int saved_index = parser->current;
      if (saved_index + 2 < parser->token_count &&
          parser->tokens[saved_index + 1].type == TOKEN_IDENTIFIER &&
          parser->tokens[saved_index + 2].type == TOKEN_EQUALS) {
        // We reached the beginning of the next statement, so exit the result block loop
        break;
      }
    }

    // Parse condition: name(status_code): pipeline
    if (!parser_check(parser, TOKEN_IDENTIFIER)) {
      // Check if we're at the end of the result block (next statement)
      if (parser_check(parser, TOKEN_HTTP_METHOD) || parser_check(parser, TOKEN_EOF)) {
        break;
      }
      fprintf(stderr, "Expected condition name\n");
      parser_advance(parser); // Skip the invalid token to prevent infinite loop
      break;
    }

    Token *condition_name = parser_advance(parser);

    if (!parser_match(parser, TOKEN_LPAREN)) {
      fprintf(stderr, "Expected ( after condition name\n");
      parser_advance(parser); // Skip the invalid token
      break;
    }

    if (!parser_check(parser, TOKEN_NUMBER)) {
      fprintf(stderr, "Expected status code number\n");
      parser_advance(parser); // Skip the invalid token
      break;
    }

    Token *status_code = parser_advance(parser);

    if (!parser_match(parser, TOKEN_RPAREN)) {
      fprintf(stderr, "Expected ) after status code\n");
      parser_advance(parser); // Skip the invalid token
      break;
    }

    if (!parser_match(parser, TOKEN_COLON)) {
      fprintf(stderr, "Expected : after condition\n");
      parser_advance(parser); // Skip the invalid token
      break;
    }

    parser_consume_newlines(parser);

    // Parse pipeline for this condition
    PipelineStep *pipeline = parser_parse_pipeline(parser);

    ResultCondition *condition;
    if (parser->ctx && parser->ctx->parse_arena) {
      condition = arena_alloc(parser->ctx->parse_arena, sizeof(ResultCondition));
      condition->condition_name = arena_strdup(parser->ctx->parse_arena, condition_name->value);
    } else {
      // Fallback for backward compatibility
      condition = malloc(sizeof(ResultCondition));
      condition->condition_name = strdup_safe(condition_name->value);
    }
    condition->status_code = atoi(status_code->value);
    condition->pipeline = pipeline;
    condition->next = NULL;

    if (tail) {
      tail->next = condition;
    } else {
      head = condition;
    }
    tail = condition;

    parser_consume_newlines(parser);
  }

  node->data.result_step.conditions = head;

  return node;
}

ASTNode *parser_parse_route_definition(Parser *parser) {
  Token *method = parser_advance(parser);

  if (!parser_check(parser, TOKEN_ROUTE)) {
    fprintf(stderr, "Expected route after HTTP method\n");
    parser_advance(parser); // Skip the invalid token
    return NULL;
  }

  Token *route = parser_advance(parser);
  parser_consume_newlines(parser);

  PipelineStep *pipeline = parser_parse_pipeline(parser);

  ASTNode *node;
  if (parser->ctx && parser->ctx->parse_arena) {
    node = arena_alloc(parser->ctx->parse_arena, sizeof(ASTNode));
    node->data.route_def.method = arena_strdup(parser->ctx->parse_arena, method->value);
    node->data.route_def.route = arena_strdup(parser->ctx->parse_arena, route->value);
  } else {
    node = malloc(sizeof(ASTNode));
    node->data.route_def.method = strdup_safe(method->value);
    node->data.route_def.route = strdup_safe(route->value);
  }
  node->type = AST_ROUTE_DEFINITION;
  node->data.route_def.pipeline = pipeline;

  return node;
}

ASTNode *parser_parse_variable_assignment(Parser *parser) {
  Token *middleware = parser_advance(parser);
  Token *name = parser_advance(parser);

  if (!parser_match(parser, TOKEN_EQUALS)) {
    fprintf(stderr, "Expected = in variable assignment\\n");
    parser_advance(parser); // Skip the invalid token
    return NULL;
  }

  if (!parser_check(parser, TOKEN_STRING)) {
    fprintf(stderr, "Expected string value in variable assignment, got token type %u\\n", 
            (unsigned int)parser_peek(parser)->type);
    parser_advance(parser); // Skip the invalid token
    return NULL;
  }

  Token *value = parser_advance(parser);

  ASTNode *node;
  if (parser->ctx && parser->ctx->parse_arena) {
    node = arena_alloc(parser->ctx->parse_arena, sizeof(ASTNode));
    node->data.var_assign.middleware = arena_strdup(parser->ctx->parse_arena, middleware->value);
    node->data.var_assign.name = arena_strdup(parser->ctx->parse_arena, name->value);
    node->data.var_assign.value = arena_strdup(parser->ctx->parse_arena, value->value);
  } else {
    node = malloc(sizeof(ASTNode));
    node->data.var_assign.middleware = strdup_safe(middleware->value);
    node->data.var_assign.name = strdup_safe(name->value);
    node->data.var_assign.value = strdup_safe(value->value);
  }
  node->type = AST_VARIABLE_ASSIGNMENT;

  return node;
}

ASTNode *parser_parse_pipeline_definition(Parser *parser) {
  parser_advance(parser); // Skip 'pipeline'
  
  if (!parser_check(parser, TOKEN_IDENTIFIER)) {
    fprintf(stderr, "Expected pipeline name after 'pipeline'\n");
    return NULL;
  }
  
  Token *name = parser_advance(parser);
  
  if (!parser_match(parser, TOKEN_EQUALS)) {
    fprintf(stderr, "Expected = after pipeline name\n");
    return NULL;
  }
  
  parser_consume_newlines(parser);
  
  // Parse the pipeline steps
  PipelineStep *pipeline = parser_parse_pipeline(parser);
  
  ASTNode *node;
  if (parser->ctx && parser->ctx->parse_arena) {
    node = arena_alloc(parser->ctx->parse_arena, sizeof(ASTNode));
    node->data.pipeline_def.name = arena_strdup(parser->ctx->parse_arena, name->value);
  } else {
    node = malloc(sizeof(ASTNode));
    node->data.pipeline_def.name = strdup_safe(name->value);
  }
  node->type = AST_PIPELINE_DEFINITION;
  node->data.pipeline_def.pipeline = pipeline;
  
  return node;
}

// Forward declarations for config parsing
ASTNode *parser_parse_config_value(Parser *parser);
ConfigProperty *parser_parse_config_properties(Parser *parser);
json_t *config_ast_to_json(ASTNode *node);
json_t *config_block_to_json(ASTNode *config_block);

// Parse a configuration block: config name { key: value, ... }
ASTNode *parser_parse_config_block(Parser *parser) {
  parser_advance(parser); // Skip 'config'
  
  if (!parser_check(parser, TOKEN_IDENTIFIER)) {
    fprintf(stderr, "Expected identifier after 'config'\n");
    return NULL;
  }
  
  Token *name = parser_advance(parser);
  
  if (!parser_check(parser, TOKEN_LBRACE)) {
    fprintf(stderr, "Expected '{' after config name\n");
    return NULL;
  }
  
  parser_advance(parser); // Skip '{'
  
  // Parse configuration properties
  ConfigProperty *properties = parser_parse_config_properties(parser);
  
  if (!parser_check(parser, TOKEN_RBRACE)) {
    fprintf(stderr, "Expected '}' to close config block\n");
    return NULL;
  }
  
  parser_advance(parser); // Skip '}'
  
  ASTNode *node;
  if (parser->ctx && parser->ctx->parse_arena) {
    node = arena_alloc(parser->ctx->parse_arena, sizeof(ASTNode));
    node->data.config_block.name = arena_strdup(parser->ctx->parse_arena, name->value);
  } else {
    node = malloc(sizeof(ASTNode));
    node->data.config_block.name = strdup_safe(name->value);
  }
  node->type = AST_CONFIG_BLOCK;
  node->data.config_block.properties = properties;
  
  return node;
}

// Parse configuration properties (key: value pairs)
ConfigProperty *parser_parse_config_properties(Parser *parser) {
  ConfigProperty *first = NULL;
  ConfigProperty *current = NULL;
  
  while (!parser_check(parser, TOKEN_RBRACE) && !parser_is_at_end(parser)) {
    // Skip newlines
    while (parser_check(parser, TOKEN_NEWLINE)) {
      parser_advance(parser);
    }
    
    // Check for end of block
    if (parser_check(parser, TOKEN_RBRACE)) {
      break;
    }
    
    // Parse key
    if (!parser_check(parser, TOKEN_IDENTIFIER)) {
      fprintf(stderr, "Expected identifier for config key\n");
      return first;
    }
    
    Token *key = parser_advance(parser);
    
    if (!parser_check(parser, TOKEN_COLON)) {
      fprintf(stderr, "Expected ':' after config key\n");
      return first;
    }
    
    parser_advance(parser); // Skip ':'
    
    // Parse value
    ASTNode *value = parser_parse_config_value(parser);
    if (!value) {
      fprintf(stderr, "Expected value for config key\n");
      return first;
    }
    
    // Create property
    ConfigProperty *prop;
    if (parser->ctx && parser->ctx->parse_arena) {
      prop = arena_alloc(parser->ctx->parse_arena, sizeof(ConfigProperty));
      prop->key = arena_strdup(parser->ctx->parse_arena, key->value);
    } else {
      prop = malloc(sizeof(ConfigProperty));
      prop->key = strdup_safe(key->value);
    }
    prop->value = value;
    prop->next = NULL;
    
    // Link to list
    if (!first) {
      first = prop;
      current = prop;
    } else {
      current->next = prop;
      current = prop;
    }
    
    // Skip optional comma
    if (parser_check(parser, TOKEN_COMMA)) {
      parser_advance(parser);
    }
    
    // Skip newlines
    while (parser_check(parser, TOKEN_NEWLINE)) {
      parser_advance(parser);
    }
  }
  
  return first;
}

// Parse a configuration value
ASTNode *parser_parse_config_value(Parser *parser) {
  ASTNode *node;
  
  if (parser_check(parser, TOKEN_STRING)) {
    Token *val = parser_advance(parser);
    if (parser->ctx && parser->ctx->parse_arena) {
      node = arena_alloc(parser->ctx->parse_arena, sizeof(ASTNode));
      node->data.config_value_string.value = arena_strdup(parser->ctx->parse_arena, val->value);
    } else {
      node = malloc(sizeof(ASTNode));
      node->data.config_value_string.value = strdup_safe(val->value);
    }
    node->type = AST_CONFIG_VALUE_STRING;
    return node;
  }
  
  if (parser_check(parser, TOKEN_NUMBER)) {
    Token *val = parser_advance(parser);
    if (parser->ctx && parser->ctx->parse_arena) {
      node = arena_alloc(parser->ctx->parse_arena, sizeof(ASTNode));
    } else {
      node = malloc(sizeof(ASTNode));
    }
    node->type = AST_CONFIG_VALUE_NUMBER;
    
    // Try to parse as float first, then as integer
    char *endptr;
    double double_val = strtod(val->value, &endptr);
    if (*endptr == '\0') {
      // Check if it's actually an integer
      long long_val = (long)double_val;
      if (double_val - long_val > -0.0000001 && double_val - long_val < 0.0000001) {
        node->data.config_value_number.value = (double)long_val;
        node->data.config_value_number.is_integer = true;
      } else {
        node->data.config_value_number.value = double_val;
        node->data.config_value_number.is_integer = false;
      }
    } else {
      node->data.config_value_number.value = atof(val->value);
      node->data.config_value_number.is_integer = true;
    }
    return node;
  }
  
  if (parser_check(parser, TOKEN_TRUE)) {
    parser_advance(parser);
    if (parser->ctx && parser->ctx->parse_arena) {
      node = arena_alloc(parser->ctx->parse_arena, sizeof(ASTNode));
    } else {
      node = malloc(sizeof(ASTNode));
    }
    node->type = AST_CONFIG_VALUE_BOOLEAN;
    node->data.config_value_boolean.value = true;
    return node;
  }
  
  if (parser_check(parser, TOKEN_FALSE)) {
    parser_advance(parser);
    if (parser->ctx && parser->ctx->parse_arena) {
      node = arena_alloc(parser->ctx->parse_arena, sizeof(ASTNode));
    } else {
      node = malloc(sizeof(ASTNode));
    }
    node->type = AST_CONFIG_VALUE_BOOLEAN;
    node->data.config_value_boolean.value = false;
    return node;
  }
  
  if (parser_check(parser, TOKEN_NULL)) {
    parser_advance(parser);
    if (parser->ctx && parser->ctx->parse_arena) {
      node = arena_alloc(parser->ctx->parse_arena, sizeof(ASTNode));
    } else {
      node = malloc(sizeof(ASTNode));
    }
    node->type = AST_CONFIG_VALUE_NULL;
    return node;
  }
  
  if (parser_check(parser, TOKEN_DOLLAR)) {
    // Handle $VAR || "default" syntax
    parser_advance(parser); // Skip '$'
    
    if (!parser_check(parser, TOKEN_IDENTIFIER)) {
      fprintf(stderr, "Expected identifier after '$'\n");
      return NULL;
    }
    
    Token *env_var = parser_advance(parser);
    char *default_value = NULL;
    
    // Check for default value with || operator
    if (parser_check(parser, TOKEN_OR)) {
      parser_advance(parser); // Skip '||'
      if (!parser_check(parser, TOKEN_STRING)) {
        fprintf(stderr, "Expected string for env default value after '||'\n");
        return NULL;
      }
      Token *default_val = parser_advance(parser);
      default_value = default_val->value;
    }
    
    if (parser->ctx && parser->ctx->parse_arena) {
      node = arena_alloc(parser->ctx->parse_arena, sizeof(ASTNode));
      node->data.config_value_env_call.env_var = arena_strdup(parser->ctx->parse_arena, env_var->value);
      node->data.config_value_env_call.default_value = default_value ? arena_strdup(parser->ctx->parse_arena, default_value) : NULL;
    } else {
      node = malloc(sizeof(ASTNode));
      node->data.config_value_env_call.env_var = strdup_safe(env_var->value);
      node->data.config_value_env_call.default_value = default_value ? strdup_safe(default_value) : NULL;
    }
    node->type = AST_CONFIG_VALUE_ENV_CALL;
    return node;
  }
  
  return NULL;
}

// Convert configuration AST to JSON
json_t *config_ast_to_json(ASTNode *node) {
  if (!node) return NULL;
  
  switch (node->type) {
    case AST_CONFIG_VALUE_STRING:
      return json_string(node->data.config_value_string.value);
    
    case AST_CONFIG_VALUE_NUMBER:
      if (node->data.config_value_number.is_integer) {
        return json_integer((json_int_t)node->data.config_value_number.value);
      } else {
        return json_real(node->data.config_value_number.value);
      }
    
    case AST_CONFIG_VALUE_BOOLEAN:
      return node->data.config_value_boolean.value ? json_true() : json_false();
    
    case AST_CONFIG_VALUE_NULL:
      return json_null();
    
    case AST_CONFIG_VALUE_ENV_CALL:
      {
        char *env_value = getenv(node->data.config_value_env_call.env_var);
        if (env_value) {
          return json_string(env_value);
        } else if (node->data.config_value_env_call.default_value) {
          return json_string(node->data.config_value_env_call.default_value);
        } else {
          return json_string("");
        }
      }
    
    case AST_CONFIG_VALUE_OBJECT:
      {
        json_t *obj = json_object();
        ConfigProperty *prop = node->data.config_value_object.properties;
        while (prop) {
          json_t *value = config_ast_to_json(prop->value);
          json_object_set_new(obj, prop->key, value);
          prop = prop->next;
        }
        return obj;
      }
    
    case AST_CONFIG_VALUE_ARRAY:
      {
        json_t *arr = json_array();
        ConfigArrayItem *item = node->data.config_value_array.items;
        while (item) {
          json_t *value = config_ast_to_json(item->value);
          json_array_append_new(arr, value);
          item = item->next;
        }
        return arr;
      }
    
    case AST_PROGRAM:
    case AST_ROUTE_DEFINITION:
    case AST_PIPELINE_STEP:
    case AST_VARIABLE_ASSIGNMENT:
    case AST_PIPELINE_DEFINITION:
    case AST_RESULT_STEP:
    case AST_CONFIG_BLOCK:
    case AST_DESCRIBE_BLOCK:
    case AST_IT_BLOCK:
    case AST_MOCK_CONFIG:
    case AST_TEST_EXECUTION:
    case AST_TEST_ASSERTION:
      // These node types are not configuration values
      return NULL;
  }
  
  return NULL;
}

// Convert configuration block to JSON
json_t *config_block_to_json(ASTNode *config_block) {
  if (!config_block || config_block->type != AST_CONFIG_BLOCK) {
    return NULL;
  }
  
  json_t *obj = json_object();
  ConfigProperty *prop = config_block->data.config_block.properties;
  
  while (prop) {
    json_t *value = config_ast_to_json(prop->value);
    json_object_set_new(obj, prop->key, value);
    prop = prop->next;
  }
  
  return obj;
}

ASTNode *parser_parse_statement(Parser *parser) {
  // Check for test blocks
  if (parser_check(parser, TOKEN_DESCRIBE)) {
    return parser_parse_describe_block(parser);
  }

  if (parser_check(parser, TOKEN_HTTP_METHOD)) {
    return parser_parse_route_definition(parser);
  }

  // Check for configuration block
  if (parser_check(parser, TOKEN_CONFIG)) {
    return parser_parse_config_block(parser);
  }

  // Check for pipeline definition: pipeline identifier = 
  if (parser_check(parser, TOKEN_IDENTIFIER)) {
    Token *first_token = parser_peek(parser);
    if (strcmp(first_token->value, "pipeline") == 0) {
      int saved = parser->current;
      parser_advance(parser); // pipeline
      if (parser_check(parser, TOKEN_IDENTIFIER)) {
        parser_advance(parser); // name
        if (parser_check(parser, TOKEN_EQUALS)) {
          parser->current = saved;
          return parser_parse_pipeline_definition(parser);
        }
      }
      parser->current = saved;
    } else {
      // Check for variable assignment: middleware identifier = 
      int saved = parser->current;
      parser_advance(parser); // middleware
      if (parser_check(parser, TOKEN_IDENTIFIER)) {
        parser_advance(parser); // name
        if (parser_check(parser, TOKEN_EQUALS)) {
          parser->current = saved;
          return parser_parse_variable_assignment(parser);
        }
      }
      parser->current = saved;
    }
  }

  // If we can't parse a statement, skip the current token to prevent infinite loops
  if (!parser_is_at_end(parser)) {
    fprintf(stderr, "Unexpected token: %s\n", parser_peek(parser)->value);
    parser_advance(parser);
  }

  return NULL;
}

ASTNode *parser_parse(Parser *parser) {
  ASTNode *program;
  if (parser->ctx && parser->ctx->parse_arena) {
    program = arena_alloc(parser->ctx->parse_arena, sizeof(ASTNode));
    program->data.program.statements = arena_alloc(parser->ctx->parse_arena, sizeof(ASTNode *) * 100);
  } else {
    program = malloc(sizeof(ASTNode));
    program->data.program.statements = malloc(sizeof(ASTNode *) * 100);
  }
  program->type = AST_PROGRAM;
  program->data.program.statement_count = 0;

  while (!parser_is_at_end(parser)) {
    if (parser_match(parser, TOKEN_NEWLINE) || parser_match(parser, TOKEN_COMMENT))
      continue;

    ASTNode *stmt = parser_parse_statement(parser);
    if (stmt) {
      program->data.program
          .statements[program->data.program.statement_count++] = stmt;
    }
  }

  return program;
}

// Stringify functions
void stringify_indent(FILE *out, int level) {
  for (int i = 0; i < level * 2; i++) {
    fprintf(out, " ");
  }
}

void stringify_pipeline(FILE *out, PipelineStep *pipeline, int level) {
  PipelineStep *step = pipeline;
  while (step) {
    stringify_indent(out, level);
    fprintf(out, "|> %s: ", step->middleware);
    if (step->is_variable) {
      fprintf(out, "%s", step->value);
    } else {
      fprintf(out, "`%s`", step->value);
    }
    fprintf(out, "\n");
    step = step->next;
  }
}

void stringify_node(FILE *out, ASTNode *node, int level) {
  if (!node)
    return;

  switch (node->type) {
  case AST_PROGRAM:
    for (int i = 0; i < node->data.program.statement_count; i++) {
      stringify_node(out, node->data.program.statements[i], level);
      if (i < node->data.program.statement_count - 1) {
        fprintf(out, "\n");
      }
    }
    break;

  case AST_ROUTE_DEFINITION:
    fprintf(out, "%s %s\n", node->data.route_def.method,
            node->data.route_def.route);
    stringify_pipeline(out, node->data.route_def.pipeline, 1);
    break;

  case AST_VARIABLE_ASSIGNMENT:
    fprintf(out, "%s %s = `%s`\n", node->data.var_assign.middleware,
            node->data.var_assign.name, node->data.var_assign.value);
    break;

  case AST_PIPELINE_DEFINITION:
    fprintf(out, "pipeline %s = \n", node->data.pipeline_def.name);
    stringify_pipeline(out, node->data.pipeline_def.pipeline, 1);
    break;

  case AST_PIPELINE_STEP:
    // This case is not used in the current implementation
    break;

  case AST_RESULT_STEP:
    // This case is not used in the current implementation
    break;

  case AST_CONFIG_BLOCK:
    fprintf(out, "config %s {\n", node->data.config_block.name);
    // TODO: Format configuration properties nicely
    ConfigProperty *prop = node->data.config_block.properties;
    while (prop) {
      fprintf(out, "  %s: ", prop->key);
      stringify_node(out, prop->value, level + 1);
      prop = prop->next;
    }
    fprintf(out, "}\n");
    break;

  case AST_CONFIG_VALUE_STRING:
    fprintf(out, "\"%s\"", node->data.config_value_string.value);
    break;

  case AST_CONFIG_VALUE_NUMBER:
    if (node->data.config_value_number.is_integer) {
      fprintf(out, "%.0f", node->data.config_value_number.value);
    } else {
      fprintf(out, "%g", node->data.config_value_number.value);
    }
    break;

  case AST_CONFIG_VALUE_BOOLEAN:
    fprintf(out, "%s", node->data.config_value_boolean.value ? "true" : "false");
    break;

  case AST_CONFIG_VALUE_NULL:
    fprintf(out, "null");
    break;

  case AST_CONFIG_VALUE_ENV_CALL:
    fprintf(out, "$%s", node->data.config_value_env_call.env_var);
    if (node->data.config_value_env_call.default_value) {
      fprintf(out, " || \"%s\"", node->data.config_value_env_call.default_value);
    }
    break;

  case AST_CONFIG_VALUE_OBJECT:
    fprintf(out, "{\n");
    ConfigProperty *obj_prop = node->data.config_value_object.properties;
    while (obj_prop) {
      stringify_indent(out, level + 1);
      fprintf(out, "%s: ", obj_prop->key);
      stringify_node(out, obj_prop->value, level + 1);
      obj_prop = obj_prop->next;
      if (obj_prop) fprintf(out, ",");
      fprintf(out, "\n");
    }
    stringify_indent(out, level);
    fprintf(out, "}");
    break;

  case AST_CONFIG_VALUE_ARRAY:
    fprintf(out, "[");
    ConfigArrayItem *item = node->data.config_value_array.items;
    while (item) {
      stringify_node(out, item->value, level);
      item = item->next;
      if (item) fprintf(out, ", ");
    }
    fprintf(out, "]");
    break;

  case AST_DESCRIBE_BLOCK:
  case AST_IT_BLOCK:
  case AST_MOCK_CONFIG:
  case AST_TEST_EXECUTION:
  case AST_TEST_ASSERTION:
    // Test nodes - minimal stringification
    fprintf(out, "[test node]");
    break;
  }
}

// Memory cleanup
void free_pipeline(PipelineStep *pipeline) {
  while (pipeline) {
    PipelineStep *next = pipeline->next;
    
    // Handle result steps specially - value is an ASTNode*, not a malloc'd string
    // Check middleware name BEFORE freeing it
    if (pipeline->middleware && strcmp(pipeline->middleware, "result") == 0) {
      free_ast((ASTNode *)(uintptr_t)(pipeline->value));
    } else {
      free(pipeline->value);
    }
    
    free(pipeline->middleware);
    free(pipeline);
    pipeline = next;
  }
}

void free_result_conditions(ResultCondition *conditions) {
  while (conditions) {
    ResultCondition *next = conditions->next;
    free(conditions->condition_name);
    free_pipeline(conditions->pipeline);
    free(conditions);
    conditions = next;
  }
}

void free_ast(ASTNode *node) {
  if (!node)
    return;

  switch (node->type) {
  case AST_PROGRAM:
    for (int i = 0; i < node->data.program.statement_count; i++) {
      free_ast(node->data.program.statements[i]);
    }
    free(node->data.program.statements);
    break;

  case AST_ROUTE_DEFINITION:
    free(node->data.route_def.method);
    free(node->data.route_def.route);
    free_pipeline(node->data.route_def.pipeline);
    break;

  case AST_VARIABLE_ASSIGNMENT:
    free(node->data.var_assign.middleware);
    free(node->data.var_assign.name);
    free(node->data.var_assign.value);
    break;

  case AST_PIPELINE_DEFINITION:
    free(node->data.pipeline_def.name);
    free_pipeline(node->data.pipeline_def.pipeline);
    break;

  case AST_RESULT_STEP:
    free_result_conditions(node->data.result_step.conditions);
    break;

  case AST_PIPELINE_STEP:
    // This case is not used in the current implementation
    break;

  case AST_CONFIG_BLOCK:
    free(node->data.config_block.name);
    // Free properties
    ConfigProperty *prop = node->data.config_block.properties;
    while (prop) {
      ConfigProperty *next = prop->next;
      free(prop->key);
      free_ast(prop->value);
      free(prop);
      prop = next;
    }
    break;

  case AST_CONFIG_VALUE_STRING:
    free(node->data.config_value_string.value);
    break;

  case AST_CONFIG_VALUE_NUMBER:
    // No dynamic memory to free
    break;

  case AST_CONFIG_VALUE_BOOLEAN:
    // No dynamic memory to free
    break;

  case AST_CONFIG_VALUE_NULL:
    // No dynamic memory to free
    break;

  case AST_CONFIG_VALUE_ENV_CALL:
    free(node->data.config_value_env_call.env_var);
    free(node->data.config_value_env_call.default_value);
    break;

  case AST_CONFIG_VALUE_OBJECT:
    {
      ConfigProperty *obj_prop = node->data.config_value_object.properties;
      while (obj_prop) {
        ConfigProperty *next = obj_prop->next;
        free(obj_prop->key);
        free_ast(obj_prop->value);
        free(obj_prop);
        obj_prop = next;
      }
    }
    break;

  case AST_CONFIG_VALUE_ARRAY:
    {
      ConfigArrayItem *item = node->data.config_value_array.items;
      while (item) {
        ConfigArrayItem *next = item->next;
        free_ast(item->value);
        free(item);
        item = next;
      }
    }
    break;

  case AST_DESCRIBE_BLOCK:
    free(node->data.describe_block.description);
    if (node->data.describe_block.mock_configs) {
      for (int i = 0; i < node->data.describe_block.mock_count; i++) {
        free_ast(node->data.describe_block.mock_configs[i]);
      }
      free(node->data.describe_block.mock_configs);
    }
    if (node->data.describe_block.tests) {
      for (int i = 0; i < node->data.describe_block.test_count; i++) {
        free_ast(node->data.describe_block.tests[i]);
      }
      free(node->data.describe_block.tests);
    }
    break;

  case AST_IT_BLOCK:
    free(node->data.it_block.description);
    if (node->data.it_block.execution) {
      free_ast(node->data.it_block.execution);
    }
    if (node->data.it_block.assertions) {
      for (int i = 0; i < node->data.it_block.assertion_count; i++) {
        free_ast(node->data.it_block.assertions[i]);
      }
      free(node->data.it_block.assertions);
    }
    if (node->data.it_block.inline_mocks) {
      for (int i = 0; i < node->data.it_block.inline_mock_count; i++) {
        free_ast(node->data.it_block.inline_mocks[i]);
      }
      free(node->data.it_block.inline_mocks);
    }
    break;

  case AST_MOCK_CONFIG:
    free(node->data.mock_config.middleware_name);
    free(node->data.mock_config.variable_name);
    free(node->data.mock_config.return_value);
    break;

  case AST_TEST_EXECUTION:
    switch (node->data.test_execution.type) {
      case TEST_EXEC_VARIABLE:
        free(node->data.test_execution.data.variable.middleware_type);
        free(node->data.test_execution.data.variable.variable_name);
        free(node->data.test_execution.data.variable.input_json);
        break;
      case TEST_EXEC_PIPELINE:
        free(node->data.test_execution.data.pipeline.pipeline_name);
        free(node->data.test_execution.data.pipeline.input_json);
        break;
      case TEST_EXEC_HTTP_CALL:
        free(node->data.test_execution.data.http_call.method);
        free(node->data.test_execution.data.http_call.path);
        break;
    }
    break;

  case AST_TEST_ASSERTION:
    switch (node->data.test_assertion.type) {
      case TEST_ASSERT_OUTPUT_EQUALS:
        free(node->data.test_assertion.data.output_equals.expected_json);
        break;
      case TEST_ASSERT_STATUS_IS:
        // No dynamic memory to free
        break;
    }
    break;
  }

  free(node);
}

void free_tokens(Token *tokens, int count) {
  for (int i = 0; i < count; i++) {
    free(tokens[i].value);
  }
  free(tokens);
}

// Inline mock parsing function (for 'and mock' within it blocks)
ASTNode *parser_parse_mock_config_inline(Parser *parser) {
  if (!parser_match(parser, TOKEN_MOCK)) {
    return NULL;
  }

  if (!parser_check(parser, TOKEN_IDENTIFIER)) {
    fprintf(stderr, "Expected middleware name after 'mock'\n");
    return NULL;
  }

  ASTNode *node = malloc(sizeof(ASTNode));
  node->type = AST_MOCK_CONFIG;
  node->data.mock_config.middleware_name = strdup(parser_advance(parser)->value);
  node->data.mock_config.variable_name = NULL;

  // Check for optional variable name (middleware.variable)
  if (parser_check(parser, TOKEN_DOT)) {
    parser_advance(parser); // consume '.'
    if (parser_check(parser, TOKEN_IDENTIFIER)) {
      node->data.mock_config.variable_name = strdup(parser_advance(parser)->value);
    }
  }

  if (!parser_match(parser, TOKEN_RETURNING)) {
    fprintf(stderr, "Expected 'returning' in mock configuration\n");
    free(node->data.mock_config.middleware_name);
    free(node->data.mock_config.variable_name);
    free(node);
    return NULL;
  }

  if (!parser_check(parser, TOKEN_STRING)) {
    fprintf(stderr, "Expected JSON string after 'returning'\n");
    free(node->data.mock_config.middleware_name);
    free(node->data.mock_config.variable_name);
    free(node);
    return NULL;
  }

  node->data.mock_config.return_value = strdup(parser_advance(parser)->value);
  return node;
}

// Test parsing functions
ASTNode *parser_parse_describe_block(Parser *parser) {
  if (!parser_match(parser, TOKEN_DESCRIBE)) {
    return NULL;
  }

  if (!parser_check(parser, TOKEN_STRING)) {
    fprintf(stderr, "Expected description string after 'describe'\n");
    return NULL;
  }

  ASTNode *node = malloc(sizeof(ASTNode));
  node->type = AST_DESCRIBE_BLOCK;
  node->data.describe_block.description = strdup(parser_advance(parser)->value);
  node->data.describe_block.mock_configs = NULL;
  node->data.describe_block.mock_count = 0;
  node->data.describe_block.tests = NULL;
  node->data.describe_block.test_count = 0;

  // Skip newlines and comments
  while (parser_match(parser, TOKEN_NEWLINE) || parser_match(parser, TOKEN_COMMENT)) {
    // Skip
  }

  // Parse mock configurations and tests
  while (!parser_is_at_end(parser) && 
         (parser_check(parser, TOKEN_WITH) || parser_check(parser, TOKEN_IT))) {
    
    // Skip newlines and comments before each block
    while (parser_match(parser, TOKEN_NEWLINE) || parser_match(parser, TOKEN_COMMENT)) {
      // Skip
    }
    
    if (parser_check(parser, TOKEN_WITH)) {
      ASTNode *mock = parser_parse_mock_config(parser);
      if (mock) {
        // Expand mock_configs array (simplified - in production use arena allocator)
        node->data.describe_block.mock_configs = realloc(
          node->data.describe_block.mock_configs,
          sizeof(ASTNode*) * (size_t)(node->data.describe_block.mock_count + 1)
        );
        node->data.describe_block.mock_configs[node->data.describe_block.mock_count++] = mock;
      }
    } else if (parser_check(parser, TOKEN_IT)) {
      ASTNode *test = parser_parse_it_block(parser);
      if (test) {
        // Expand tests array (simplified - in production use arena allocator)
        node->data.describe_block.tests = realloc(
          node->data.describe_block.tests,
          sizeof(ASTNode*) * (size_t)(node->data.describe_block.test_count + 1)
        );
        node->data.describe_block.tests[node->data.describe_block.test_count++] = test;
      }
    }
    
    // Skip newlines and comments after each block
    while (parser_match(parser, TOKEN_NEWLINE) || parser_match(parser, TOKEN_COMMENT)) {
      // Skip
    }
  }

  return node;
}

ASTNode *parser_parse_it_block(Parser *parser) {
  if (!parser_match(parser, TOKEN_IT)) {
    return NULL;
  }

  if (!parser_check(parser, TOKEN_STRING)) {
    fprintf(stderr, "Expected description string after 'it'\n");
    return NULL;
  }

  ASTNode *node = malloc(sizeof(ASTNode));
  node->type = AST_IT_BLOCK;
  node->data.it_block.description = strdup(parser_advance(parser)->value);
  node->data.it_block.execution = NULL;
  node->data.it_block.assertions = NULL;
  node->data.it_block.assertion_count = 0;
  node->data.it_block.inline_mocks = NULL;
  node->data.it_block.inline_mock_count = 0;

  // Skip newlines and comments
  while (parser_match(parser, TOKEN_NEWLINE) || parser_match(parser, TOKEN_COMMENT)) {
    // Skip
  }

  // Parse execution and assertions
  if (parser_check(parser, TOKEN_WHEN)) {
    node->data.it_block.execution = parser_parse_test_execution(parser);
  }

  // Skip newlines between execution and potential input
  while (parser_match(parser, TOKEN_NEWLINE) || parser_match(parser, TOKEN_COMMENT)) {
    // Skip
  }

  // Handle optional 'with input' for execution
  if (parser_match(parser, TOKEN_WITH) && parser_match(parser, TOKEN_INPUT)) {
    if (parser_check(parser, TOKEN_STRING) && node->data.it_block.execution) {
      char *input_json = strdup(parser_advance(parser)->value);
      // Store input based on execution type
      if (node->data.it_block.execution->data.test_execution.type == TEST_EXEC_VARIABLE) {
        node->data.it_block.execution->data.test_execution.data.variable.input_json = input_json;
      } else if (node->data.it_block.execution->data.test_execution.type == TEST_EXEC_PIPELINE) {
        node->data.it_block.execution->data.test_execution.data.pipeline.input_json = input_json;
      } else {
        free(input_json); // HTTP calls don't use input JSON
      }
    }
  }

  // Skip newlines between execution and potential inline mocks
  while (parser_match(parser, TOKEN_NEWLINE) || parser_match(parser, TOKEN_COMMENT)) {
    // Skip
  }

  // Handle inline mocks with 'and mock'
  while (parser_check(parser, TOKEN_AND) && 
         parser->current + 1 < parser->token_count &&
         parser->tokens[parser->current + 1].type == TOKEN_MOCK) {
    
    parser_advance(parser); // consume 'and'
    ASTNode *inline_mock = parser_parse_mock_config_inline(parser);
    if (inline_mock) {
      // Store inline mock in it_block
      node->data.it_block.inline_mocks = realloc(
          node->data.it_block.inline_mocks,
          sizeof(ASTNode*) * (size_t)(node->data.it_block.inline_mock_count + 1)
      );
      node->data.it_block.inline_mocks[node->data.it_block.inline_mock_count] = inline_mock;
      node->data.it_block.inline_mock_count++;
      
      // Skip newlines after mock
      while (parser_match(parser, TOKEN_NEWLINE) || parser_match(parser, TOKEN_COMMENT)) {
        // Skip
      }
    }
  }

  // Skip newlines between mocks and assertions
  while (parser_match(parser, TOKEN_NEWLINE) || parser_match(parser, TOKEN_COMMENT)) {
    // Skip
  }

  while (parser_check(parser, TOKEN_THEN) || parser_check(parser, TOKEN_AND)) {
    ASTNode *assertion = parser_parse_test_assertion(parser);
    if (assertion) {
      // Expand assertions array (simplified - in production use arena allocator)
      node->data.it_block.assertions = realloc(
        node->data.it_block.assertions,
        sizeof(ASTNode*) * (size_t)(node->data.it_block.assertion_count + 1)
      );
      node->data.it_block.assertions[node->data.it_block.assertion_count++] = assertion;
    }
    
    // Skip newlines between assertions
    while (parser_match(parser, TOKEN_NEWLINE) || parser_match(parser, TOKEN_COMMENT)) {
      // Skip
    }
  }

  return node;
}

ASTNode *parser_parse_mock_config(Parser *parser) {
  if (!parser_match(parser, TOKEN_WITH)) {
    return NULL;
  }

  if (!parser_match(parser, TOKEN_MOCK)) {
    fprintf(stderr, "Expected 'mock' after 'with'\n");
    return NULL;
  }

  if (!parser_check(parser, TOKEN_IDENTIFIER)) {
    fprintf(stderr, "Expected middleware name after 'mock'\n");
    return NULL;
  }

  ASTNode *node = malloc(sizeof(ASTNode));
  node->type = AST_MOCK_CONFIG;
  node->data.mock_config.middleware_name = strdup(parser_advance(parser)->value);
  node->data.mock_config.variable_name = NULL;

  // Check for optional variable name (middleware.variable)
  if (parser_check(parser, TOKEN_DOT)) {
    parser_advance(parser); // consume '.'
    if (parser_check(parser, TOKEN_IDENTIFIER)) {
      node->data.mock_config.variable_name = strdup(parser_advance(parser)->value);
    }
  }

  if (!parser_match(parser, TOKEN_RETURNING)) {
    fprintf(stderr, "Expected 'returning' in mock configuration\n");
    free(node->data.mock_config.middleware_name);
    free(node->data.mock_config.variable_name);
    free(node);
    return NULL;
  }

  if (!parser_check(parser, TOKEN_STRING)) {
    fprintf(stderr, "Expected JSON string after 'returning'\n");
    free(node->data.mock_config.middleware_name);
    free(node->data.mock_config.variable_name);
    free(node);
    return NULL;
  }

  node->data.mock_config.return_value = strdup(parser_advance(parser)->value);
  return node;
}

ASTNode *parser_parse_test_execution(Parser *parser) {
  if (!parser_match(parser, TOKEN_WHEN)) {
    return NULL;
  }

  ASTNode *node = malloc(sizeof(ASTNode));
  node->type = AST_TEST_EXECUTION;

  if (parser_match(parser, TOKEN_EXECUTING)) {
    if (parser_match(parser, TOKEN_VARIABLE)) {
      node->data.test_execution.type = TEST_EXEC_VARIABLE;
      
      if (!parser_check(parser, TOKEN_IDENTIFIER)) {
        fprintf(stderr, "Expected middleware type after 'variable'\n");
        free(node);
        return NULL;
      }
      node->data.test_execution.data.variable.middleware_type = strdup(parser_advance(parser)->value);
      
      if (!parser_check(parser, TOKEN_IDENTIFIER)) {
        fprintf(stderr, "Expected variable name\n");
        free(node->data.test_execution.data.variable.middleware_type);
        free(node);
        return NULL;
      }
      node->data.test_execution.data.variable.variable_name = strdup(parser_advance(parser)->value);
      node->data.test_execution.data.variable.input_json = NULL;
      
    } else if (parser_check(parser, TOKEN_IDENTIFIER) && 
               strcmp(parser_peek(parser)->value, "pipeline") == 0) {
      parser_advance(parser); // consume 'pipeline'
      node->data.test_execution.type = TEST_EXEC_PIPELINE;
      
      if (!parser_check(parser, TOKEN_IDENTIFIER)) {
        fprintf(stderr, "Expected pipeline name\n");
        free(node);
        return NULL;
      }
      node->data.test_execution.data.pipeline.pipeline_name = strdup(parser_advance(parser)->value);
      node->data.test_execution.data.pipeline.input_json = NULL;
      
    } else {
      fprintf(stderr, "Expected 'variable' or 'pipeline' after 'executing'\n");
      free(node);
      return NULL;
    }
  } else if (parser_match(parser, TOKEN_CALLING)) {
    node->data.test_execution.type = TEST_EXEC_HTTP_CALL;
    
    if (!parser_check(parser, TOKEN_HTTP_METHOD)) {
      fprintf(stderr, "Expected HTTP method\n");
      free(node);
      return NULL;
    }
    node->data.test_execution.data.http_call.method = strdup(parser_advance(parser)->value);
    
    if (!parser_check(parser, TOKEN_ROUTE)) {
      fprintf(stderr, "Expected route path\n");
      free(node->data.test_execution.data.http_call.method);
      free(node);
      return NULL;
    }
    node->data.test_execution.data.http_call.path = strdup(parser_advance(parser)->value);
    
  } else {
    fprintf(stderr, "Expected 'executing' or 'calling' after 'when'\n");
    free(node);
    return NULL;
  }

  return node;
}

ASTNode *parser_parse_test_assertion(Parser *parser) {
  if (!parser_match(parser, TOKEN_THEN) && !parser_match(parser, TOKEN_AND)) {
    return NULL;
  }

  ASTNode *node = malloc(sizeof(ASTNode));
  node->type = AST_TEST_ASSERTION;

  if (parser_match(parser, TOKEN_OUTPUT) && parser_match(parser, TOKEN_EQUALS_ASSERTION)) {
    node->data.test_assertion.type = TEST_ASSERT_OUTPUT_EQUALS;
    
    if (!parser_check(parser, TOKEN_STRING)) {
      fprintf(stderr, "Expected JSON string after 'output equals'\n");
      free(node);
      return NULL;
    }
    node->data.test_assertion.data.output_equals.expected_json = strdup(parser_advance(parser)->value);
    
  } else if (parser_match(parser, TOKEN_STATUS) && parser_match(parser, TOKEN_IS)) {
    node->data.test_assertion.type = TEST_ASSERT_STATUS_IS;
    
    if (!parser_check(parser, TOKEN_NUMBER)) {
      fprintf(stderr, "Expected status code number after 'status is'\n");
      free(node);
      return NULL;
    }
    node->data.test_assertion.data.status_is.expected_status = atoi(parser_advance(parser)->value);
    
  } else {
    fprintf(stderr, "Expected 'output equals' or 'status is' in assertion\n");
    free(node);
    return NULL;
  }

  return node;
} 
