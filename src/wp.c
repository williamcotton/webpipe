#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <signal.h>
#include <unistd.h>
#include <libgen.h>
#include <getopt.h>
#include <sys/stat.h>
#include <time.h>
#include "wp.h"
#include "../deps/dotenv-c/dotenv.h"

#define MAX_PATH_LENGTH 4096
#define MAX_INCLUDES 100

// Execution modes
typedef enum {
    WP_MODE_INTERACTIVE,
    WP_MODE_DAEMON,
    WP_MODE_TEST,
    WP_MODE_TIMEOUT
} wp_execution_mode_t;

// File modification tracking
typedef struct {
    char filepath[MAX_PATH_LENGTH];
    time_t last_mod_time;
} FileModInfo;

typedef struct {
    FileModInfo files[MAX_INCLUDES];
    int count;
} ModificationTracker;

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

// File modification tracking functions
static time_t get_file_mod_time(const char* path) {
    struct stat attr;
    if (stat(path, &attr) == 0) {
        return attr.st_mtime;
    }
    return 0;
}

static void init_mod_tracker(ModificationTracker* tracker, const char* main_file) {
    tracker->count = 1;
    strncpy(tracker->files[0].filepath, main_file, MAX_PATH_LENGTH - 1);
    tracker->files[0].filepath[MAX_PATH_LENGTH - 1] = '\0';
    tracker->files[0].last_mod_time = 0;  // Initialize to 0 to force first load
}

static int check_modifications(ModificationTracker* tracker) {
    int modified = 0;
    for (int i = 0; i < tracker->count; i++) {
        time_t current_mod = get_file_mod_time(tracker->files[i].filepath);
        if (current_mod > tracker->files[i].last_mod_time) {
            printf("\nFile modified: %s\n", tracker->files[i].filepath);
            tracker->files[i].last_mod_time = current_mod;
            modified = 1;
        }
    }
    return modified;
}

// Initialize environment variables from .env file
static int initialize_environment(const char *wp_file_path) {
    // Get directory containing the WP file
    char *path_copy = strdup(wp_file_path);
    char *dir = dirname(path_copy);
    
    // Load .env file from same directory, don't overwrite system env vars
    int result = env_load(dir, false);
    
    free(path_copy);
    return result; // 0 on success, -1 if .env file not found (not an error)
}

// Print usage information
static void print_usage(const char *program_name) {
    fprintf(stderr, "Usage: %s <wp_file> [options]\n", program_name);
    fprintf(stderr, "\n");
    fprintf(stderr, "Options:\n");
    fprintf(stderr, "  --daemon         Run in daemon mode (background service)\n");
    fprintf(stderr, "  --test           Run in test mode (until SIGTERM)\n");
    fprintf(stderr, "  --timeout <sec>  Run for specified seconds then exit\n");
    fprintf(stderr, "  --port <num>     Port to listen on (default: 8080, env: WP_PORT)\n");
    fprintf(stderr, "  --watch          Enable file monitoring (default: enabled)\n");
    fprintf(stderr, "  --no-watch       Disable file monitoring\n");
    fprintf(stderr, "  --help           Show this help message\n");
    fprintf(stderr, "\n");
    fprintf(stderr, "Default mode is interactive (press Ctrl+C to stop)\n");
}

// Parse command line arguments
static wp_execution_mode_t parse_arguments(int argc, char *argv[], char **wp_file, int *timeout, int *port, int *watch_enabled) {
    static struct option long_options[] = {
        {"daemon", no_argument, 0, 'd'},
        {"test", no_argument, 0, 't'},
        {"timeout", required_argument, 0, 'T'},
        {"port", required_argument, 0, 'p'},
        {"watch", no_argument, 0, 'w'},
        {"no-watch", no_argument, 0, 'W'},
        {"help", no_argument, 0, 'h'},
        {0, 0, 0, 0}
    };

    *timeout = 0;
    *port = 8080; // Default port
    *watch_enabled = 1; // Default: enabled
    
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
    
    int option_index = 0;
    int c;
    
    while ((c = getopt_long(argc, argv, "dthT:p:wW", long_options, &option_index)) != -1) {
        switch (c) {
            case 'd':
                mode = WP_MODE_DAEMON;
                break;
            case 't':
                mode = WP_MODE_TEST;
                break;
            case 'T':
                *timeout = atoi(optarg);
                if (*timeout <= 0) {
                    fprintf(stderr, "Error: timeout must be a positive number\n");
                    exit(1);
                }
                mode = WP_MODE_TIMEOUT;
                break;
            case 'p':
                *port = atoi(optarg);
                if (*port <= 0 || *port > 65535) {
                    fprintf(stderr, "Error: port must be between 1 and 65535\n");
                    exit(1);
                }
                break;
            case 'w':
                *watch_enabled = 1;
                break;
            case 'W':
                *watch_enabled = 0;
                break;
            case 'h':
                print_usage(argv[0]);
                exit(0);
            case '?':
                exit(1);
            default:
                break;
        }
    }
    
    // Get the wp_file from remaining args
    if (optind >= argc) {
        fprintf(stderr, "Error: wp_file is required\n");
        print_usage(argv[0]);
        exit(1);
    }
    
    *wp_file = argv[optind];
    
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
static void wait_for_shutdown(wp_execution_mode_t mode, int port, int watch_enabled, ModificationTracker* mod_tracker, const char* wp_file) {
    (void)mode;
    printf("WP Runtime started on port %d\n", port);
    printf("Press Ctrl+C to stop...\n");
    if (watch_enabled) {
        printf("File monitoring enabled\n");
    }
    while (!shutdown_requested) {
        if (watch_enabled && check_modifications(mod_tracker)) {
            printf("Reloading server configuration...\n");
            // Hot reload will be implemented in Phase 3
            wp_runtime_cleanup();
            if (wp_runtime_init(wp_file, port) != 0) {
                fprintf(stderr, "Failed to reload server, shutting down\n");
                shutdown_requested = 1;
            } else {
                printf("Server reloaded successfully\n");
            }
        }
        usleep(100000); // 100ms sleep for responsive file monitoring
    }
    printf("Shutdown signal received, stopping...\n");
}

// Main function
int main(int argc, char *argv[]) {
    // Parse command line arguments
    char *wp_file;
    int port;
    int watch_enabled;
    wp_execution_mode_t mode = parse_arguments(argc, argv, &wp_file, &timeout_seconds, &port, &watch_enabled);
    
    // Set up signal handlers
    setup_signal_handlers(mode);
    
    // Initialize environment variables from .env file
    initialize_environment(wp_file);
    
    // Initialize file modification tracker
    ModificationTracker mod_tracker = {0};
    if (watch_enabled) {
        init_mod_tracker(&mod_tracker, wp_file);
    }
    
    // Server mode - run the runtime
    if (wp_runtime_init(wp_file, port) != 0) {
        return 1;
    }
    
    // Wait for shutdown based on mode
    wait_for_shutdown(mode, port, watch_enabled, &mod_tracker, wp_file);
    
    // Cleanup
    wp_runtime_cleanup();
    return 0;
}
