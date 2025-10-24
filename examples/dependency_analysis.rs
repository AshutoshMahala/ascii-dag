use ascii_dag::cycles::generic::roots::{RootFindable, find_leaves_fn, find_roots_fn};
use ascii_dag::graph::DAG;
use ascii_dag::layout::generic::impact::{
    ImpactAnalyzable, compute_blast_radius_fn, compute_descendants_fn,
};
use ascii_dag::layout::generic::metrics::GraphMetrics;
use std::collections::HashMap;

fn main() {
    println!("=== Dependency Analysis Tool ===\n");

    example_simple_analysis();
    example_trait_based_registry();
    example_metrics_dashboard();
}

fn example_simple_analysis() {
    println!("1. Simple Dependency Analysis");
    println!("   Analyzing a package dependency graph\n");

    // Define package dependencies
    let get_deps = |pkg: &&str| match *pkg {
        "app" => vec!["core", "ui"],
        "ui" => vec!["core", "renderer"],
        "renderer" => vec!["core"],
        "core" => vec!["utils"],
        "utils" => vec![],
        _ => vec![],
    };

    let packages = ["app", "ui", "renderer", "core", "utils"];

    // Find root packages (no dependencies)
    let roots = find_roots_fn(&packages, get_deps);
    println!("   📦 Root packages (can be built first):");
    for root in &roots {
        println!("      - {}", root);
    }
    println!();

    // Find leaf packages (nothing depends on them)
    let leaves = find_leaves_fn(&packages, get_deps);
    println!("   🍃 Leaf packages (final outputs):");
    for leaf in &leaves {
        println!("      - {}", leaf);
    }
    println!();

    // Impact analysis: What breaks if core changes?
    let impacted = compute_descendants_fn(&packages, &"core", get_deps);
    println!("   ⚠️  If 'core' changes, these packages need rebuilding:");
    for pkg in &impacted {
        println!("      - {}", pkg);
    }
    println!("   Impact: {} packages affected\n", impacted.len());

    // Blast radius: Full dependency tree for ui
    let (deps, impacts) = compute_blast_radius_fn(&packages, &"ui", get_deps);
    println!("   💥 Blast radius for 'ui':");
    println!("      Dependencies: {:?}", deps);
    println!("      Impacts: {:?}", impacts);
    println!();
}

fn example_trait_based_registry() {
    println!("2. Trait-Based Error Registry");
    println!("   Using traits for cleaner API\n");

    #[derive(Debug, Clone)]
    struct ErrorDef {
        id: String,
        message: String,
        caused_by: Vec<String>,
    }

    struct ErrorRegistry {
        errors: HashMap<String, ErrorDef>,
    }

    impl RootFindable for ErrorRegistry {
        type Id = String;

        fn get_all_ids(&self) -> Vec<String> {
            self.errors.keys().cloned().collect()
        }

        fn get_dependencies(&self, id: &String) -> Vec<String> {
            self.errors
                .get(id)
                .map(|e| e.caused_by.clone())
                .unwrap_or_default()
        }
    }

    impl ImpactAnalyzable for ErrorRegistry {
        type Id = String;

        fn get_all_ids(&self) -> Vec<String> {
            self.errors.keys().cloned().collect()
        }

        fn get_dependencies(&self, id: &String) -> Vec<String> {
            self.errors
                .get(id)
                .map(|e| e.caused_by.clone())
                .unwrap_or_default()
        }
    }

    // Create error registry
    let mut errors = HashMap::new();
    errors.insert(
        "E001".to_string(),
        ErrorDef {
            id: "E001".to_string(),
            message: "Invalid configuration".to_string(),
            caused_by: vec![],
        },
    );
    errors.insert(
        "E002".to_string(),
        ErrorDef {
            id: "E002".to_string(),
            message: "Database connection failed".to_string(),
            caused_by: vec!["E001".to_string()],
        },
    );
    errors.insert(
        "E003".to_string(),
        ErrorDef {
            id: "E003".to_string(),
            message: "Transaction rollback".to_string(),
            caused_by: vec!["E002".to_string()],
        },
    );

    let registry = ErrorRegistry { errors };

    // Use trait methods
    println!("   🔍 Root errors (primary causes):");
    for root in registry.find_roots() {
        println!("      - {}", root);
    }
    println!();

    println!("   📊 Error E001 analysis:");
    let impacted = registry.compute_descendants(&"E001".to_string());
    println!("      Cascading errors: {}", impacted.len());
    for err in &impacted {
        println!("      - {}", err);
    }
    println!();
}

fn example_metrics_dashboard() {
    println!("3. Dependency Graph Metrics");
    println!("   Statistical analysis of the dependency structure\n");

    // Build system example
    let get_deps = |file: &&str| match *file {
        "app.exe" => vec!["main.o", "utils.o", "io.o"],
        "main.o" => vec!["main.c", "types.h"],
        "utils.o" => vec!["utils.c", "types.h"],
        "io.o" => vec!["io.c", "types.h"],
        "main.c" => vec![],
        "utils.c" => vec![],
        "io.c" => vec![],
        "types.h" => vec![],
        _ => vec![],
    };

    let files = [
        "app.exe", "main.o", "utils.o", "io.o", "main.c", "utils.c", "io.c", "types.h",
    ];

    let metrics = GraphMetrics::compute(&files, get_deps);

    println!("   📈 Graph Statistics:");
    println!("      Total files: {}", metrics.node_count());
    println!("      Total dependencies: {}", metrics.edge_count());
    println!("      Root files (sources): {}", metrics.root_count());
    println!("      Leaf files (outputs): {}", metrics.leaf_count());
    println!("      Max depth: {}", metrics.max_depth());
    println!("      Max impact: {} files", metrics.max_descendants());
    println!("      Avg dependencies: {:.2}", metrics.avg_dependencies());
    println!("      Density: {:.2}%", metrics.density() * 100.0);
    println!();

    println!("   🔍 Graph Properties:");
    println!("      Is tree: {}", metrics.is_tree());
    println!("      Is forest: {}", metrics.is_forest());
    println!("      Is sparse: {}", metrics.is_sparse());
    println!("      Is dense: {}", metrics.is_dense());
    println!();

    // Visualize with DAG
    println!("   📊 Visualization:");
    let mut dag = DAG::new();
    dag.add_node(1, "types.h");
    dag.add_node(2, "main.c");
    dag.add_node(3, "main.o");
    dag.add_node(4, "app.exe");
    dag.add_edge(2, 3);
    dag.add_edge(1, 3);
    dag.add_edge(3, 4);

    println!("{}", dag.render());

    // Find the most impactful file
    let mut max_impact = 0;
    let mut most_impactful = "";
    for file in &files {
        let impact = compute_descendants_fn(&files, file, get_deps).len();
        if impact > max_impact {
            max_impact = impact;
            most_impactful = file;
        }
    }

    println!(
        "   ⚡ Most impactful file: '{}' (affects {} other files)",
        most_impactful, max_impact
    );
    println!();
}
