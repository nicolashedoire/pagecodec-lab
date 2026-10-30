/*
 * Paired, randomized memory-representation benchmark for macOS.
 * Build: clang -O3 -Wall -Wextra -Werror compare.c codec.c -lcompression -o compare
 *
 * This compares materializing complete process-owned buffers. It does not
 * measure physical DRAM latency, kernel compression, RSS savings, or energy.
 * Source, packed storage and validation destination coexist throughout.
 */
#include "codec.h"
#include <compression.h>
#include <errno.h>
#include <inttypes.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#define METHOD_COUNT 5
#define MAX_REPETITIONS 31
#define MAX_INPUT_BYTES ((size_t)64 * 1024 * 1024)

typedef struct {
    uint64_t offset;
    uint32_t length;
    uint32_t mode;
} Page;
_Static_assert(sizeof(Page) == 16, "Page metadata must be 16 bytes");

typedef struct {
    size_t payload_bytes;
    size_t raw_pages;
    size_t compressed_pages;
    double encode_samples[MAX_REPETITIONS];
    double decode_samples[MAX_REPETITIONS];
} Result;

typedef struct {
    uint8_t *source;
    uint8_t *packed;
    uint8_t *output;
    uint8_t *candidate;
    size_t candidate_capacity;
    Page *table;
    void *encode_scratch[METHOD_COUNT];
    void *decode_scratch[METHOD_COUNT];
    size_t size;
    size_t page_size;
    size_t page_count;
    unsigned inner_iterations;
} Workspace;

static const char *method_names[METHOD_COUNT] = {
    "raw", "apple_lz4", "apple_lz4_raw", "apple_lzfse", "custom_rc"
};
static volatile uint64_t observed_checksum;

static void fail(const char *message) {
    fprintf(stderr, "compare: %s\n", message);
    exit(EXIT_FAILURE);
}

static void *allocate(size_t n) {
    void *result = malloc(n ? n : 1);
    if (!result) fail("allocation failed");
    return result;
}

static double now_ms(void) {
    struct timespec value;
    if (clock_gettime(CLOCK_MONOTONIC, &value)) fail("clock_gettime failed");
    return (double)value.tv_sec * 1000.0 + (double)value.tv_nsec / 1000000.0;
}

static uint64_t parse_number(const char *text) {
    char *end = NULL;
    errno = 0;
    unsigned long long value = strtoull(text, &end, 10);
    if (errno || !text[0] || text[0] == '-' || *end) fail("invalid numeric option");
    return (uint64_t)value;
}

static uint64_t random_u64(uint64_t *state) {
    uint64_t value = *state;
    value ^= value >> 12;
    value ^= value << 25;
    value ^= value >> 27;
    *state = value;
    return value * UINT64_C(2685821657736338717);
}

static uint64_t checksum(const uint8_t *data, size_t size) {
    uint64_t hash = UINT64_C(14695981039346656037);
    for (size_t i = 0; i < size; ++i) {
        hash ^= data[i];
        hash *= UINT64_C(1099511628211);
    }
    return hash;
}

__attribute__((noinline))
static void copy_buffer(uint8_t *destination, const uint8_t *source, size_t n) {
    memcpy(destination, source, n);
}

static compression_algorithm apple_algorithm(unsigned method) {
    if (method == 1) return COMPRESSION_LZ4;
    if (method == 2) return COMPRESSION_LZ4_RAW;
    return COMPRESSION_LZFSE;
}

