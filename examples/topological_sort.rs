use ascii_dag::layout::generic::{topological_sort_fn, TopologicallySortable};
use std::collections::HashMap;

fn main() {
    println!("=== Generic Topological Sorting Examples ===\n");

    // Example 1: Build system tasks
    example_build_tasks();

    // Example 2: Package dependencies
    example_packages();

    // Example 3: Trait-based API
    example_trait_based();

    // Example 4: Cycle detection
    example_cycle_detection();
}

fn example_build_tasks() {
    println!("1. Build System Tasks:");
    println!("   Dependencies: deploy→test→build→compile\n");

    let get_deps = |task: &&str| match *task {
        "deploy" => vec!["test", "build"],
        "test" => vec!["build"],
        "build" => vec!["compile"],
        "compile" => vec![],
        _ => vec![],
    };

    let tasks = ["deploy", "test", "build", "compile"];
    
    match topological_sort_fn(&tasks, get_deps) {
        Ok(sorted) => {
            println!("   Execution order:");
            for (i, task) in sorted.iter().enumerate() {
                println!("   {}. {}", i + 1, task);
            }
        }
        Err(cycle) => println!("   ERROR: Cycle detected: {:?}", cycle),
    }
    println!();
}

fn example_packages() {
    println!("2. Package Dependencies:");
    println!("   app depends on lib-a and lib-b");
    println!("   both libs depend on lib-core\n");

    let get_deps = |pkg: &&str| match *pkg {
        "app" => vec!["lib-a", "lib-b"],
        "lib-a" => vec!["lib-core"],
        "lib-b" => vec!["lib-core"],
        "lib-core" => vec![],
        _ => vec![],
    };

    let packages = ["app", "lib-b", "lib-a", "lib-core"]; // Unsorted

    match topological_sort_fn(&packages, get_deps) {
        Ok(sorted) => {
            println!("   Build order:");
            for (i, pkg) in sorted.iter().enumerate() {
                println!("   {}. {}", i + 1, pkg);
            }
        }
        Err(cycle) => println!("   ERROR: Cycle detected: {:?}", cycle),
    }
    println!();
}

fn example_trait_based() {
    println!("3. Trait-Based API (Course Prerequisites):");

    struct CourseSchedule {
        courses: Vec<String>,
        prerequisites: HashMap<String, Vec<String>>,
    }

    impl TopologicallySortable for CourseSchedule {
        type Id = String;

        fn get_all_ids(&self) -> Vec<String> {
            self.courses.clone()
        }

        fn get_dependencies(&self, id: &String) -> Vec<String> {
            self.prerequisites.get(id).cloned().unwrap_or_default()
        }
    }

    let mut prereqs = HashMap::new();
    prereqs.insert("CS301".to_string(), vec!["CS201".to_string()]);
    prereqs.insert("CS201".to_string(), vec!["CS101".to_string()]);
    prereqs.insert("CS101".to_string(), vec![]);

    let schedule = CourseSchedule {
        courses: vec![
            "CS301".to_string(),
            "CS101".to_string(), 
            "CS201".to_string(),
        ],
        prerequisites: prereqs,
    };

    println!("   CS301 requires CS201, which requires CS101\n");

    match schedule.topological_sort() {
        Ok(sorted) => {
            println!("   Semester order:");
            for (i, course) in sorted.iter().enumerate() {
                println!("   Semester {}: {}", i + 1, course);
            }
        }
        Err(cycle) => println!("   ERROR: Circular prerequisites: {:?}", cycle),
    }
    println!();
}

fn example_cycle_detection() {
    println!("4. Cycle Detection:");
    println!("   Task A depends on B, B on C, C on A (circular!)\n");

    let get_deps = |task: &&str| match *task {
        "A" => vec!["B"],
        "B" => vec!["C"],
        "C" => vec!["A"], // Creates cycle!
        _ => vec![],
    };

    let tasks = ["A", "B", "C"];

    match topological_sort_fn(&tasks, get_deps) {
        Ok(_) => println!("   ✓ No cycles detected"),
        Err(cycle) => {
            println!("   ✗ Cycle detected!");
            println!("   Circular dependency: {:?}", cycle);
        }
    }
    println!();
}
