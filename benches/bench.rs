use std::hint::black_box;
use iai_callgrind::{library_benchmark, library_benchmark_group, main};
use a_sabr::{
    bundle::Bundle, 
    contact_manager::segmentation::seg::SegmentationManager,
    contact_plan::from_tvgutil_file::TVGUtilContactPlan, 
    node_manager::none::NoManagement,
    routing::{aliases::*, Router},
    types::NodeID,
};

// --- 1. FONCTION DE SETUP (HORS MESURE) ---
// iai-callgrind appelle cette fonction AVANT de déclencher Callgrind.
// Les 18 millions d'instructions du parsing JSON s'arrêtent ici.
fn setup_router_env(router_type: &str) -> Box<dyn Router<NoManagement, SegmentationManager>> {
    let ptvg_filepath = "benches/ptvg_files/sample1.json";
    
    let spsn_opts = SpsnOptions {
        check_size: false,
        check_priority: false,
        max_entries: 10,
    };

    let contact_plan = TVGUtilContactPlan::parse::<NoManagement, SegmentationManager>(ptvg_filepath)
        .expect("Failed to parse contact plan");

    build_generic_router::<NoManagement, SegmentationManager>(
        router_type, 
        contact_plan, 
        Some(spsn_opts)
    ).expect("Failed to build router")
}

// --- 2. LE BENCHMARK (LA ZONE MESURÉE) ---
#[library_benchmark]
// Chaque ligne ci-dessous définit un cas de test indépendant.
// iai-callgrind : 1. Exécute setup() -> 2. Allume Callgrind -> 3. Exécute run_routing()
#[bench::spsn_hybrid(setup_router_env("SpsnHybridParenting"))]
#[bench::spsn_node(setup_router_env("SpsnNodeParenting"))]
#[cfg_attr(feature = "contact_work_area", bench::contact_parenting(setup_router_env("SpsnContactParenting")))]
#[cfg_attr(feature = "first_depleted", bench::depleted_hybrid(setup_router_env("CgrFirstDepletedHybridParenting")))]

fn run_routing(router: Box<dyn Router<NoManagement, SegmentationManager>>) {
    // Force le compilateur à considérer l'objet router comme "utilisé" et inconnu.
    let mut router = black_box(router);

    let source: NodeID = 178;
    let bundle = Bundle {
        source: 178,
        destinations: vec![159],
        priority: 0,
        size: 47419533.0,
        expiration: 24060.0,
    };
    let curr_time = 60.0;
    let excluded_nodes: Vec<NodeID> = vec![];

    // L'exécution de l'algorithme proprement dit.
    let result = router.route(
        black_box(source),
        black_box(&bundle),
        black_box(curr_time),
        black_box(&excluded_nodes),
    );

    // Empêche l'optimisation (élimination) de l'appel à .route()
    black_box(result);
}

// --- 3. GROUPE DE BENCHMARKS ---
library_benchmark_group!(
    name = routing_group;
    benchmarks = run_routing
);

// --- 4. POINT D'ENTRÉE (CONFIG PAR DÉFAUT) ---
// IMPORTANT : On ne met plus de valgrind_args(["--collect-atstart=yes"])
// car cela forcerait la mesure du setup (le parsing JSON).
main!(library_benchmark_groups = routing_group);
