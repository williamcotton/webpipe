#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <signal.h>
#include <unistd.h>
#include "wp.h"

// Execution modes
typedef enum {
    WP_MODE_INTERACTIVE,
    WP_MODE_DAEMON,
    WP_MODE_TEST,
    WP_MODE_TIMEOUT
} wp_execution_mode_t;

// Global variables for signal handling
static volatile int shutdown_requested = 0;
static int timeout_seconds = 0;

// Signal handler for graceful shutdown
static void signal_handler(int signum) {
    (void)signum; // Suppress unused parameter warning
    shutdown_requested = 1;
}

// Timeout handler
static void timeout_handler(int signum) {
    (void)signum; // Suppress unused parameter warning
    printf("Timeout reached, shutting down...\n");
    shutdown_requested = 1;
}

// Print usage information
static void print_usage(const char *program_name) {
    fprintf(stderr, "Usage: %s <wp_file> [options]\n", program_name);
    fprintf(stderr, "  or:  %s -f <wp_file> (parse only)\n", program_name);
    fprintf(stderr, "\n");
    fprintf(stderr, "Options:\n");
    fprintf(stderr, "  --daemon         Run in daemon mode (background service)\n");
    fprintf(stderr, "  --test           Run in test mode (until SIGTERM)\n");
    fprintf(stderr, "  --timeout <sec>  Run for specified seconds then exit\n");
    fprintf(stderr, "  --port <num>     Port to listen on (default: 8080, env: WP_PORT)\n");
    fprintf(stderr, "  -f <wp_file>     Parse only (don't start server)\n");
    fprintf(stderr, "\n");
    fprintf(stderr, "Default mode is interactive (press Enter to stop)\n");
}

// Parse command line arguments
static wp_execution_mode_t parse_arguments(int argc, char *argv[], char **wp_file, int *timeout, int *port) {
    if (argc < 2) {
        print_usage(argv[0]);
        exit(1);
    }

    *wp_file = argv[1];
    *timeout = 0;
    *port = 8080; // Default port
    
    // Check for environment variable
    const char *env_port = getenv("WP_PORT");
    if (env_port) {
        int env_port_num = atoi(env_port);
        if (env_port_num > 0 && env_port_num <= 65535) {
            *port = env_port_num;
        }
    }
    
    // Default to interactive mode
    wp_execution_mode_t mode = WP_MODE_INTERACTIVE;
    
    // Parse options starting from argv[2]
    for (int i = 2; i < argc; i++) {
        if (strcmp(argv[i], "--daemon") == 0) {
            mode = WP_MODE_DAEMON;
        } else if (strcmp(argv[i], "--test") == 0) {
            mode = WP_MODE_TEST;
        } else if (strcmp(argv[i], "--timeout") == 0) {
            if (i + 1 >= argc) {
                fprintf(stderr, "Error: --timeout requires a number of seconds\n");
                exit(1);
            }
            *timeout = atoi(argv[i + 1]);
            if (*timeout <= 0) {
                fprintf(stderr, "Error: timeout must be a positive number\n");
                exit(1);
            }
            mode = WP_MODE_TIMEOUT;
            i++; // Skip the timeout value
        } else if (strcmp(argv[i], "--port") == 0) {
            if (i + 1 >= argc) {
                fprintf(stderr, "Error: --port requires a port number\n");
                exit(1);
            }
            *port = atoi(argv[i + 1]);
            if (*port <= 0 || *port > 65535) {
                fprintf(stderr, "Error: port must be between 1 and 65535\n");
                exit(1);
            }
            i++; // Skip the port value
        } else {
            fprintf(stderr, "Error: Unknown option '%s'\n", argv[i]);
            print_usage(argv[0]);
            exit(1);
        }
    }
    
    return mode;
}

// Set up signal handlers based on execution mode
static void setup_signal_handlers(wp_execution_mode_t mode) {
    // Always handle SIGTERM and SIGINT for graceful shutdown
    signal(SIGTERM, signal_handler);
    signal(SIGINT, signal_handler);
    
    // For timeout mode, also set up SIGALRM
    if (mode == WP_MODE_TIMEOUT) {
        signal(SIGALRM, timeout_handler);
        alarm((unsigned int)timeout_seconds);
    }
}

// Wait for shutdown based on execution mode
static void wait_for_shutdown(wp_execution_mode_t mode, int port) {
    switch (mode) {
        case WP_MODE_INTERACTIVE:
            printf("WP Runtime started on port %d\n", port);
            printf("Press Enter to stop...\n");
            getchar();
            break;
            
        case WP_MODE_DAEMON:
            printf("WP Runtime started in daemon mode on port %d\n", port);
            while (!shutdown_requested) {
                sleep(1);
            }
            printf("Shutdown signal received, stopping...\n");
            break;
            
        case WP_MODE_TEST:
            printf("WP Runtime started in test mode on port %d\n", port);
            while (!shutdown_requested) {
                sleep(1);
            }
            break;
            
        case WP_MODE_TIMEOUT:
            printf("WP Runtime started with %d second timeout on port %d\n", timeout_seconds, port);
            while (!shutdown_requested) {
                sleep(1);
            }
            break;
    }
}

// Main function
int main(int argc, char *argv[]) {
    // Check if this is a parse-only run (legacy support)
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

        if (file_size <= 0) {
            fprintf(stderr, "Error: File '%s' is empty or invalid\n", argv[2]);
            fclose(file);
            return -1;
        }

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

    // Parse command line arguments
    char *wp_file;
    int port;
    wp_execution_mode_t mode = parse_arguments(argc, argv, &wp_file, &timeout_seconds, &port);
    
    // Set up signal handlers
    setup_signal_handlers(mode);
    
    // Server mode - run the runtime
    if (wp_runtime_init(wp_file, port) != 0) {
        return 1;
    }
    
    // Wait for shutdown based on mode
    wait_for_shutdown(mode, port);
    
    // Cleanup
    wp_runtime_cleanup();
    return 0;
}
