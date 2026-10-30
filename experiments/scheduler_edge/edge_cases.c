/* Functional state fixtures, not a scheduler performance benchmark. */
#include "edge_environment.h"

static struct processor_set clusters[MAX_PSETS];
static struct thread candidate;
static unsigned checks;

#define CHECK(condition) do { \
    ++checks; \
    if (!(condition)) { fprintf(stderr, "check failed at %s:%d: %s\n", __FILE__, __LINE__, #condition); exit(1); } \
} while (0)

static void set_load(unsigned cluster, unsigned bucket, uint64_t load, uint64_t runtime_us) {
    atomic_store(&clusters[cluster].pset_load_average[bucket], load << 8);
    atomic_store(&clusters[cluster].pset_execution_time[bucket].pset_avg_thread_execution_time, runtime_us);
}

static void reset_fixture(void) {
    memset(clusters, 0, sizeof(clusters));
    memset(&candidate, 0, sizeof(candidate));
    /* Synthetic three-cluster topology, not a claim about the host's live psets. */
    sched_edge_max_clusters = 3;
    for (unsigned i = 0; i < MAX_PSETS; ++i) {
        clusters[i].pset_cluster_id = i;
        clusters[i].pset_type = i == 2 ? CLUSTER_TYPE_E : CLUSTER_TYPE_P;
        clusters[i].available_cpus = 4;
        clusters[i].fixture_die = 0;
        pset_array[i] = &clusters[i];
        for (unsigned b = 0; b < TH_BUCKET_SCHED_MAX; ++b) {
            atomic_init(&clusters[i].pset_load_average[b], UINT64_C(1) << 8);
            atomic_init(&clusters[i].pset_execution_time[b].pset_avg_thread_execution_time, 100);
        }
    }
    sched_edge_cpu_init_completed();
}

static unsigned choose(unsigned preferred) {
    processor_set_t p = &clusters[preferred];
    cluster_shared_rsrc_type_t resource = sched_edge_thread_shared_rsrc_type(&candidate);
    uint32_t load = resource == CLUSTER_SHARED_RSRC_TYPE_NONE
        ? sched_edge_cluster_load_metric(p, candidate.th_sched_bucket)
        : (uint32_t)sched_pset_cluster_shared_rsrc_load(p, resource);
    /* The complete upstream caller does additional guards before this function.
     * These fixtures deliberately enter only its nonzero-load migration branch. */
    assert(load != 0 && load != UINT32_MAX);
    return sched_edge_migrate_edges_evaluate(p, load, &candidate)->pset_cluster_id;
}

static void normal_policy_cases(void) {
    reset_fixture();
    CHECK(clusters[0].sched_edges[1].sce_migration_allowed == 1);
    CHECK(clusters[0].sched_edges[1].sce_migration_weight == 0);
    CHECK(clusters[0].sched_edges[2].sce_migration_weight == 64);
    CHECK(clusters[2].sched_edges[0].sce_migration_allowed == 0);
    CHECK(clusters[2].sched_edges[0].sce_steal_allowed == 0);

    set_load(1, 0, 0, 100); set_load(2, 0, 0, 100);
    CHECK(choose(0) == 1); /* Idle homogeneous peer precedes idle E cluster. */

    reset_fixture();
    set_load(2, 0, 1, 37);
    CHECK(choose(0) == 0); /* 100 - 37 = 63 us: below P->E threshold. */
    set_load(2, 0, 1, 36);
    CHECK(choose(0) == 2); /* Equality at 64 us admits migration. */
    set_load(0, 0, 1, 1); set_load(2, 0, 0, 100);
    CHECK(choose(0) == 2); /* Idle candidate bypasses the non-idle edge weight. */

    reset_fixture();
    set_load(0, 0, 0, 100); set_load(1, 0, 0, 100);
    CHECK(choose(2) == 2); /* E->P forbidden by source initialization. */
    clusters[2].sched_edges[0].sce_migration_allowed = 1;
    CHECK(choose(2) == 0); /* An explicit configuration change changes the result. */

    reset_fixture();
    set_load(2, 0, 0, 100); clusters[2].available_cpus = 0;
    CHECK(sched_edge_cluster_load_metric(&clusters[2], 0) == UINT32_MAX);
    CHECK(choose(0) == 0);

    reset_fixture();
    set_load(1, 0, 1, 20); set_load(2, 0, 1, 20);
    CHECK(choose(0) == 1); /* Equal advantage keeps the homogeneous candidate. */
    set_load(2, 0, 1, 10);
    CHECK(choose(0) == 2); /* Strictly larger qualifying advantage can select E. */

    reset_fixture();
    clusters[0].sched_edges[2].sce_migration_allowed = 0;
    set_load(2, 0, 0, 100);
    CHECK(choose(0) == 0);
    processor_set_t saved = pset_array[1]; pset_array[1] = NULL;
    CHECK(choose(0) == 0); /* Missing candidate is skipped. */
    pset_array[1] = saved;
}

static void resource_and_metric_cases(void) {
    reset_fixture();
    candidate.fixture_shared_policy[CLUSTER_SHARED_RSRC_TYPE_RR] = true;
    candidate.fixture_shared_policy[CLUSTER_SHARED_RSRC_TYPE_NATIVE_FIRST] = true;
    CHECK(sched_edge_thread_shared_rsrc_type(&candidate) == CLUSTER_SHARED_RSRC_TYPE_RR);
    clusters[0].pset_cluster_shared_rsrc_load[0] = 2;
    clusters[1].pset_cluster_shared_rsrc_load[0] = 2;
    clusters[2].pset_cluster_shared_rsrc_load[0] = 1;
    CHECK(choose(0) == 2); /* Shared-resource decision bypasses ordinary 64us weight. */

    reset_fixture();
    candidate.fixture_shared_policy[CLUSTER_SHARED_RSRC_TYPE_NATIVE_FIRST] = true;
    for (unsigned i = 0; i < 3; ++i) clusters[i].pset_cluster_shared_rsrc_load[1] = i == 2 ? 0 : 2;
    CHECK(choose(0) == 0); /* Preferred native cluster still has capacity. */
    clusters[0].pset_cluster_shared_rsrc_load[1] = 4;
    clusters[1].pset_cluster_shared_rsrc_load[1] = 3;
    CHECK(choose(0) == 1); /* Spare native capacity precedes an idle E cluster. */
    clusters[1].pset_cluster_shared_rsrc_load[1] = 4;
    CHECK(choose(0) == 2); /* Both native clusters full; now E is eligible. */

    reset_fixture();
    atomic_store(&clusters[0].pset_load_average[0], 127);
    CHECK(sched_get_pset_load_average(&clusters[0], 0) == 0);
    atomic_store(&clusters[0].pset_load_average[0], 128);
    CHECK(sched_get_pset_load_average(&clusters[0], 0) == 100);
    atomic_store(&clusters[0].pset_execution_time[0].pset_avg_thread_execution_time, 0);
    CHECK(sched_get_pset_load_average(&clusters[0], 0) == 1);
    CHECK(sched_edge_pset_available(NULL) == false);

    reset_fixture();
    clusters[0].local_psets[0] = UINT64_C(1) << 2;
    clusters[0].remote_psets[0] = UINT64_C(1) << 1;
    uint64_t candidates = (UINT64_C(1) << 1) | (UINT64_C(1) << 2);
    CHECK(sched_edge_iterate_clusters_ordered(&clusters[0], candidates, -1) == 2);
    CHECK(sched_edge_iterate_clusters_ordered(&clusters[0], candidates, 2) == 1);
    CHECK(sched_edge_iterate_clusters_ordered(&clusters[0], candidates, 1) == -1);
}

static void all_bucket_threshold_cases(void) {
    const unsigned loads[] = {1, 2, 63, 64, 65, 100, 128};
    for (unsigned bucket = 0; bucket < TH_BUCKET_SCHED_MAX; ++bucket) {
        for (unsigned a = 0; a < sizeof(loads)/sizeof(loads[0]); ++a) {
            for (unsigned b = 0; b < sizeof(loads)/sizeof(loads[0]); ++b) {
                reset_fixture(); candidate.th_sched_bucket = (int)bucket;
                set_load(0, bucket, 1, loads[a]);
                set_load(1, bucket, 1, loads[a]);
                set_load(2, bucket, 1, loads[b]);
                unsigned expected = loads[a] >= loads[b] && loads[a] - loads[b] >= 64 ? 2 : 0;
                CHECK(choose(0) == expected);
            }
        }
    }
}

int main(void) {
    normal_policy_cases();
    resource_and_metric_cases();
    all_bucket_threshold_cases();
    printf("{\"functional_checks\":%u,\"failures\":0,\"performance_measurement\":false,"
           "\"kernel_integration\":false,\"policy\":\"verbatim Apple Edge subset\"}\n", checks);
    return 0;
}
