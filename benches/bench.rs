use iai_callgrind::{
    library_benchmark, library_benchmark_group, main, 
    client_requests::callgrind
};
// On utilise le black_box standard de Rust (disponible depuis Rust 1.66)
use std::hint::black_box;
use a_sabr::{
    bundle::Bundle, contact_manager::segmentation::seg::SegmentationManager,
    contact_plan::from_tvgutil_file::TVGUtilContactPlan, node_manager::none::NoManagement,
    routing::{aliases::*, Router}, // On importe le Trait Router ici
    types::NodeID,
};

// --- HELPER : Setup du Router ---
// Utilisation des types concrets NoManagement et SegmentationManager
fn setup_router(router_type: &str) -> Box<dyn Router<NoManagement, SegmentationManager>> {
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

// --- MACRO : Générateur de Bench ---
macro_rules! bench_router {
    ($fn_name:ident, $type_str:expr) => {
        #[library_benchmark]
        fn $fn_name() {
            // Setup hors mesure
            let mut router = setup_router($type_str);
            let source = 178;
            let bundle = Bundle {
                source: 178, destinations: vec![159], priority: 0,
                size: 47419533.0, expiration: 24060.0,
            };
            let curr_time = 60.0;
            let excluded_nodes: Vec<NodeID> = vec![];

            // Instrumentation Valgrind
            callgrind::start_instrumentation();
            
            let _ = black_box(router.route(
                black_box(source),
                black_box(&bundle),
                black_box(curr_time),
                black_box(&excluded_nodes),
            ));

            callgrind::stop_instrumentation();
        }
    };
}

// --- DÉFINITION DES BENCHMARKS ---
bench_router!(bench_spsn_hybrid, "SpsnHybridParenting");
bench_router!(bench_spsn_node, "SpsnNodeParenting");

#[cfg(feature = "contact_work_area")]
bench_router!(bench_contact_parenting, "SpsnContactParenting");
#[cfg(not(feature = "contact_work_area"))]
#[library_benchmark] fn bench_contact_parenting() {}

#[cfg(feature = "first_depleted")]
bench_router!(bench_depleted_hybrid, "CgrFirstDepletedHybridParenting");
#[cfg(not(feature = "first_depleted"))]
#[library_benchmark] fn bench_depleted_hybrid() {}

// --- CONFIGURATION ET GROUPE ---
library_benchmark_group!(
    name = routing_group;
    benchmarks = 
        bench_spsn_hybrid, 
        bench_spsn_node,
        bench_contact_parenting,
        bench_depleted_hybrid
);

main!(
    config = iai_callgrind::LibraryBenchmarkConfig::default()
        .valgrind_args([ // <--- C'était ça le changement
            "toggle-collect=*callgrind*start_instrumentation*"
        ]);
    library_benchmark_groups = routing_group
);
