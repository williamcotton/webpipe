#include "../unity/unity.h"
#include "../helpers/test_utils.h"
#include "../../src/wp.h"
#include <string.h>

void setUp(void) {
    // Set up function called before each test
}

void tearDown(void) {
    // Tear down function called after each test
}

static void test_arena_create_success(void) {
    MemoryArena *arena = arena_create(1024);
    
    TEST_ASSERT_NOT_NULL(arena);
    TEST_ASSERT_NOT_NULL(arena->memory);
    TEST_ASSERT_EQUAL(1024, arena->size);
    TEST_ASSERT_EQUAL(0, arena->used);
    
    arena_free(arena);
}

static void test_arena_create_zero_size(void) {
    MemoryArena *arena = arena_create(0);
    
    TEST_ASSERT_NULL(arena);
}

static void test_arena_create_large_size(void) {
    MemoryArena *arena = arena_create(1024 * 1024);
    
    TEST_ASSERT_NOT_NULL(arena);
    TEST_ASSERT_NOT_NULL(arena->memory);
    TEST_ASSERT_EQUAL(1024 * 1024, arena->size);
    TEST_ASSERT_EQUAL(0, arena->used);
    
    arena_free(arena);
}

static void test_arena_alloc_success(void) {
    MemoryArena *arena = arena_create(1024);
    
    void *ptr1 = arena_alloc(arena, 100);
    TEST_ASSERT_NOT_NULL(ptr1);
    TEST_ASSERT_EQUAL(100, arena->used);
    
    void *ptr2 = arena_alloc(arena, 200);
    TEST_ASSERT_NOT_NULL(ptr2);
    // Account for 8-byte alignment: 100 bytes aligned to 104, then 200 more = 304
    TEST_ASSERT_EQUAL(304, arena->used);
    
    // Check that pointers are different and don't overlap
    TEST_ASSERT_NOT_EQUAL(ptr1, ptr2);
    TEST_ASSERT_TRUE(ptr2 >= (void*)((uintptr_t)ptr1 + 100));
    
    arena_free(arena);
}

static void test_arena_alloc_insufficient_space(void) {
    MemoryArena *arena = arena_create(100);
    
    void *ptr1 = arena_alloc(arena, 50);
    TEST_ASSERT_NOT_NULL(ptr1);
    TEST_ASSERT_EQUAL(50, arena->used);
    
    void *ptr2 = arena_alloc(arena, 60);  // Should fail, only 50 bytes left
    TEST_ASSERT_NULL(ptr2);
    TEST_ASSERT_EQUAL(50, arena->used);  // Should not change
    
    arena_free(arena);
}

static void test_arena_alloc_exact_fit(void) {
    MemoryArena *arena = arena_create(100);
    
    void *ptr = arena_alloc(arena, 100);
    TEST_ASSERT_NOT_NULL(ptr);
    TEST_ASSERT_EQUAL(100, arena->used);
    
    void *ptr2 = arena_alloc(arena, 1);  // Should fail, no space left
    TEST_ASSERT_NULL(ptr2);
    TEST_ASSERT_EQUAL(100, arena->used);
    
    arena_free(arena);
}

static void test_arena_alloc_zero_size(void) {
    MemoryArena *arena = arena_create(1024);
    
    void *ptr = arena_alloc(arena, 0);
    TEST_ASSERT_NULL(ptr);
    TEST_ASSERT_EQUAL(0, arena->used);
    
    arena_free(arena);
}

static void test_arena_alloc_null_arena(void) {
    void *ptr = arena_alloc(NULL, 100);
    TEST_ASSERT_NULL(ptr);
}

static void test_arena_strdup_success(void) {
    MemoryArena *arena = arena_create(1024);
    const char *original = "Hello, World!";
    
    char *copy = arena_strdup(arena, original);
    
    TEST_ASSERT_NOT_NULL(copy);
    TEST_ASSERT_STRING_EQUAL(original, copy);
    TEST_ASSERT_NOT_EQUAL(original, copy);  // Different pointers
    TEST_ASSERT_EQUAL(strlen(original) + 1, arena->used);
    
    arena_free(arena);
}

static void test_arena_strdup_null_string(void) {
    MemoryArena *arena = arena_create(1024);
    
    char *copy = arena_strdup(arena, NULL);
    
    TEST_ASSERT_NULL(copy);
    TEST_ASSERT_EQUAL(0, arena->used);
    
    arena_free(arena);
}

static void test_arena_strdup_empty_string(void) {
    MemoryArena *arena = arena_create(1024);
    const char *original = "";
    
    char *copy = arena_strdup(arena, original);
    
    TEST_ASSERT_NOT_NULL(copy);
    TEST_ASSERT_STRING_EQUAL(original, copy);
    TEST_ASSERT_EQUAL(1, arena->used);  // Just the null terminator
    
    arena_free(arena);
}

static void test_arena_strdup_insufficient_space(void) {
    MemoryArena *arena = arena_create(5);
    const char *original = "Hello, World!";  // 13 + 1 = 14 bytes needed
    
    char *copy = arena_strdup(arena, original);
    
    TEST_ASSERT_NULL(copy);
    TEST_ASSERT_EQUAL(0, arena->used);
    
    arena_free(arena);
}