static void encode(Workspace *w, unsigned method, Result *result) {
    result->payload_bytes = result->raw_pages = result->compressed_pages = 0;
    if (method == 0) {
        copy_buffer(w->packed, w->source, w->size);
        result->payload_bytes = w->size;
        result->raw_pages = w->page_count;
        return;
    }
    for (size_t offset = 0, index = 0; offset < w->size;
         offset += w->page_size, ++index) {
        const size_t remaining = w->size - offset;
        const size_t length = remaining < w->page_size ? remaining : w->page_size;
        size_t compressed;
        if (method == 4) {
            compressed = rc_encode(w->candidate, w->candidate_capacity,
                                   w->source + offset, length);
            if (!compressed) fail("custom codec encode failed within its declared bound");
            if (compressed > w->candidate_capacity) fail("custom codec exceeded destination bound");
        } else {
            compressed = compression_encode_buffer(
                w->candidate, w->candidate_capacity, w->source + offset, length,
                w->encode_scratch[method], apple_algorithm(method));
        }
        const bool keep = compressed && compressed < length;
        const size_t stored = keep ? compressed : length;
        if (stored > w->size - result->payload_bytes) fail("packed storage overflow");
        w->table[index].offset = result->payload_bytes;
        w->table[index].length = (uint32_t)stored;
        w->table[index].mode = keep ? 1U : 0U;
        copy_buffer(w->packed + result->payload_bytes,
                    keep ? w->candidate : w->source + offset, stored);
        result->payload_bytes += stored;
        if (keep) ++result->compressed_pages;
        else ++result->raw_pages;
    }
}

static void decode(Workspace *w, unsigned method, size_t payload_bytes) {
    if (method == 0) {
        copy_buffer(w->output, w->packed, w->size);
        return;
    }
    for (size_t offset = 0, index = 0; offset < w->size;
         offset += w->page_size, ++index) {
        const size_t remaining = w->size - offset;
        const size_t length = remaining < w->page_size ? remaining : w->page_size;
        const Page page = w->table[index];
        if (page.offset > payload_bytes || page.length > payload_bytes - page.offset)
            fail("invalid page payload bounds");
        if (page.mode == 0) {
            if (page.length != length) fail("invalid raw page length");
            copy_buffer(w->output + offset, w->packed + page.offset, length);
        } else if (page.mode == 1) {
            if (method == 4) {
                if (!rc_decode(w->output + offset, length,
                               w->packed + page.offset, page.length))
                    fail("custom codec decode rejected its own output");
            } else {
                size_t actual = compression_decode_buffer(
                    w->output + offset, length, w->packed + page.offset, page.length,
                    w->decode_scratch[method], apple_algorithm(method));
                if (actual != length) fail("Apple codec decoded an unexpected length");
            }
        } else {
            fail("invalid page mode");
        }
    }
}

static void trial(Workspace *w, unsigned method, Result *result,
                  unsigned round, bool warmup, uint64_t expected_checksum) {
    const size_t previous_payload = result->payload_bytes;
    const size_t previous_raw = result->raw_pages;
    const size_t previous_compressed = result->compressed_pages;
    double start = now_ms();
    for (unsigned iteration = 0; iteration < w->inner_iterations; ++iteration) {
        encode(w, method, result);
        __asm__ __volatile__("" ::: "memory");
    }
    double encoded = now_ms();
    for (unsigned iteration = 0; iteration < w->inner_iterations; ++iteration) {
        decode(w, method, result->payload_bytes);
        __asm__ __volatile__("" ::: "memory");
    }
    double decoded = now_ms();
    if (memcmp(w->source, w->output, w->size)) fail("byte-for-byte round-trip mismatch");
    observed_checksum = checksum(w->output, w->size);
    if (observed_checksum != expected_checksum) fail("checksum mismatch");
    if (!warmup) {
        if (result->payload_bytes != previous_payload || result->raw_pages != previous_raw ||
            result->compressed_pages != previous_compressed)
            fail("non-deterministic storage size between repeated trials");
        result->encode_samples[round] = (encoded - start) / w->inner_iterations;
        result->decode_samples[round] = (decoded - encoded) / w->inner_iterations;
    }
}

static int compare_double(const void *left, const void *right) {
    double a = *(const double *)left, b = *(const double *)right;
    return (a > b) - (a < b);
}

static double median(const double *samples, unsigned count) {
    double copy[MAX_REPETITIONS];
    memcpy(copy, samples, count * sizeof(*samples));
    qsort(copy, count, sizeof(*samples), compare_double);
    return count & 1U ? copy[count / 2]
                      : (copy[count / 2 - 1] + copy[count / 2]) * 0.5;
}

