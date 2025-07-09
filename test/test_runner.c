#include "unity/unity.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

// Test function declarations
void run_arena_tests(void);
void run_lexer_tests(void);
void run_parser_tests(void);
void run_plugin_tests(void);
void run_jq_tests(void);
void run_lua_tests(void);
void run_pg_tests(void);
void run_pipeline_tests(void);
void run_server_tests(void);
void run_e2e_tests(void);
void run_perf_tests(void);

int main(int argc, char *argv[]) {
    UNITY_BEGIN();
    
    if (argc > 1) {
        // Run specific test category
        if (strcmp(argv[1], "unit") == 0) {
            printf("Running unit tests...\n");
            run_arena_tests();
            run_lexer_tests();
            run_parser_tests();
            run_plugin_tests();
        } else if (strcmp(argv[1], "integration") == 0) {
            printf("Running integration tests...\n");
            run_jq_tests();
            run_lua_tests();
            run_pg_tests();
            run_pipeline_tests();
        } else if (strcmp(argv[1], "system") == 0) {
            printf("Running system tests...\n");
            run_server_tests();
            run_e2e_tests();
        } else if (strcmp(argv[1], "perf") == 0) {
            printf("Running performance tests...\n");
            run_perf_tests();
        } else if (strcmp(argv[1], "all") == 0) {
            printf("Running all tests...\n");
            run_arena_tests();
            run_lexer_tests();
            run_parser_tests();
            run_plugin_tests();
            run_jq_tests();
            run_lua_tests();
            run_pg_tests();
            run_pipeline_tests();
            run_server_tests();
            run_e2e_tests();
            run_perf_tests();
        } else {
            printf("Usage: %s [unit|integration|system|perf|all]\n", argv[0]);
            return 1;
        }
    } else {
        // Run all tests by default
        printf("Running all tests...\n");
        run_arena_tests();
        run_lexer_tests();
        run_parser_tests();
        run_plugin_tests();
        run_jq_tests();
        run_lua_tests();
        run_pg_tests();
        run_pipeline_tests();
        run_server_tests();
        run_e2e_tests();
        run_perf_tests();
    }
    
    return UNITY_END();
}