static void test_arena_strndup_success(void) {
    MemoryArena *arena = arena_create(1024);
    const char *original = "Hello, World!";
    
    char *copy = arena_strndup(arena, original, 5);
    
    TEST_ASSERT_NOT_NULL(copy);
    TEST_ASSERT_STRING_EQUAL("Hello", copy);
    TEST_ASSERT_EQUAL(6, arena->used);  // 5 chars + null terminator
    
    arena_free(arena);
}

static void test_arena_strndup_longer_than_string(void) {
    MemoryArena *arena = arena_create(1024);
    const char *original = "Hello";
    
    char *copy = arena_strndup(arena, original, 10);
    
    TEST_ASSERT_NOT_NULL(copy);
    TEST_ASSERT_STRING_EQUAL("Hello", copy);
    TEST_ASSERT_EQUAL(6, arena->used);  // 5 chars + null terminator
    
    arena_free(arena);
}

static void test_arena_strndup_zero_length(void) {
    MemoryArena *arena = arena_create(1024);
    const char *original = "Hello";
    
    char *copy = arena_strndup(arena, original, 0);
    
    TEST_ASSERT_NOT_NULL(copy);
    TEST_ASSERT_STRING_EQUAL("", copy);
    TEST_ASSERT_EQUAL(1, arena->used);  // Just null terminator
    
    arena_free(arena);
}

static void test_arena_strndup_null_string(void) {
    MemoryArena *arena = arena_create(1024);
    
    char *copy = arena_strndup(arena, NULL, 5);
    
    TEST_ASSERT_NULL(copy);
    TEST_ASSERT_EQUAL(0, arena->used);
    
    arena_free(arena);
}

static void test_arena_free_null(void) {
    // Should not crash
    arena_free(NULL);
}

static void test_arena_alignment(void) {
    MemoryArena *arena = arena_create(1024);
    
    // Allocate various sizes and check alignment
    void *ptr1 = arena_alloc(arena, 1);
    void *ptr2 = arena_alloc(arena, 1);
    void *ptr3 = arena_alloc(arena, 1);
    
    TEST_ASSERT_NOT_NULL(ptr1);
    TEST_ASSERT_NOT_NULL(ptr2);
    TEST_ASSERT_NOT_NULL(ptr3);
    
    // Check that allocations are properly spaced
    TEST_ASSERT_TRUE(ptr2 >= (void*)((uintptr_t)ptr1 + 1));
    TEST_ASSERT_TRUE(ptr3 >= (void*)((uintptr_t)ptr2 + 1));
    
    arena_free(arena);
}

static void test_arena_multiple_allocations(void) {
    MemoryArena *arena = arena_create(1024);
    void *ptrs[10];
    size_t sizes[10] = {8, 16, 32, 64, 128, 8, 16, 32, 64, 128};
    
    size_t total_used = 0;
    for (int i = 0; i < 10; i++) {
        ptrs[i] = arena_alloc(arena, sizes[i]);
        TEST_ASSERT_NOT_NULL(ptrs[i]);
        total_used += sizes[i];
        TEST_ASSERT_EQUAL(total_used, arena->used);
    }
    
    // Verify all pointers are different
    for (int i = 0; i < 10; i++) {
        for (int j = i + 1; j < 10; j++) {
            TEST_ASSERT_NOT_EQUAL(ptrs[i], ptrs[j]);
        }
    }
    
    arena_free(arena);
}

static void test_arena_set_get_current(void) {
    MemoryArena *arena1 = arena_create(1024);
    MemoryArena *arena2 = arena_create(2048);
    
    // Test setting and getting current arena
    set_current_arena(arena1);
    TEST_ASSERT_EQUAL(arena1, get_current_arena());
    
    set_current_arena(arena2);
    TEST_ASSERT_EQUAL(arena2, get_current_arena());
    
    set_current_arena(NULL);
    TEST_ASSERT_NULL(get_current_arena());
    
    arena_free(arena1);
    arena_free(arena2);
}

int main(void) {
    UNITY_BEGIN();
    
    RUN_TEST(test_arena_create_success);
    RUN_TEST(test_arena_create_zero_size);
    RUN_TEST(test_arena_create_large_size);
    RUN_TEST(test_arena_alloc_success);
    RUN_TEST(test_arena_alloc_insufficient_space);
    RUN_TEST(test_arena_alloc_exact_fit);
    RUN_TEST(test_arena_alloc_zero_size);
    RUN_TEST(test_arena_alloc_null_arena);
    RUN_TEST(test_arena_strdup_success);
    RUN_TEST(test_arena_strdup_null_string);
    RUN_TEST(test_arena_strdup_empty_string);
    RUN_TEST(test_arena_strdup_insufficient_space);
    RUN_TEST(test_arena_strndup_success);
    RUN_TEST(test_arena_strndup_longer_than_string);
    RUN_TEST(test_arena_strndup_zero_length);
    RUN_TEST(test_arena_strndup_null_string);
    RUN_TEST(test_arena_free_null);
    RUN_TEST(test_arena_alignment);
    RUN_TEST(test_arena_multiple_allocations);
    RUN_TEST(test_arena_set_get_current);
    
    return UNITY_END();
}
