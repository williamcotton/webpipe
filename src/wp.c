#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "wp.h"

// Main function
int main(int argc, char *argv[]) {
  if (argc < 2) {
    fprintf(stderr, "Usage: %s <wp_file> [options]\n", argv[0]);
    fprintf(stderr, "  or:  %s -f <wp_file> (parse only)\n", argv[0]);
    return 1;
  }

  // Check if this is a parse-only run
  if (argc == 3 && strcmp(argv[1], "-f") == 0) {
    // Parse-only mode
    FILE *file = fopen(argv[2], "r");
    if (!file) {
      fprintf(stderr, "Error: Could not open file '%s'\n", argv[2]);
      return 1;
    }

    fseek(file, 0, SEEK_END);
    long file_size = ftell(file);
    fseek(file, 0, SEEK_SET);

    char *source = malloc((size_t)file_size + 1);
    fread(source, 1, (size_t)file_size, file);
    source[file_size] = '\0';
    fclose(file);

    // Tokenize
    int token_count;
    Token *tokens = lexer_tokenize(source, &token_count);

    // Parse
    Parser *parser = parser_new(tokens, token_count);
    ASTNode *ast = parser_parse(parser);

    // Stringify and output
    stringify_node(stdout, ast, 0);

    // Cleanup
    parser_free(parser);
    free_ast(ast);
    free_tokens(tokens, token_count);
    free(source);

    return 0;
  }

  // Server mode - run the runtime
  if (wp_runtime_init(argv[1]) != 0) {
    return 1;
  }
  
  printf("WP Runtime started on port 8080\n");
  printf("Press Enter to stop...\n");
  getchar();
  
  wp_runtime_cleanup();
  return 0;
}
