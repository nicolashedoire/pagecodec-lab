/* Local, single-threaded state adapters. These are NOT XNU ABI layouts.
 * The Apple decision functions themselves are included verbatim below.
 * No live kernel state, actual runqueues, CPU affinity, IPI or locks are used.
 */
#include <assert.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdatomic.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#ifdef __options_decl
#undef __options_decl
#endif
#define __options_decl(name, type, ...) typedef type name; enum __VA_ARGS__
#include "generated/apple_types.inc"

#define MAX_PSETS 4
#define TH_BUCKET_SCHED_MAX 6
#define MAX(a, b) ((a) > (b) ? (a) : (b))
#define os_atomic_load(p, order) atomic_load_explicit((p), memory_order_##order)
typedef int boolean_t;
typedef int sched_bucket_t;
typedef enum { CLUSTER_TYPE_P = 1, CLUSTER_TYPE_E = 2 } cluster_type_t;

struct processor_set {
    uint32_t pset_cluster_id;
    cluster_type_t pset_type;
    unsigned available_cpus;
    unsigned fixture_die;
    _Atomic uint64_t pset_load_average[TH_BUCKET_SCHED_MAX];
    struct { _Atomic uint64_t pset_avg_thread_execution_time; }
        pset_execution_time[TH_BUCKET_SCHED_MAX];
    uint64_t pset_cluster_shared_rsrc_load[CLUSTER_SHARED_RSRC_TYPE_COUNT];
    sched_clutch_edge sched_edges[MAX_PSETS];
    uint64_t native_psets[1], foreign_psets[1], local_psets[1], remote_psets[1];
};
typedef struct processor_set *processor_set_t;
struct thread {
    sched_bucket_t th_sched_bucket;
    bool fixture_shared_policy[CLUSTER_SHARED_RSRC_TYPE_COUNT];
};
typedef struct thread *thread_t;
static processor_set_t pset_array[MAX_PSETS];
static uint32_t sched_edge_max_clusters;

static int pset_available_cpu_count(processor_set_t p) { return (int)p->available_cpus; }
static cluster_type_t pset_type_for_id(uint32_t id) { return pset_array[id]->pset_type; }
static uint64_t sched_pset_cluster_shared_rsrc_load(processor_set_t p, cluster_shared_rsrc_type_t type) {
    return p->pset_cluster_shared_rsrc_load[type];
}
static bool thread_shared_rsrc_policy_get(thread_t t, cluster_shared_rsrc_type_t type) {
    return t->fixture_shared_policy[type];
}
static int bit_first(uint64_t bits) { return bits ? __builtin_ctzll(bits) : -1; }
static int bit_next(uint64_t bits, int previous) {
    return previous >= 63 ? -1 : bit_first(bits & (~UINT64_C(0) << (previous + 1)));
}
static bool bit_test(uint64_t bits, int index) { return (bits & (UINT64_C(1) << index)) != 0; }
static void bitmap_set(uint64_t *bits, int index) { *bits |= UINT64_C(1) << index; }
static void bitmap_clear(uint64_t *bits, int index) { *bits &= ~(UINT64_C(1) << index); }
typedef uint64_t bitmap_t;

/* Source initialization expects these operations. No lock/concurrency claim. */
typedef int spl_t;
static spl_t splsched(void) { return 0; }
static void splx(spl_t s) { (void)s; }
static void pset_lock(processor_set_t p) { (void)p; }
static void pset_unlock(processor_set_t p) { (void)p; }
static unsigned ml_get_die_id(int cluster) { return pset_array[cluster]->fixture_die; }

#include "generated/apple_metric.inc"
#include "generated/apple_edge.inc"
