// Exact application-level feature computation; not a transparent RAM compressor.
#include <algorithm>
#include <array>
#include <chrono>
#include <cmath>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <ctime>
#include <fstream>
#include <iomanip>
#include <iostream>
#include <limits>
#include <numeric>
#include <random>
#include <sstream>
#include <stdexcept>
#include <string>
#include <vector>
#if defined(__unix__) || defined(__APPLE__)
#include <sys/resource.h>
#endif
extern "C" {
#include "../../codec.h"
}

namespace {
constexpr size_t features = 8;
constexpr int32_t missing_input = std::numeric_limits<int32_t>::min();
constexpr int64_t missing_output = std::numeric_limits<int64_t>::min();
constexpr std::array<const char *, features> feature_names = {
    "sum", "difference", "product", "x_squared", "y_squared", "minimum", "maximum", "absolute_difference"};
constexpr std::array<const char *, features> feature_formulas = {
    "x+y", "x-y", "x*y", "x*x", "y*y", "min(x,y)", "max(x,y)", "abs(x-y)"};
constexpr std::array<const char *, 5> method_names = {
    "materialized", "compressed_blocks", "recipe", "recipe_cache", "recipe_direct"};
using Clock = std::chrono::steady_clock;

struct Options {
    std::string x, y, output, only_method;
    size_t repetitions = 9, tile_rows = 256, cache_tiles = 64, queries = 4096;
    uint64_t seed = 20261030;
};
struct Query { size_t first, count; uint8_t mask; };
struct Workload { const char *name; std::vector<Query> queries; uint64_t expected = 0; };
struct Sample { double wall_ms = 0, cpu_ms = 0; uint64_t checksum = 0, hits = 0, misses = 0; };
struct Trial { size_t repetition, order_position; Sample construction; std::array<Sample, 4> workloads; };
struct Memory {
    size_t source_logical = 0, source_capacity = 0, derived_logical = 0, derived_capacity = 0;
    size_t index_capacity = 0, cache_capacity = 0, scratch_capacity = 0;
    size_t recipe_descriptor = features, other_objects = 0, schedule = 0, construction_extra = 0;
    size_t total() const {
        return source_capacity + derived_capacity + index_capacity + cache_capacity + scratch_capacity
            + recipe_descriptor + other_objects + schedule;
    }
};
struct Result { Memory memory; std::vector<Trial> trials; bool present = false; };

size_t number(const std::string &value, const char *name) {
    if (value.empty() || value[0] == '-') throw std::runtime_error(std::string("Invalid ") + name);
    size_t consumed = 0;
    const auto n = std::stoull(value, &consumed);
    if (consumed != value.size() || n > std::numeric_limits<size_t>::max())
        throw std::runtime_error(std::string("Invalid ") + name);
    return static_cast<size_t>(n);
}
Options options(int argc, char **argv) {
    Options o;
    for (int i = 1; i < argc; ++i) {
        const std::string key = argv[i];
        if (key == "--help") {
            std::cout << "recipe-bench --x FILE --y FILE [--output FILE] [--repetitions 9] "
                         "[--tile-rows 256] [--cache-tiles 64] [--queries 4096] [--seed 20261030] "
                         "[--only-method materialized|compressed_blocks|recipe|recipe_cache|recipe_direct]\n";
            std::exit(0);
        }
        if (i + 1 == argc) throw std::runtime_error("Missing value for " + key);
        const std::string value = argv[++i];
        if (key == "--x") o.x = value;
        else if (key == "--y") o.y = value;
        else if (key == "--output") o.output = value;
        else if (key == "--only-method") o.only_method = value;
        else if (key == "--repetitions") o.repetitions = number(value, "repetitions");
        else if (key == "--tile-rows") o.tile_rows = number(value, "tile rows");
        else if (key == "--cache-tiles") o.cache_tiles = number(value, "cache tiles");
        else if (key == "--queries") o.queries = number(value, "queries");
        else if (key == "--seed") o.seed = number(value, "seed");
        else throw std::runtime_error("Unknown argument " + key);
    }
    if (o.x.empty() || o.y.empty()) throw std::runtime_error("Both --x and --y are required");
    if (!o.repetitions || o.repetitions > 100 || !o.tile_rows || o.tile_rows > 65536 ||
        !o.cache_tiles || o.cache_tiles > 65536 || !o.queries || o.queries > 1000000)
        throw std::runtime_error("Arguments exceed documented positive limits");
    if (o.tile_rows * o.cache_tiles > 4 * 1024 * 1024)
        throw std::runtime_error("Tile cache exceeds 256 MiB capacity limit");
    if (!o.only_method.empty() && std::none_of(method_names.begin(), method_names.end(),
        [&](const char *name) { return o.only_method == name; })) throw std::runtime_error("Unknown method");
    return o;
}
std::vector<int32_t> read_i32(const std::string &path) {
    std::ifstream stream(path, std::ios::binary | std::ios::ate);
    if (!stream) throw std::runtime_error("Cannot open input");
    const auto end = stream.tellg();
    if (end <= 0 || end % 4 != 0 || end > 32 * 1024 * 1024)
        throw std::runtime_error("Inputs must be nonempty, <=32 MiB, with a multiple-of-four byte length");
    std::vector<int32_t> data(static_cast<size_t>(end) / 4);
    stream.seekg(0);
    stream.read(reinterpret_cast<char *>(data.data()), end);
    if (!stream) throw std::runtime_error("Input read failed");
    for (size_t i = 0; i < data.size(); ++i) {
        const auto *p = reinterpret_cast<const unsigned char *>(&data[i]);
        const uint32_t u = uint32_t(p[0]) | (uint32_t(p[1]) << 8) | (uint32_t(p[2]) << 16) | (uint32_t(p[3]) << 24);
        data[i] = static_cast<int32_t>(u >= 0x80000000U ? int64_t(u) - 0x100000000LL : int64_t(u));
    }
    return data;
}
int64_t feature(int32_t x32, int32_t y32, uint8_t id) {
    if (x32 == missing_input || y32 == missing_input) return missing_output;
    const int64_t x = x32, y = y32;
    switch (id) {
        case 0: return x + y;
        case 1: return x - y;
        case 2: return x * y;
        case 3: return x * x;
        case 4: return y * y;
        case 5: return std::min(x, y);
        case 6: return std::max(x, y);
        case 7: return x >= y ? x - y : y - x;
        default: throw std::runtime_error("Invalid feature");
    }
}
uint64_t mix(uint64_t x) {
    x ^= x >> 30; x *= 0xbf58476d1ce4e5b9ULL;
    x ^= x >> 27; x *= 0x94d049bb133111ebULL;
    return x ^ (x >> 31);
}
uint64_t term(int64_t value, size_t row, size_t id) {
    return mix(static_cast<uint64_t>(value) ^ (uint64_t(row) * 0x9e3779b97f4a7c15ULL + id));
}
std::string hex(uint64_t n) { std::ostringstream s; s << std::hex << std::setw(16) << std::setfill('0') << n; return s.str(); }
uint64_t source_fingerprint(const std::vector<int32_t> &data) {
    uint64_t hash = 14695981039346656037ULL;
    for (int32_t value : data) for (unsigned j = 0; j < 4; ++j) {
        hash ^= (static_cast<uint32_t>(value) >> (8 * j)) & 255U;
        hash *= 1099511628211ULL;
    }
    return hash;
}
std::array<Workload, 4> workloads(size_t n, const Options &o) {
    std::array<Workload, 4> w = {{{"sequential_all", {}, 0}, {"repeated_hot_windows", {}, 0},
                                  {"sparse_points", {}, 0}, {"changing_hotset", {}, 0}}};
    w[0].queries.push_back({0, n, 255});
    const size_t tiles = (n + o.tile_rows - 1) / o.tile_rows;
    const size_t hot = std::min(tiles, std::max(size_t(1), std::min(size_t(16), o.cache_tiles / 4)));
    for (size_t i = 0; i < std::min(o.queries, size_t(256)); ++i) {
        const size_t first = (i % hot) * o.tile_rows;
        w[1].queries.push_back({first, std::min(o.tile_rows, n - first), 255});
    }
    std::mt19937_64 rng(o.seed);
    for (size_t i = 0; i < o.queries; ++i)
        w[2].queries.push_back({static_cast<size_t>(rng() % n), 1, static_cast<uint8_t>(1U << (rng() % features))});
    for (size_t i = 0; i < std::min(o.queries, size_t(1024)); ++i) {
        const size_t tile = ((i % o.cache_tiles) + ((i / o.cache_tiles) % 4) * o.cache_tiles) % tiles;
        const size_t first = tile * o.tile_rows;
        w[3].queries.push_back({first, std::min(o.tile_rows, n - first), 255});
    }
    return w;
}
uint64_t oracle(const Workload &w, const std::vector<int32_t> &x, const std::vector<int32_t> &y) {
    uint64_t checksum = 0;
    for (const auto &q : w.queries) for (size_t f = 0; f < features; ++f) if (q.mask & (1U << f))
        for (size_t row = q.first; row < q.first + q.count; ++row) checksum += term(feature(x[row], y[row], uint8_t(f)), row, f);
    return checksum;
}

struct Tile { const int64_t *data; size_t stride; };
class Method {
    const std::vector<int32_t> &x_, &y_;
    size_t kind_, tile_rows_, cache_tiles_;
    std::array<uint8_t, features> recipe_{{0, 1, 2, 3, 4, 5, 6, 7}};
    std::vector<int64_t> derived_, scratch_, cache_;
    std::vector<std::vector<uint8_t>> blocks_;
    std::vector<size_t> tags_;
    size_t last_ = std::numeric_limits<size_t>::max(), build_extra_ = 0;
    void compute(size_t tile, int64_t *destination) const {
        const size_t first = tile * tile_rows_, count = std::min(tile_rows_, x_.size() - first);
        for (size_t f = 0; f < features; ++f) for (size_t j = 0; j < count; ++j)
            destination[f * count + j] = feature(x_[first + j], y_[first + j], recipe_[f]);
    }
public:
    uint64_t hits = 0, misses = 0;
    Method(size_t kind, const std::vector<int32_t> &x, const std::vector<int32_t> &y, const Options &o)
        : x_(x), y_(y), kind_(kind), tile_rows_(o.tile_rows), cache_tiles_(o.cache_tiles) {
        if (kind_ == 0) {
            derived_.resize(features * x.size());
            for (size_t f = 0; f < features; ++f) for (size_t row = 0; row < x.size(); ++row)
                derived_[f * x.size() + row] = feature(x[row], y[row], recipe_[f]);
        } else if (kind_ == 3) {
            cache_.resize(cache_tiles_ * tile_rows_ * features);
            tags_.assign(cache_tiles_, std::numeric_limits<size_t>::max());
        } else if (kind_ != 4) {
            scratch_.resize(tile_rows_ * features);
            if (kind_ == 1) {
                const size_t count = (x.size() + tile_rows_ - 1) / tile_rows_;
                blocks_.resize(count);
                std::vector<uint8_t> encoded(rc_bound(tile_rows_ * features * sizeof(int64_t)));
                build_extra_ = encoded.capacity() + sizeof(encoded);
                for (size_t tile = 0; tile < count; ++tile) {
                    compute(tile, scratch_.data());
                    const size_t bytes = std::min(tile_rows_, x.size() - tile * tile_rows_) * features * sizeof(int64_t);
                    const size_t length = rc_encode(encoded.data(), encoded.size(), reinterpret_cast<const uint8_t *>(scratch_.data()), bytes);
                    if (!length) throw std::runtime_error("Frozen codec encode failed");
                    blocks_[tile].assign(encoded.begin(), encoded.begin() + static_cast<std::ptrdiff_t>(length));
                }
            }
        }
    }
    void validate() {
        if (kind_ != 1) return;
        // Full per-value validation is deliberately outside construction/query timing.
        for (size_t index = 0; index < blocks_.size(); ++index) {
            const size_t rows = std::min(tile_rows_, x_.size() - index * tile_rows_);
            if (!rc_decode(reinterpret_cast<uint8_t *>(scratch_.data()), rows * features * sizeof(int64_t),
                           blocks_[index].data(), blocks_[index].size())) throw std::runtime_error("Codec validation decode failed");
            for (size_t f = 0; f < features; ++f) for (size_t j = 0; j < rows; ++j)
                if (scratch_[f * rows + j] != feature(x_[index * tile_rows_ + j], y_[index * tile_rows_ + j], recipe_[f]))
                    throw std::runtime_error("Frozen codec changed a derived value");
        }
    }
    void reset() {
        hits = misses = 0; last_ = std::numeric_limits<size_t>::max();
        std::fill(tags_.begin(), tags_.end(), std::numeric_limits<size_t>::max());
    }
    Tile tile(size_t index) {
        if (kind_ == 0) return {derived_.data() + index * tile_rows_, x_.size()};
        const size_t rows = std::min(tile_rows_, x_.size() - index * tile_rows_);
        if (kind_ == 3) {
            const size_t slot = index % cache_tiles_;
            int64_t *p = cache_.data() + slot * tile_rows_ * features;
            if (tags_[slot] == index) ++hits;
            else { ++misses; compute(index, p); tags_[slot] = index; }
            return {p, rows};
        }
        if (last_ == index) ++hits;
        else {
            ++misses;
            if (kind_ == 1) {
                if (!rc_decode(reinterpret_cast<uint8_t *>(scratch_.data()), rows * features * sizeof(int64_t),
                               blocks_[index].data(), blocks_[index].size())) throw std::runtime_error("Decode failed");
            } else compute(index, scratch_.data());
            last_ = index;
        }
        return {scratch_.data(), rows};
    }
    uint64_t run(const Workload &w) {
        uint64_t checksum = 0;
        for (const auto &q : w.queries) {
            size_t row = q.first;
            while (row < q.first + q.count) {
                const size_t index = row / tile_rows_, offset = row % tile_rows_;
                const size_t count = std::min(q.first + q.count - row, tile_rows_ - offset);
                if (kind_ == 4) {
                    // Compute only requested scalar features; no derived tile is materialized.
                    // Traversal and checksum consumption match the other representations.
                    for (size_t f = 0; f < features; ++f) if (q.mask & (1U << f))
                        for (size_t j = 0; j < count; ++j)
                            checksum += term(feature(x_[row+j], y_[row+j], recipe_[f]), row+j, f);
                } else {
                    const Tile t = tile(index);
                    for (size_t f = 0; f < features; ++f) if (q.mask & (1U << f))
                        for (size_t j = 0; j < count; ++j) checksum += term(t.data[f * t.stride + offset + j], row + j, f);
                }
                row += count;
            }
        }
        return checksum;
    }
    Memory memory(size_t schedule) const {
        Memory m;
        m.source_logical = (x_.size() + y_.size()) * sizeof(int32_t);
        m.source_capacity = (x_.capacity() + y_.capacity()) * sizeof(int32_t);
        m.derived_logical = derived_.size() * sizeof(int64_t);
        m.derived_capacity = derived_.capacity() * sizeof(int64_t);
        for (const auto &b : blocks_) { m.derived_logical += b.size(); m.derived_capacity += b.capacity(); }
        m.index_capacity = blocks_.capacity() * sizeof(std::vector<uint8_t>) + tags_.capacity() * sizeof(size_t);
        m.cache_capacity = cache_.capacity() * sizeof(int64_t);
        m.scratch_capacity = scratch_.capacity() * sizeof(int64_t);
        m.other_objects = sizeof(*this) - sizeof(recipe_) + sizeof(x_) + sizeof(y_);
        m.schedule = schedule; m.construction_extra = build_extra_;
        return m;
    }
};

Sample duration(Clock::time_point start, std::clock_t cpu) {
    Sample s;
    s.wall_ms = std::chrono::duration<double, std::milli>(Clock::now() - start).count();
    s.cpu_ms = 1000.0 * double(std::clock() - cpu) / CLOCKS_PER_SEC;
    return s;
}
double median(std::vector<double> a) {
    std::sort(a.begin(), a.end());
    return a.size() % 2 ? a[a.size()/2] : (a[a.size()/2 - 1] + a[a.size()/2])/2;
}
double med(const Result &r, int workload, bool cpu = false) {
    std::vector<double> values;
    for (const auto &t : r.trials) {
        const Sample &s = workload < 0 ? t.construction : t.workloads[static_cast<size_t>(workload)];
        values.push_back(cpu ? s.cpu_ms : s.wall_ms);
    }
    return median(values);
}
std::string quote(const std::string &value) {
    std::ostringstream out; out << '"';
    for (unsigned char c : value) {
        if (c == '"' || c == '\\') out << '\\' << c;
        else if (c < 32) out << "\\u" << std::hex << std::setw(4) << std::setfill('0') << unsigned(c);
        else out << c;
    }
    out << '"'; return out.str();
}
std::string basename(const std::string &path) { return path.substr(path.find_last_of("/\\") + 1); }
uint64_t rss() {
#if defined(__unix__) || defined(__APPLE__)
    rusage usage{};
    if (getrusage(RUSAGE_SELF, &usage) != 0) return 0;
#if defined(__APPLE__)
    return static_cast<uint64_t>(usage.ru_maxrss);
#else
    return static_cast<uint64_t>(usage.ru_maxrss) * 1024;
#endif
#else
    return 0;
#endif
}
void write_json(std::ostream &out, const Options &o, const std::vector<int32_t> &x, const std::vector<int32_t> &y,
                const std::array<Workload, 4> &w, const std::array<Result, 5> &results) {
    out << std::setprecision(12) << "{\n\"schema_version\":1,\"experiment\":\"recipe\",\"checksums_verified\":true,"
        << "\n\"inputs\":{\"x_name\":" << quote(basename(o.x)) << ",\"y_name\":" << quote(basename(o.y))
        << ",\"rows\":" << x.size() << ",\"source_logical_bytes\":" << 8*x.size()
        << ",\"x_fnv1a64_le\":" << quote(hex(source_fingerprint(x))) << ",\"y_fnv1a64_le\":" << quote(hex(source_fingerprint(y)))
        << ",\"x_missing\":" << std::count(x.begin(), x.end(), missing_input)
        << ",\"y_missing\":" << std::count(y.begin(), y.end(), missing_input) << "},"
        << "\n\"configuration\":{\"repetitions\":" << o.repetitions << ",\"warmups\":0,\"seed\":" << o.seed
        << ",\"tile_rows\":" << o.tile_rows << ",\"tile_derived_bytes\":" << o.tile_rows*features*8
        << ",\"cache_tiles\":" << o.cache_tiles << ",\"sparse_queries\":" << o.queries
        << ",\"only_method\":" << (o.only_method.empty() ? "null" : quote(o.only_method))
        << ",\"cache_policy\":\"direct-mapped tile_index modulo cache_tiles; reset before each workload\"},"
        << "\n\"semantics\":{\"source_type\":\"little-endian int32\",\"derived_type\":\"int64\","
        << "\"missing_input\":-2147483648,\"missing_output\":\"-9223372036854775808\","
        << "\"missing_policy\":\"Either missing source makes all eight features missing\","
        << "\"derived_storage\":\"native-endian int64 within this process; codec is lossless on those bytes\","
        << "\"features\":[";
    for (size_t f = 0; f < features; ++f) {
        if (f) out << ',';
        out << "{\"name\":" << quote(feature_names[f]) << ",\"formula\":" << quote(feature_formulas[f]) << '}';
    }
    out << "]},\n\"protocol\":{\"construction_includes\":\"allocation, feature computation or recipe setup; compressed blocks also encode\","
        << "\"queries_include\":\"tile lookup or direct scalar calculation, computation/decompression, checksum consumption\","
        << "\"excluded\":\"input I/O, schedule generation, streamed oracle, per-value codec validation, JSON output; no source edits\","
        << "\"memory_accounting\":\"actual vector capacities plus C++ objects and schedule; excludes allocator bookkeeping, executable, libraries and report bookkeeping\","
        << "\"all_methods_retain_sources\":true,\"one_method_state_at_a_time\":true,"
        << "\"construction_peak_note\":\"accounted retained bytes plus maximum extra encoded scratch; not measured RSS\","
        << "\"rss_note\":\"process lifetime high-water; use --only-method in fresh processes for method attribution\","
        << "\"checksum_note\":\"unsigned 64-bit modular sum of mixed value/row/feature terms; compressed payloads also checked per-value\"},"
        << "\n\"process_peak_rss_bytes\":" << rss() << ",\n\"workloads\":[";
    for (size_t wi = 0; wi < w.size(); ++wi) {
        if (wi) out << ',';
        uint64_t rows = 0, values = 0;
        for (const auto &q : w[wi].queries) {
            rows += q.count;
            for (size_t f = 0; f < features; ++f) if (q.mask & (1U << f)) values += q.count;
        }
        out << "{\"name\":" << quote(w[wi].name) << ",\"queries\":" << w[wi].queries.size()
            << ",\"row_visits\":" << rows << ",\"value_visits\":" << values
            << ",\"expected_checksum\":" << quote(hex(w[wi].expected)) << '}';
    }
    out << "],\n\"methods\":[";
    bool first = true;
    for (size_t kind = 0; kind < results.size(); ++kind) if (results[kind].present) {
        if (!first) out << ',';
        first = false;
        const Result &r = results[kind]; const Memory &m = r.memory;
        out << "\n{\"name\":" << quote(method_names[kind]) << ",\"memory\":{"
            << "\"source_logical_bytes\":" << m.source_logical << ",\"source_capacity_bytes\":" << m.source_capacity
            << ",\"derived_stored_bytes\":" << m.derived_logical << ",\"derived_capacity_bytes\":" << m.derived_capacity
            << ",\"index_capacity_bytes\":" << m.index_capacity << ",\"cache_capacity_bytes\":" << m.cache_capacity
            << ",\"scratch_capacity_bytes\":" << m.scratch_capacity << ",\"recipe_descriptor_bytes\":" << m.recipe_descriptor
            << ",\"other_object_metadata_bytes\":" << m.other_objects << ",\"common_query_schedule_bytes\":" << m.schedule
            << ",\"retained_accounted_bytes\":" << m.total() << ",\"construction_extra_scratch_bytes\":" << m.construction_extra
            << ",\"construction_accounted_peak_bytes\":" << m.total()+m.construction_extra << "},"
            << "\"construction_median_wall_ms\":" << med(r, -1) << ",\"construction_median_cpu_ms\":" << med(r, -1, true)
            << ",\"workloads\":[";
        for (size_t wi = 0; wi < w.size(); ++wi) {
            if (wi) out << ',';
            out << "{\"name\":" << quote(w[wi].name) << ",\"median_wall_ms\":" << med(r, static_cast<int>(wi))
                << ",\"median_cpu_ms\":" << med(r, static_cast<int>(wi), true)
                << ",\"checksum\":" << quote(hex(w[wi].expected)) << ",\"samples\":[";
            for (size_t ti = 0; ti < r.trials.size(); ++ti) {
                if (ti) out << ',';
                const auto &s = r.trials[ti].workloads[wi];
                out << "{\"wall_ms\":" << s.wall_ms << ",\"cpu_ms\":" << s.cpu_ms
                    << ",\"tile_hits\":" << s.hits << ",\"tile_misses\":" << s.misses << '}';
            }
            out << "]}";
        }
        out << "],\"construction_samples\":[";
        for (size_t ti = 0; ti < r.trials.size(); ++ti) {
            if (ti) out << ',';
            const auto &t = r.trials[ti];
            out << "{\"repetition\":" << t.repetition << ",\"order_position\":" << t.order_position
                << ",\"wall_ms\":" << t.construction.wall_ms << ",\"cpu_ms\":" << t.construction.cpu_ms << '}';
        }
        out << "]}";
    }
    out << "],\n\"amortization_vs_materialized\":[";
    first = true;
    if (results[0].present) for (size_t kind = 1; kind < results.size(); ++kind) if (results[kind].present)
        for (size_t wi = 0; wi < w.size(); ++wi) {
            if (!first) out << ',';
            first = false;
            const double build_delta = med(results[kind], -1) - med(results[0], -1);
            const double query_delta = med(results[kind], static_cast<int>(wi)) - med(results[0], static_cast<int>(wi));
            out << "{\"method\":" << quote(method_names[kind]) << ",\"workload\":" << quote(w[wi].name)
                << ",\"construction_delta_ms\":" << build_delta << ",\"per_workload_delta_ms\":" << query_delta
                << ",\"crossing_workload_executions\":";
            if (query_delta != 0 && -build_delta/query_delta >= 0) out << -build_delta/query_delta;
            else out << "null";
            out << ",\"interpretation\":" << quote(query_delta > 0 ? "After a positive crossing, materialized is faster in total wall time" :
                query_delta < 0 ? "After a positive crossing, this method is faster in total wall time" : "No measured per-workload slope difference") << '}';
        }
    out << "],\n\"amortization_note\":\"Illustrative construction + executions * median workload time; one execution means the entire named workload, not one scalar query; no confidence claim\"\n}\n";
}
}  // namespace