static void print_string(const char *value) {
    putchar('"');
    for (const unsigned char *p = (const unsigned char *)value; *p; ++p) {
        if (*p == '"' || *p == '\\') printf("\\%c", *p);
        else if (*p < 0x20) printf("\\u%04x", (unsigned)*p);
        else putchar(*p);
    }
    putchar('"');
}

static void print_samples(const double *samples, unsigned count) {
    putchar('[');
    for (unsigned i = 0; i < count; ++i)
        printf("%s%.6f", i ? "," : "", samples[i]);
    putchar(']');
}

int main(int argc, char **argv) {
    const char *input_path = NULL;
    unsigned repetitions = 9;
    size_t page_size = 16 * 1024;
    uint64_t seed = UINT64_C(20261030);
    for (int i = 1; i < argc; ++i) {
        if (!strcmp(argv[i], "--help")) {
            printf("Usage: %s --input FILE [--repetitions 9] [--page-kib 16] [--seed NUMBER]\n", argv[0]);
            return 0;
        }
        if (i + 1 >= argc) fail("option requires a value; see --help");
        if (!strcmp(argv[i], "--input")) input_path = argv[++i];
        else if (!strcmp(argv[i], "--repetitions")) {
            uint64_t value = parse_number(argv[++i]);
            if (value < 1 || value > MAX_REPETITIONS) fail("repetitions must be 1..31");
            repetitions = (unsigned)value;
        } else if (!strcmp(argv[i], "--page-kib")) {
            uint64_t value = parse_number(argv[++i]);
            if (value < 1 || value > 1024) fail("page size must be 1..1024 KiB");
            page_size = (size_t)value * 1024;
        } else if (!strcmp(argv[i], "--seed")) seed = parse_number(argv[++i]);
        else fail("unknown option; see --help");
    }
    if (!input_path) fail("--input FILE is required");
    FILE *file = fopen(input_path, "rb");
    if (!file) fail("cannot open input file");
    if (fseek(file, 0, SEEK_END)) fail("input must be a seekable regular file");
    long file_length = ftell(file);
    if (file_length <= 0 || (uint64_t)file_length > MAX_INPUT_BYTES)
        fail("input size must be between 1 byte and 64 MiB");
    rewind(file);
    Workspace w = {0};
    w.size = (size_t)file_length;
    w.page_size = page_size;
    w.page_count = (w.size + page_size - 1) / page_size;
    const size_t batching_bytes = (size_t)8 * 1024 * 1024;
    size_t iterations = (batching_bytes + w.size - 1) / w.size;
    if (iterations < 1) iterations = 1;
    if (iterations > 1024) iterations = 1024;
    w.inner_iterations = (unsigned)iterations;
    w.source = allocate(w.size);
    if (fread(w.source, 1, w.size, file) != w.size) fail("cannot read entire input file");
    if (fclose(file)) fail("cannot close input file");
    const size_t custom_bound = rc_bound(page_size);
    if (custom_bound < page_size || custom_bound > page_size + 1024 * 1024)
        fail("unexpected custom codec bound");
    w.candidate_capacity = page_size + 1024;
    if (custom_bound > w.candidate_capacity) w.candidate_capacity = custom_bound;
    w.packed = allocate(w.size + w.candidate_capacity);
    w.output = allocate(w.size);
    w.candidate = allocate(w.candidate_capacity);
    w.table = allocate(w.page_count * sizeof(Page));
    memset(w.packed, 0xa5, w.size + w.candidate_capacity);
    memset(w.output, 0x5a, w.size);
    memset(w.candidate, 0, w.candidate_capacity);
    memset(w.table, 0, w.page_count * sizeof(Page));
    for (unsigned method = 1; method <= 3; ++method) {
        size_t encode_size = compression_encode_scratch_buffer_size(apple_algorithm(method));
        size_t decode_size = compression_decode_scratch_buffer_size(apple_algorithm(method));
        w.encode_scratch[method] = allocate(encode_size);
        w.decode_scratch[method] = allocate(decode_size);
        memset(w.encode_scratch[method], 0, encode_size);
        memset(w.decode_scratch[method], 0, decode_size);
    }

    Result results[METHOD_COUNT] = {0};
    const uint64_t expected_checksum = checksum(w.source, w.size);
    for (unsigned method = 0; method < METHOD_COUNT; ++method)
        trial(&w, method, &results[method], 0, true, expected_checksum);
    unsigned orders[MAX_REPETITIONS][METHOD_COUNT];
    uint64_t random_state = seed ^ UINT64_C(0x9e3779b97f4a7c15);
    if (!random_state) random_state = 1;
    for (unsigned round = 0; round < repetitions; ++round) {
        for (unsigned i = 0; i < METHOD_COUNT; ++i) orders[round][i] = i;
        for (unsigned i = METHOD_COUNT - 1; i > 0; --i) {
            unsigned j = (unsigned)(random_u64(&random_state) % (i + 1));
            unsigned temporary = orders[round][i];
            orders[round][i] = orders[round][j];
            orders[round][j] = temporary;
        }
        for (unsigned i = 0; i < METHOD_COUNT; ++i) {
            unsigned method = orders[round][i];
            trial(&w, method, &results[method], round, false, expected_checksum);
        }
    }

    printf("{\"schema_version\":1,\"input_path\":");
    print_string(input_path);
    printf(",\"input_bytes\":%zu,\"page_size\":%zu,\"page_count\":%zu,"
           "\"repetitions\":%u,\"inner_iterations\":%u,\"warmup_rounds_per_method\":1,\"shuffle_seed\":%" PRIu64 ","
           "\"checksum_fnv1a64\":\"%016" PRIx64 "\","
           "\"timing_scope\":\"single-thread warm-buffer materialization; encode batch then decode batch; per-buffer times divide batch duration by inner_iterations; fixed batching=min(1024,max(1,ceil(8MiB/input_bytes))); allocation, IO and validation excluded; copies and decisions included; validation after each batch\","
           "\"storage_scope\":\"payload plus 16-byte page table for compressed methods; source, output, unused reserved capacity, scratch, allocator and OS overhead excluded; not measured RSS\","
           "\"apple_selection\":\"keep compressed only if payload is strictly smaller than original page\","
           "\"custom_selection\":\"keep custom output including its headers only if smaller than original page\","
           "\"trial_order\":[",
           w.size, page_size, w.page_count, repetitions, w.inner_iterations, seed, expected_checksum);
    for (unsigned round = 0; round < repetitions; ++round) {
        printf("%s[", round ? "," : "");
        for (unsigned i = 0; i < METHOD_COUNT; ++i) {
            if (i) putchar(',');
            print_string(method_names[orders[round][i]]);
        }
        putchar(']');
    }
    printf("],\"results\":[");
    for (unsigned method = 0; method < METHOD_COUNT; ++method) {
        const Result *r = &results[method];
        const size_t metadata = method == 0 ? 0 : w.page_count * sizeof(Page);
        printf("%s{\"method\":\"%s\",\"payload_bytes\":%zu,\"metadata_bytes\":%zu,"
               "\"total_storage_bytes\":%zu,\"raw_pages\":%zu,\"compressed_pages\":%zu,"
               "\"encode_ms\":%.6f,\"decode_ms\":%.6f,\"encode_samples_ms\":",
               method ? "," : "", method_names[method], r->payload_bytes, metadata,
               r->payload_bytes + metadata, r->raw_pages, r->compressed_pages,
               median(r->encode_samples, repetitions), median(r->decode_samples, repetitions));
        print_samples(r->encode_samples, repetitions);
        printf(",\"decode_samples_ms\":");
        print_samples(r->decode_samples, repetitions);
        printf(",\"verified\":true}");
    }
    puts("]}");
    for (unsigned method = 1; method <= 3; ++method) {
        free(w.encode_scratch[method]);
        free(w.decode_scratch[method]);
    }
    free(w.source); free(w.packed); free(w.output); free(w.candidate); free(w.table);
    return 0;
}
