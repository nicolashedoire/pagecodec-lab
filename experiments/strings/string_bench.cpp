// Immutable byte-string storage experiment, not an OS memory compressor.
// Build: c++ -O3 -std=c++17 -Wall -Wextra -Werror string_bench.cpp -lz -o string_bench
#include <algorithm>
#include <array>
#include <chrono>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <iomanip>
#include <iostream>
#include <numeric>
#include <random>
#include <sstream>
#include <stdexcept>
#include <string>
#include <string_view>
#include <unordered_map>
#include <vector>
#include <zlib.h>

namespace {
using Clock = std::chrono::steady_clock;
constexpr uint64_t kFnvOffset = UINT64_C(14695981039346656037);
constexpr uint64_t kFnvPrime = UINT64_C(1099511628211);
constexpr size_t kAnchorBytes = 8;
const std::array<std::string, 4> kMethods = {"concat", "intern", "bounded_overlap", "zlib_blocks"};

struct Options {
    std::string input, output, only_method;
    size_t trials = 9, queries = 20000, block_bytes = 65536;
    size_t candidate_cap = 64, max_overlap = 256;
    uint64_t seed = 20261030;
};

uint64_t parse_uint(const std::string& text) {
    if (text.empty() || text.find_first_not_of("0123456789") != std::string::npos)
        throw std::runtime_error("Expected an unsigned decimal integer: " + text);
    return std::stoull(text);
}

Options options(int argc, char** argv) {
    Options out;
    for (int i = 1; i < argc; ++i) {
        const std::string arg = argv[i];
        if (arg == "--help") {
            std::cout << "string_bench --input FILE [--output FILE] [--trials 9] [--queries 20000]\n"
                         "  [--seed 20261030] [--block-bytes 65536] [--candidate-cap 64]\n"
                         "  [--max-overlap-bytes 256] [--only-method concat|intern|bounded_overlap|zlib_blocks]\n";
            std::exit(0);
        }
        if (++i >= argc) throw std::runtime_error("Missing value for " + arg);
        const std::string value = argv[i];
        if (arg == "--input") out.input = value;
        else if (arg == "--output") out.output = value;
        else if (arg == "--only-method") out.only_method = value;
        else if (arg == "--trials") out.trials = parse_uint(value);
        else if (arg == "--queries") out.queries = parse_uint(value);
        else if (arg == "--seed") out.seed = parse_uint(value);
        else if (arg == "--block-bytes") out.block_bytes = parse_uint(value);
        else if (arg == "--candidate-cap") out.candidate_cap = parse_uint(value);
        else if (arg == "--max-overlap-bytes") out.max_overlap = parse_uint(value);
        else throw std::runtime_error("Unknown argument: " + arg);
    }
    if (out.input.empty()) throw std::runtime_error("--input is required");
    if (!out.trials || out.trials > 1000 || !out.queries || !out.block_bytes ||
        out.block_bytes > UINT32_MAX || !out.candidate_cap || !out.max_overlap)
        throw std::runtime_error("Options must be positive; trials <= 1000 and block-bytes <= UINT32_MAX");
    if (!out.only_method.empty() && std::find(kMethods.begin(), kMethods.end(), out.only_method) == kMethods.end())
        throw std::runtime_error("Unknown --only-method");
    if (!out.output.empty() && out.input == out.output)
        throw std::runtime_error("Input and output paths must differ");
    return out;
}

// Both contracts cross the same non-inlined full-byte consumer. In particular,
// materialized bytes must be available at the destination before this call.
#if defined(__GNUC__) || defined(__clang__)
__attribute__((noinline))
#elif defined(_MSC_VER)
__declspec(noinline)
#endif
uint64_t consume(uint64_t state, std::string_view value) {
    for (unsigned char byte : value) state = (state ^ byte) * kFnvPrime;
    return (state ^ static_cast<uint64_t>(value.size())) * kFnvPrime;
}

std::string hex64(uint64_t value) {
    std::ostringstream out;
    out << std::hex << std::setw(16) << std::setfill('0') << value;
    return out.str();
}

struct Input {
    std::vector<std::string> objects;
    uint64_t file_bytes = 0, object_bytes = 0, file_hash = kFnvOffset;
    size_t longest = 0;
    uint64_t estimated_memory() const {
        // Omits allocation slack/headers; may count short-string characters twice.
        return objects.capacity() * sizeof(std::string) + object_bytes;
    }
};

Input read_input(const std::string& path) {
    std::ifstream file(path, std::ios::binary);
    if (!file) throw std::runtime_error("Cannot open input");
    Input input;
    std::string line;
    while (std::getline(file, line)) {
        if (line.size() > UINT32_MAX) throw std::runtime_error("An object exceeds the 32-bit block-offset format");
        const bool newline = !file.eof();
        input.file_bytes += line.size() + static_cast<size_t>(newline);
        input.object_bytes += line.size();
        input.longest = std::max(input.longest, line.size());
        for (unsigned char byte : line) input.file_hash = (input.file_hash ^ byte) * kFnvPrime;
        if (newline) input.file_hash = (input.file_hash ^ '\n') * kFnvPrime;
        input.objects.push_back(std::move(line));
    }
    if (file.bad()) throw std::runtime_error("Input read failed");
    return input;
}

struct Entry { uint64_t offset, length; };
struct Block { uint64_t offset, stored_bytes, raw_bytes, raw_fallback; };
static_assert(sizeof(Entry) == 16, "Entry accounting requires two packed 64-bit fields");
static_assert(sizeof(Block) == 32, "Block accounting requires four packed 64-bit fields");

struct Workspace {
    std::vector<char> decode, copy;
    uint64_t cached_block = UINT64_MAX; // Used only during untimed verification.
};

struct Store {
    std::string method;
    std::vector<char> payload;
    std::vector<Entry> index;
    std::vector<Block> blocks;
    size_t max_block = 0, max_object = 0;
    uint64_t construction_estimated_peak = 0;
    uint64_t unique_objects = 0, overlap_saved_bytes = 0, candidate_checks = 0;
    uint64_t capacity_bytes() const {
        return payload.capacity() + index.capacity() * sizeof(Entry) + blocks.capacity() * sizeof(Block);
    }
    uint64_t logical_bytes() const {
        return payload.size() + index.size() * sizeof(Entry) + blocks.size() * sizeof(Block);
    }
    Workspace workspace(bool materialize) const {
        Workspace work;
        if (method == "zlib_blocks") work.decode.resize(max_block);
        if (materialize) work.copy.resize(max_object);
        return work;
    }
    std::string_view view(size_t id, Workspace& work, bool validation_cache = false) const {
        const Entry entry = index.at(id);
        if (!entry.length) return {};
        if (method != "zlib_blocks") {
            if (entry.offset > payload.size() || entry.length > payload.size() - entry.offset)
                throw std::runtime_error("Invalid arena index");
            return {payload.data() + entry.offset, static_cast<size_t>(entry.length)};
        }
        const size_t block_id = static_cast<size_t>(entry.offset >> 32);
        const size_t within = static_cast<uint32_t>(entry.offset);
        const Block block = blocks.at(block_id);
        if (within > block.raw_bytes || entry.length > block.raw_bytes - within)
            throw std::runtime_error("Invalid block index");
        if (block.raw_fallback)
            return {payload.data() + block.offset + within, static_cast<size_t>(entry.length)};
        if (!validation_cache || work.cached_block != block_id) {
            uLongf decoded = work.decode.size();
            const int status = uncompress(reinterpret_cast<Bytef*>(work.decode.data()), &decoded,
                                          reinterpret_cast<const Bytef*>(payload.data() + block.offset),
                                          static_cast<uLong>(block.stored_bytes));
            if (status != Z_OK || decoded != block.raw_bytes) throw std::runtime_error("zlib decode failed");
            work.cached_block = block_id;
        }
        return {work.decode.data() + within, static_cast<size_t>(entry.length)};
    }
    void compact(uint64_t input_memory, uint64_t auxiliary) {
        construction_estimated_peak = std::max(construction_estimated_peak, input_memory + auxiliary + capacity_bytes() + payload.size());
        payload.shrink_to_fit();
        index.shrink_to_fit();
        blocks.shrink_to_fit();
    }
};

void append(std::vector<char>& out, std::string_view value) {
    if (!value.empty()) out.insert(out.end(), value.begin(), value.end());
}
uint64_t anchor(std::string_view value) {
    uint64_t key = 0;
    for (size_t i = 0; i < kAnchorBytes; ++i)
        key |= static_cast<uint64_t>(static_cast<unsigned char>(value[i])) << (8 * i);
    return key;
}

Store build(const std::string& method, const Input& input, const Options& opt) {
    Store store;
    store.method = method;
    store.max_object = input.longest;
    store.index.resize(input.objects.size());
    const uint64_t input_memory = input.estimated_memory();
    uint64_t auxiliary = 0;
    if (method == "concat") {
        store.payload.reserve(input.object_bytes);
        for (size_t i = 0; i < input.objects.size(); ++i) {
            const auto& object = input.objects[i];
            store.index[i] = {store.payload.size(), object.size()};
            append(store.payload, object);
        }
    } else if (method == "intern" || method == "bounded_overlap") {
        std::unordered_map<std::string_view, uint64_t> ids;
        ids.reserve(input.objects.size());
        std::vector<std::string_view> unique;
        unique.reserve(input.objects.size());
        for (size_t i = 0; i < input.objects.size(); ++i) {
            const std::string_view object = input.objects[i];
            const auto result = ids.emplace(object, unique.size());
            if (result.second) unique.push_back(object);
            store.index[i] = {result.first->second, object.size()};
        }
        store.unique_objects = unique.size();
        std::vector<uint64_t> offsets(unique.size());
        auxiliary = ids.bucket_count() * sizeof(void*) + ids.size() * (sizeof(std::pair<const std::string_view, uint64_t>) + 2 * sizeof(void*))
                    + unique.capacity() * sizeof(std::string_view) + offsets.capacity() * sizeof(uint64_t);
        if (method == "intern") {
            for (size_t i = 0; i < unique.size(); ++i) {
                offsets[i] = store.payload.size();
                append(store.payload, unique[i]);
            }
        } else {
            std::unordered_map<uint64_t, std::vector<size_t>> buckets;
            buckets.reserve(unique.size());
            for (size_t i = 0; i < unique.size(); ++i)
                if (unique[i].size() >= kAnchorBytes) buckets[anchor(unique[i])].push_back(i);
            std::vector<uint8_t> used(unique.size(), 0);
            auxiliary += used.capacity() + buckets.bucket_count() * sizeof(void*)
                         + buckets.size() * (sizeof(std::pair<const uint64_t, std::vector<size_t>>) + 2 * sizeof(void*));
            for (const auto& pair : buckets) auxiliary += pair.second.capacity() * sizeof(size_t);
            size_t fallback = 0;
            for (size_t done = 0; done < unique.size(); ++done) {
                size_t selected = unique.size(), best_overlap = 0, examined = 0;
                const size_t limit = std::min(opt.max_overlap, store.payload.size());
                for (size_t overlap = limit; overlap >= kAnchorBytes && examined < opt.candidate_cap; --overlap) {
                    const std::string_view tail(store.payload.data() + store.payload.size() - overlap, overlap);
                    const auto found = buckets.find(anchor(tail));
                    if (found == buckets.end()) continue;
                    auto& candidates = found->second;
                    for (size_t cursor = candidates.size(); cursor && examined < opt.candidate_cap;) {
                        --cursor;
                        ++examined;
                        const size_t id = candidates[cursor];
                        if (used[id]) {
                            candidates[cursor] = candidates.back();
                            candidates.pop_back();
                            continue;
                        }
                        if (unique[id].size() >= overlap && std::memcmp(unique[id].data(), tail.data(), overlap) == 0) {
                            selected = id;
                            best_overlap = overlap;
                            candidates[cursor] = candidates.back();
                            candidates.pop_back();
                            break;
                        }
                    }
                    if (selected != unique.size()) break;
                }
                store.candidate_checks += examined;
                if (selected == unique.size()) {
                    while (fallback < unique.size() && used[fallback]) ++fallback;
                    selected = fallback;
                }
                if (selected == unique.size()) throw std::runtime_error("Overlap selection failed");
                used[selected] = 1;
                offsets[selected] = store.payload.size() - best_overlap;
                append(store.payload, unique[selected].substr(best_overlap));
                store.overlap_saved_bytes += best_overlap;
            }
        }
        for (auto& entry : store.index) entry.offset = offsets.at(entry.offset);
        store.compact(input_memory, auxiliary);
    } else if (method == "zlib_blocks") {
        std::vector<char> raw, encoded;
        raw.reserve(std::max(opt.block_bytes, input.longest));
        auto flush = [&]() {
            if (raw.empty()) return;
            uLongf bytes = compressBound(static_cast<uLong>(raw.size()));
            encoded.resize(bytes);
            if (compress2(reinterpret_cast<Bytef*>(encoded.data()), &bytes,
                          reinterpret_cast<const Bytef*>(raw.data()), static_cast<uLong>(raw.size()), 1) != Z_OK)
                throw std::runtime_error("zlib compression failed");
            const bool fallback = bytes >= raw.size();
            const size_t stored = fallback ? raw.size() : static_cast<size_t>(bytes);
            store.blocks.push_back({store.payload.size(), stored, raw.size(), static_cast<uint64_t>(fallback)});
            append(store.payload, {fallback ? raw.data() : encoded.data(), stored});
            store.max_block = std::max(store.max_block, raw.size());
            auxiliary = raw.capacity() + encoded.capacity();
            store.construction_estimated_peak = std::max(store.construction_estimated_peak, input_memory + auxiliary + store.capacity_bytes());
            raw.clear();
        };
        for (size_t i = 0; i < input.objects.size(); ++i) {
            const auto& object = input.objects[i];
            if (object.empty()) { store.index[i] = {0, 0}; continue; }
            if (!raw.empty() && object.size() > opt.block_bytes - std::min(opt.block_bytes, raw.size())) flush();
            if (store.blocks.size() > UINT32_MAX || raw.size() > UINT32_MAX)
                throw std::runtime_error("Block index format exhausted");
            store.index[i] = {(static_cast<uint64_t>(store.blocks.size()) << 32) | raw.size(), object.size()};
            append(raw, object);
        }
        flush();
        store.compact(input_memory, auxiliary);
    } else throw std::runtime_error("Unknown method");
    store.compact(input_memory, 0);
    return store;
}

void verify(const Store& store, const Input& input) {
    Workspace work = store.workspace(false);
    for (size_t i = 0; i < input.objects.size(); ++i) {
        const auto actual = store.view(i, work, true);
        const auto& expected = input.objects[i];
        if (actual.size() != expected.size() || (!actual.empty() && std::memcmp(actual.data(), expected.data(), actual.size()) != 0))
            throw std::runtime_error("Byte-for-byte verification failed for " + store.method);
    }
}

struct QueryResult { std::vector<double> ns_per_query; uint64_t checksum = kFnvOffset; };
struct MethodResult {
    std::string method;
    uint64_t payload = 0, index = 0, blocks = 0, logical = 0, capacity = 0;
    uint64_t view_workspace = 0, copy_workspace = 0, estimated_peak = 0;
    uint64_t block_count = 0, raw_blocks = 0, unique_objects = 0, overlap_saved = 0, candidate_checks = 0;
    std::vector<double> construction_ms;
    std::array<QueryResult, 4> workloads;
};

std::string json_string(const std::string& value) {
    std::ostringstream out;
    out << '"';
    for (unsigned char c : value) {
        if (c == '"' || c == '\\') out << '\\' << c;
        else if (c < 32) out << "\\u" << std::hex << std::setw(4) << std::setfill('0') << unsigned(c) << std::dec;
        else out << c;
    }
    out << '"';
    return out.str();
}
template <class T> void json_array(std::ostream& out, const std::vector<T>& values) {
    out << '[';
    for (size_t i = 0; i < values.size(); ++i) { if (i) out << ','; out << values[i]; }
    out << ']';
}

uint64_t run_queries(const Store& store, const std::vector<size_t>& trace, Workspace& work, bool copy, size_t count) {
    uint64_t checksum = kFnvOffset;
    for (size_t i = 0; i < count; ++i) {
        auto value = store.view(trace[i], work);
        if (copy && !value.empty()) {
            std::memcpy(work.copy.data(), value.data(), value.size());
            value = {work.copy.data(), value.size()};
        }
        checksum = consume(checksum, value);
    }
    return checksum;
}

int run(const Options& opt) {
    Input initial = read_input(opt.input);
    const uint64_t file_bytes = initial.file_bytes, object_bytes = initial.object_bytes, input_hash = initial.file_hash;
    const size_t object_count = initial.objects.size(), longest = initial.longest;
    std::array<std::vector<size_t>, 2> traces;
    std::array<uint64_t, 2> expected = {kFnvOffset, kFnvOffset};
    std::mt19937_64 trace_rng(opt.seed);
    const size_t hot_count = object_count ? std::max<size_t>(1, object_count / 100) : 0;
    std::vector<size_t> hot_ids(object_count);
    std::iota(hot_ids.begin(), hot_ids.end(), 0);
    std::shuffle(hot_ids.begin(), hot_ids.end(), trace_rng);
    if (object_count) {
        for (size_t i = 0; i < opt.queries; ++i) {
            traces[0].push_back(trace_rng() % object_count);
            const bool hot = trace_rng() % 10 != 0;
            traces[1].push_back(hot ? hot_ids[trace_rng() % hot_count] : trace_rng() % object_count);
        }
        for (size_t t = 0; t < traces.size(); ++t)
            for (size_t id : traces[t]) expected[t] = consume(expected[t], initial.objects[id]);
    }
    std::vector<size_t>().swap(hot_ids);
    initial = Input{}; // Source strings are not retained during timed lookup.
    std::vector<MethodResult> results;
    for (const auto& method : kMethods) {
        if (!opt.only_method.empty() && method != opt.only_method) continue;
        MethodResult row; row.method = method; results.push_back(std::move(row));
    }
    std::vector<std::vector<std::string>> trial_orders, query_orders;
    std::mt19937_64 order_rng(opt.seed ^ UINT64_C(0x9e3779b97f4a7c15));
    uint64_t observable = 0;
    const std::array<std::string, 4> workload_names = {"uniform.view", "uniform.copy", "hot.view", "hot.copy"};
    for (size_t trial = 0; trial < opt.trials; ++trial) {
        std::vector<size_t> order(results.size());
        std::iota(order.begin(), order.end(), 0);
        std::shuffle(order.begin(), order.end(), order_rng);
        trial_orders.emplace_back();
        for (size_t method_id : order) {
            MethodResult& result = results[method_id];
            trial_orders.back().push_back(result.method);
            Input input = read_input(opt.input); // Parsing excluded from construction.
            if (input.file_hash != input_hash || input.objects.size() != object_count || input.file_bytes != file_bytes)
                throw std::runtime_error("Input changed while benchmarking");
            const auto begin = Clock::now();
            Store store = build(result.method, input, opt);
            const auto end = Clock::now();
            result.construction_ms.push_back(std::chrono::duration<double, std::milli>(end - begin).count());
            verify(store, input);
            input = Input{};
            if (trial && (result.logical != store.logical_bytes() || result.capacity != store.capacity_bytes()))
                throw std::runtime_error("Non-deterministic storage accounting");
            result.payload = store.payload.size(); result.index = store.index.size() * sizeof(Entry);
            result.blocks = store.blocks.size() * sizeof(Block);
            result.logical = store.logical_bytes(); result.capacity = store.capacity_bytes();
            result.estimated_peak = std::max(result.estimated_peak, store.construction_estimated_peak);
            result.block_count = store.blocks.size(); result.raw_blocks = 0;
            for (const auto& block : store.blocks) result.raw_blocks += block.raw_fallback;
            result.unique_objects = store.unique_objects; result.overlap_saved = store.overlap_saved_bytes;
            result.candidate_checks = store.candidate_checks;
            std::array<size_t, 4> query_order = {0, 1, 2, 3};
            std::shuffle(query_order.begin(), query_order.end(), order_rng);
            query_orders.emplace_back();
            query_orders.back().push_back(std::to_string(trial) + ":" + result.method);
            for (size_t workload : query_order) {
                query_orders.back().push_back(workload_names[workload]);
                const size_t trace_id = workload / 2;
                const bool copy = workload % 2;
                Workspace work = store.workspace(copy);
                const uint64_t workspace_bytes = work.decode.capacity() + work.copy.capacity();
                if (copy) result.copy_workspace = workspace_bytes; else result.view_workspace = workspace_bytes;
                observable ^= run_queries(store, traces[trace_id], work, copy, std::min<size_t>(256, traces[trace_id].size()));
                const auto start = Clock::now();
                const uint64_t checksum = run_queries(store, traces[trace_id], work, copy, traces[trace_id].size());
                const auto finish = Clock::now();
                if (checksum != expected[trace_id]) throw std::runtime_error("Timed query checksum mismatch");
                observable ^= checksum;
                const double elapsed = std::chrono::duration<double, std::nano>(finish - start).count();
                result.workloads[workload].ns_per_query.push_back(traces[trace_id].empty() ? 0 : elapsed / traces[trace_id].size());
                result.workloads[workload].checksum = checksum;
            }
        }
    }
    std::ostringstream report;
    report << std::setprecision(12);
    const auto last_slash = opt.input.find_last_of("/\\");
    report << "{\n\"schema_version\":1,\n\"experiment\":\"immutable_strings\",\n\"input_name\":"
           << json_string(opt.input.substr(last_slash == std::string::npos ? 0 : last_slash + 1))
           << ",\n\"input_bytes\":" << file_bytes << ",\n\"input_fnv1a64\":" << json_string(hex64(input_hash))
           << ",\n\"object_count\":" << object_count << ",\n\"total_object_bytes\":" << object_bytes
           << ",\n\"max_object_bytes\":" << longest << ",\n\"trials\":" << opt.trials
           << ",\n\"queries_per_workload\":" << traces[0].size() << ",\n\"seed\":" << opt.seed
           << ",\n\"block_bytes\":" << opt.block_bytes << ",\n\"candidate_cap\":" << opt.candidate_cap
           << ",\n\"max_overlap_bytes\":" << opt.max_overlap << ",\n\"minimum_overlap_bytes\":8"
           << ",\n\"hot_set_objects\":" << hot_count << ",\n\"hot_probability\":0.9"
           << ",\n\"zlib_version\":" << json_string(zlibVersion()) << ",\n\"zlib_level\":1"
           << ",\n\"benchmark_trace_capacity_bytes\":" << (traces[0].capacity() + traces[1].capacity()) * sizeof(size_t)
           << ",\n\"source_oracle_retained_during_lookup\":false"
           << ",\n\"timing_scope\":\"Batched object lookup plus full returned-byte FNV consumption; copy also copies every byte. Caller workspace allocation and verification excluded; zlib internal allocation/deallocation remains timed. Construction includes final compaction.\""
           << ",\n\"storage_scope\":\"Dynamic vector buffer capacities only, plus caller lookup scratch where labeled. Container/control objects, allocator headers and transient internal zlib allocations are excluded; these counters are not process RSS.\""
           << ",\n\"view_contract\":\"Borrowed view valid until next lookup on that workspace; zlib decodes each compressed-block access with no persistent decoded cache.\""
           << ",\n\"construction_estimate_note\":\"Estimated capacity accounting, not RSS or an upper bound; includes input strings and approximate hash-table nodes, omits allocator headers and zlib internals.\""
           << ",\n\"trial_order\":[";
    auto string_rows = [&](const std::vector<std::vector<std::string>>& rows) {
        for (size_t i = 0; i < rows.size(); ++i) {
            if (i) report << ',';
            report << '[';
            for (size_t j = 0; j < rows[i].size(); ++j) { if (j) report << ','; report << json_string(rows[i][j]); }
            report << ']';
        }
    };
    string_rows(trial_orders);
    report << "],\n\"workload_order\":["; string_rows(query_orders);
    report << "],\n\"observable_checksum\":" << json_string(hex64(observable)) << ",\n\"methods\":[\n";
    for (size_t i = 0; i < results.size(); ++i) {
        if (i) report << ",\n";
        const auto& row = results[i];
        report << "{\"method\":" << json_string(row.method) << ",\"verified_all_objects\":true"
               << ",\"payload_bytes\":" << row.payload << ",\"index_bytes\":" << row.index
               << ",\"block_metadata_bytes\":" << row.blocks << ",\"logical_storage_bytes\":" << row.logical
               << ",\"capacity_storage_bytes\":" << row.capacity << ",\"lookup_workspace_view_bytes\":" << row.view_workspace
               << ",\"lookup_workspace_copy_bytes\":" << row.copy_workspace
               << ",\"resident_view_capacity_bytes\":" << row.capacity + row.view_workspace
               << ",\"resident_copy_capacity_bytes\":" << row.capacity + row.copy_workspace
               << ",\"construction_estimated_peak_bytes\":" << row.estimated_peak
               << ",\"block_count\":" << row.block_count << ",\"raw_fallback_blocks\":" << row.raw_blocks
               << ",\"unique_objects\":";
        if (row.method == "intern" || row.method == "bounded_overlap") report << row.unique_objects; else report << "null";
        report << ",\"overlap_saved_bytes\":" << row.overlap_saved << ",\"overlap_candidate_checks\":" << row.candidate_checks
               << ",\"construction_samples_ms\":";
        json_array(report, row.construction_ms);
        report << ",\"workloads\":{";
        for (size_t t = 0; t < 2; ++t) {
            if (t) report << ',';
            report << json_string(t ? "hot" : "uniform") << ":{";
            for (size_t c = 0; c < 2; ++c) {
                if (c) report << ',';
                const auto& workload = row.workloads[t * 2 + c];
                report << json_string(c ? "copy" : "view") << ":{\"checksum_fnv1a64\":" << json_string(hex64(workload.checksum))
                       << ",\"ns_per_query_samples\":";
                json_array(report, workload.ns_per_query);
                report << '}';
            }
            report << '}';
        }
        report << "}}";
    }
    report << "\n]}\n";
    if (opt.output.empty()) std::cout << report.str();
    else {
        std::ofstream output(opt.output, std::ios::binary);
        if (!output || !(output << report.str())) throw std::runtime_error("Output write failed");
    }
    return 0;
}
} // namespace

int main(int argc, char** argv) {
    try { return run(options(argc, argv)); }
    catch (const std::exception& error) { std::cerr << "string_bench: " << error.what() << '\n'; return 1; }
}
