use std::hint::black_box;
use iai_callgrind::{library_benchmark, library_benchmark_group, main};
use a_sabr::{
    bundle::Bundle, contact_manager::segmentation::seg::SegmentationManager,
    contact_plan::from_tvgutil_file::TVGUtilContactPlan, node_manager::none::NoManagement,
    routing::{aliases::*, Router},
    types::NodeID,
};

// --- 1. FONCTION DE SETUP (HORS MESURE) ---
// Cette fonction est appelée par iai-callgrind AVANT de lancer le compteur.
fn setup(router_type: &str) -> Box<dyn Router<NoManagement, SegmentationManager>> {
    let ptvg_filepath = "benches/ptvg_files/sample1.json";
    let spsn_opts = SpsnOptions {
        check_size: false,
        check_priority: false,
        max_entries: 10,
    };

    let contact_plan = TVGUtilContactPlan::parse::<NoManagement, SegmentationManager>(ptvg_filepath)
        .expect("Failed to parse contact plan");

    build_generic_router::<NoManagement, SegmentationManager>(router_type, contact_plan, Some(spsn_opts))
        .expect("Failed to build router")
}

// --- 2. LE BENCHMARK UNIQUE ---
// On définit UN benchmark qui accepte le router déjà prêt en argument.
#[library_benchmark]
// On crée une variante pour chaque type de router. 
// iai-callgrind isolera parfaitement l'exécution de .route() pour chaque cas.
#[bench::spsn_hybrid(setup("SpsnHybridParenting"))]
#[bench::spsn_node(setup("SpsnNodeParenting"))]
#[cfg_attr(feature = "contact_work_area", bench::contact_parenting(setup("SpsnContactParenting")))]
#[cfg_attr(feature = "first_depleted", bench::depleted_hybrid(setup("CgrFirstDepletedHybridParenting")))]
fn run_routing(mut router: Box<dyn Router<NoManagement, SegmentationManager>>) {
    let source = 178;
    let bundle = Bundle {
        source: 178,
        destinations: vec![159],
        priority: 0,
        size: 47419533.0,
        expiration: 24060.0,
    };
    let curr_time = 60.0;
    let excluded_nodes: Vec<NodeID> = vec![];

    // Seul cet appel sera mesuré.
    let _ = black_box(router.route(
        black_box(source),
        black_box(&bundle),
        black_box(curr_time),
        black_box(&excluded_nodes),
    ));
}

// --- 3. CONFIGURATION ---
library_benchmark_group!(
    name = routing_group;
    benchmarks = run_routing
);

// Plus besoin de valgrind_args ou de toggles complexes !
// La lib sait qu'elle doit mesurer ce qui est dans "run_routing".
main!(library_benchmark_groups = routing_group);