int main(int argc, char **argv) {
    try {
        const Options o = options(argc, argv);
        const auto x = read_i32(o.x), y = read_i32(o.y);
        if (x.size() != y.size()) throw std::runtime_error("Input row counts differ");
        auto w = workloads(x.size(), o);
        size_t schedule = sizeof(w);
        for (auto &item : w) { item.expected = oracle(item, x, y); schedule += item.queries.capacity() * sizeof(Query); }
        std::array<Result, 5> results;
        std::vector<size_t> order;
        for (size_t i = 0; i < method_names.size(); ++i)
            if (o.only_method.empty() || o.only_method == method_names[i]) order.push_back(i);
        std::mt19937_64 rng(o.seed ^ 0xd1b54a32d192ed03ULL);
        for (size_t repetition = 0; repetition < o.repetitions; ++repetition) {
            // Explicit Fisher-Yates makes the schedule independent of std::shuffle implementations.
            for (size_t i = order.size(); i > 1; --i) std::swap(order[i-1], order[static_cast<size_t>(rng() % i)]);
            for (size_t position = 0; position < order.size(); ++position) {
                const size_t kind = order[position];
                Trial t{}; t.repetition = repetition; t.order_position = position;
                const auto cpu = std::clock(); const auto start = Clock::now();
                Method method(kind, x, y, o);
                t.construction = duration(start, cpu);
                method.validate();
                for (size_t wi = 0; wi < w.size(); ++wi) {
                    method.reset();
                    const auto query_cpu = std::clock(); const auto query_start = Clock::now();
                    const uint64_t checksum = method.run(w[wi]);
                    t.workloads[wi] = duration(query_start, query_cpu);
                    t.workloads[wi].checksum = checksum;
                    t.workloads[wi].hits = method.hits; t.workloads[wi].misses = method.misses;
                    if (checksum != w[wi].expected) throw std::runtime_error("Query checksum differs from streamed source oracle");
                }
                results[kind].memory = method.memory(schedule);
                results[kind].trials.push_back(t); results[kind].present = true;
            }
        }
        if (o.output.empty()) write_json(std::cout, o, x, y, w, results);
        else {
            std::ofstream output(o.output);
            if (!output) throw std::runtime_error("Cannot create output");
            write_json(output, o, x, y, w, results);
            if (!output) throw std::runtime_error("Output write failed");
        }
        return 0;
    } catch (const std::exception &error) {
        std::cerr << "recipe-bench: " << error.what() << '\n'; return 1;
    }
}
