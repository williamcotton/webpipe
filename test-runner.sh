#!/bin/bash

# Test runner script for the flow project

set -e

print_usage() {
    echo "Usage: $0 [all|unit|integration|system|leaks] [test_binaries...]"
    echo "  all          - Run all tests"
    echo "  unit         - Run unit tests only"
    echo "  integration  - Run integration tests only"
    echo "  system       - Run system tests only"
    echo "  leaks        - Run leak detection on all tests"
    exit 1
}

run_tests() {
    local test_type="$1"
    shift
    local tests=("$@")
    
    if [ ${#tests[@]} -eq 0 ]; then
        echo "No tests found for $test_type"
        return 0
    fi
    
    echo "Running $test_type tests..."
    local failed=0
    
    for test in "${tests[@]}"; do
        echo "Running $test"
        if ! "$test"; then
            echo "$test FAILED"
            failed=1
        fi
    done
    
    if [ $failed -eq 1 ]; then
        echo "Some $test_type tests failed!"
        exit 1
    else
        echo "All $test_type tests passed!"
    fi
}

run_leaks() {
    local tests=("$@")
    
    if [ ${#tests[@]} -eq 0 ]; then
        echo "No tests found for leak detection"
        return 0
    fi
    
    echo "Running leak detection on all tests..."
    local failed=0
    
    # Platform detection
    PLATFORM=$(uname -s | tr 'a-z' 'A-Z')
    
    for test in "${tests[@]}"; do
        echo "Checking leaks for $test"
        if [ "$PLATFORM" = "LINUX" ]; then
            if ! valgrind --tool=memcheck --leak-check=full --error-exitcode=1 --num-callers=30 -s "$test"; then
                echo "$test has memory leaks"
                failed=1
            fi
        elif [ "$PLATFORM" = "DARWIN" ]; then
            if ! leaks --atExit -- "$test"; then
                echo "$test has memory leaks"
                failed=1
            fi
        else
            echo "Unsupported platform: $PLATFORM"
            failed=1
        fi
    done
    
    if [ $failed -eq 1 ]; then
        echo "Some tests have memory leaks!"
        exit 1
    else
        echo "All tests are leak-free!"
    fi
}

if [ $# -eq 0 ]; then
    print_usage
fi

case "$1" in
    all)
        shift
        run_tests "all" "$@"
        ;;
    unit)
        shift
        run_tests "unit" "$@"
        ;;
    integration)
        shift
        run_tests "integration" "$@"
        ;;
    system)
        shift
        run_tests "system" "$@"
        ;;
    leaks)
        shift
        run_leaks "$@"
        ;;
    *)
        print_usage
        ;;
esac