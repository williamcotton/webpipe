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
  while (parser_match(parser, TOKEN_NEWLINE)) {
  }
}

PipelineStep *parser_parse_pipeline(Parser *parser) {
  PipelineStep *head = NULL;
  PipelineStep *tail = NULL;

  while (parser_match(parser, TOKEN_PIPE)) {
    if (!parser_check(parser, TOKEN_IDENTIFIER)) {
      fprintf(stderr, "Expected plugin name after |>\n");
      return head;
    }

    Token *plugin = parser_advance(parser);

    // Check for result step
    if (strcmp(plugin->value, "result") == 0) {
      // This is a result step - parse result conditions
      if (!parser_match(parser, TOKEN_LBRACE)) {
        fprintf(stderr, "Expected { after result\n");
        return head;
      }

      ASTNode *result_node = parser_parse_result_step(parser);
      
      // Create a special pipeline step for result
      PipelineStep *step = malloc(sizeof(PipelineStep));
      step->plugin = strdup_safe("result");
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
      fprintf(stderr, "Expected : after plugin name\n");
      return head;
    }

    char *value = NULL;
    bool is_variable = false;

    if (parser_check(parser, TOKEN_STRING)) {
      value = strdup_safe(parser_advance(parser)->value);
      is_variable = false;
    } else if (parser_check(parser, TOKEN_IDENTIFIER)) {
      value = strdup_safe(parser_advance(parser)->value);
      is_variable = true;
    }

    PipelineStep *step = malloc(sizeof(PipelineStep));
    step->plugin = strdup_safe(plugin->value);
    step->value = value;
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
  ASTNode *node = malloc(sizeof(ASTNode));
  node->type = AST_RESULT_STEP;
  node->data.result_step.conditions = NULL;

  ResultCondition *head = NULL;
  ResultCondition *tail = NULL;

  while (!parser_check(parser, TOKEN_RBRACE) && !parser_is_at_end(parser)) {
    // Parse condition: name(status_code) { pipeline }
    if (!parser_check(parser, TOKEN_IDENTIFIER)) {
      fprintf(stderr, "Expected condition name\n");
      break;
    }

    Token *condition_name = parser_advance(parser);

    if (!parser_match(parser, TOKEN_LPAREN)) {
      fprintf(stderr, "Expected ( after condition name\n");
      break;
    }

    if (!parser_check(parser, TOKEN_NUMBER)) {
      fprintf(stderr, "Expected status code number\n");
      break;
    }

    Token *status_code = parser_advance(parser);

    if (!parser_match(parser, TOKEN_RPAREN)) {
      fprintf(stderr, "Expected ) after status code\n");
      break;
    }

    if (!parser_match(parser, TOKEN_LBRACE)) {
      fprintf(stderr, "Expected { after condition\n");
      break;
    }

    parser_consume_newlines(parser);

    // Parse pipeline for this condition
    PipelineStep *pipeline = parser_parse_pipeline(parser);

    if (!parser_match(parser, TOKEN_RBRACE)) {
      fprintf(stderr, "Expected } after condition pipeline\n");
      break;
    }

    ResultCondition *condition = malloc(sizeof(ResultCondition));
    condition->condition_name = strdup_safe(condition_name->value);
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

  if (!parser_match(parser, TOKEN_RBRACE)) {
    fprintf(stderr, "Expected } at end of result step\n");
  }

  return node;
}

ASTNode *parser_parse_route_definition(Parser *parser) {
  Token *method = parser_advance(parser);

  if (!parser_check(parser, TOKEN_ROUTE)) {
    fprintf(stderr, "Expected route after HTTP method\n");
    return NULL;
  }

  Token *route = parser_advance(parser);
  parser_consume_newlines(parser);

  PipelineStep *pipeline = parser_parse_pipeline(parser);

  ASTNode *node = malloc(sizeof(ASTNode));
  node->type = AST_ROUTE_DEFINITION;
  node->data.route_def.method = strdup_safe(method->value);
  node->data.route_def.route = strdup_safe(route->value);
  node->data.route_def.pipeline = pipeline;

  return node;
}

ASTNode *parser_parse_variable_assignment(Parser *parser) {
  Token *plugin = parser_advance(parser);
  Token *name = parser_advance(parser);

  if (!parser_match(parser, TOKEN_EQUALS)) {
    fprintf(stderr, "Expected = in variable assignment\n");
    return NULL;
  }

  if (!parser_check(parser, TOKEN_STRING)) {
    fprintf(stderr, "Expected string value in variable assignment\n");
    return NULL;
  }

  Token *value = parser_advance(parser);

  ASTNode *node = malloc(sizeof(ASTNode));
  node->type = AST_VARIABLE_ASSIGNMENT;
  node->data.var_assign.plugin = strdup_safe(plugin->value);
  node->data.var_assign.name = strdup_safe(name->value);
  node->data.var_assign.value = strdup_safe(value->value);

  return node;
}

ASTNode *parser_parse_statement(Parser *parser) {
  if (parser_check(parser, TOKEN_HTTP_METHOD)) {
    return parser_parse_route_definition(parser);
  }

  // Check for variable assignment
  if (parser_check(parser, TOKEN_IDENTIFIER)) {
    int saved = parser->current;
    parser_advance(parser); // plugin
    if (parser_check(parser, TOKEN_IDENTIFIER)) {
      parser_advance(parser); // name
      if (parser_check(parser, TOKEN_EQUALS)) {
        parser->current = saved;
        return parser_parse_variable_assignment(parser);
      }
    }
    parser->current = saved;
  }

  return NULL;
}

ASTNode *parser_parse(Parser *parser) {
  ASTNode *program = malloc(sizeof(ASTNode));
  program->type = AST_PROGRAM;
  program->data.program.statements = malloc(sizeof(ASTNode *) * 100);
  program->data.program.statement_count = 0;

  while (!parser_is_at_end(parser)) {
    if (parser_match(parser, TOKEN_NEWLINE))
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
    fprintf(out, "|> %s: ", step->plugin);
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
    fprintf(out, "%s %s = `%s`\n", node->data.var_assign.plugin,
            node->data.var_assign.name, node->data.var_assign.value);
    break;

  case AST_PIPELINE_STEP:
    // This case is not used in the current implementation
    break;

  case AST_RESULT_STEP:
    // This case is not used in the current implementation
    break;
  }
}

// Memory cleanup
void free_pipeline(PipelineStep *pipeline) {
  while (pipeline) {
    PipelineStep *next = pipeline->next;
    free(pipeline->plugin);
    free(pipeline->value);
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
    free(node->data.var_assign.plugin);
    free(node->data.var_assign.name);
    free(node->data.var_assign.value);
    break;

  case AST_RESULT_STEP:
    free_result_conditions(node->data.result_step.conditions);
    break;

  case AST_PIPELINE_STEP:
    // This case is not used in the current implementation
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
