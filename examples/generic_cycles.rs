//! Example: Using generic cycle detection with custom error chains
//!
//! This demonstrates how to use the higher-order cycle detection
//! with a custom error type that has a `caused_by` relationship.

use ascii_dag::cycles::generic::{detect_cycle_fn, CycleDetectable};

// Custom error type with causal relationships
#[derive(Debug, Clone)]
struct Error {
    id: usize,
    message: String,
    caused_by: Vec<usize>, // IDs of errors that caused this one
}

impl Error {
    fn new(id: usize, message: &str, caused_by: Vec<usize>) -> Self {
        Self {
            id,
            message: message.to_string(),
            caused_by,
        }
    }
}

// Implement the trait for automatic cycle detection
impl CycleDetectable for Error {
    type Id = usize;

    fn id(&self) -> Self::Id {
        self.id
    }

    fn dependencies(&self) -> Vec<Self::Id> {
        self.caused_by.clone()
    }
}

fn main() {
    println!("=== Generic Cycle Detection Examples ===\n");

    // Example 1: Using closure-based detection (most flexible)
    example_closure_based();

    // Example 2: Using trait-based detection (most ergonomic)
    example_trait_based();

    // Example 3: Complex error chain
    example_complex_chain();
}

fn example_closure_based() {
    println!("1. Closure-based cycle detection:");
    println!("   (Works with any data structure)\n");

    // Define error relationships using a closure
    let get_caused_by = |error_id: &usize| -> Vec<usize> {
        match error_id {
            1 => vec![2],      // ParseError caused by FileReadError
            2 => vec![3],      // FileReadError caused by PermissionError
            3 => vec![],       // PermissionError is root cause
            _ => vec![],
        }
    };

    let all_error_ids = vec![1, 2, 3];
    let cycle = detect_cycle_fn(&all_error_ids, get_caused_by);

    if let Some(path) = cycle {
        println!("   ❌ Cycle detected: {:?}", path);
    } else {
        println!("   ✓ No cycle - valid error chain");
    }

    // Example with a cycle
    let get_cyclic_deps = |id: &usize| -> Vec<usize> {
        match id {
            1 => vec![2],
            2 => vec![3],
            3 => vec![1], // Cycle!
            _ => vec![],
        }
    };

    let cycle = detect_cycle_fn(&all_error_ids, get_cyclic_deps);
    if let Some(path) = cycle {
        println!("   ❌ Cycle detected: {:?}", path);
    }

    println!();
}

fn example_trait_based() {
    println!("2. Trait-based cycle detection:");
    println!("   (Most ergonomic for structured data)\n");

    let errors = vec![
        Error::new(1, "DatabaseConnectionFailed", vec![2, 3]),
        Error::new(2, "NetworkTimeout", vec![4]),
        Error::new(3, "ConfigMissing", vec![]),
        Error::new(4, "DNSResolutionFailed", vec![]),
    ];

    // Using the trait
    let cycle = ascii_dag::cycles::generic::detect_cycle(&errors);

    if let Some(path) = cycle {
        println!("   ❌ Cycle in error chain: {:?}", path);
    } else {
        println!("   ✓ No cycle - valid error chain");
        println!("   Error dependency tree:");
        for error in &errors {
            if error.caused_by.is_empty() {
                println!("   - {} (root cause)", error.message);
            } else {
                println!("   - {} ← caused by {:?}", error.message, error.caused_by);
            }
        }
    }

    println!();
}

fn example_complex_chain() {
    println!("3. Complex error chain with cycle detection:\n");

    // Simulate a retry scenario that could create cycles
    let errors = vec![
        Error::new(1, "RequestFailed", vec![2]),
        Error::new(2, "RetryExceeded", vec![3]),
        Error::new(3, "ServiceUnavailable", vec![4]),
        Error::new(4, "HealthCheckFailed", vec![5]),
        Error::new(5, "ConnectionRefused", vec![]),
    ];

    println!("   Checking error chain for circular dependencies...");
    
    let cycle = ascii_dag::cycles::generic::detect_cycle(&errors);
    
    if let Some(path) = cycle {
        println!("   ❌ CRITICAL: Circular error dependency detected!");
        println!("   Cycle path: {:?}", path);
        println!("   This could cause infinite retry loops!");
    } else {
        println!("   ✓ Error chain is valid (no cycles)");
        println!("   Safe to implement retry logic");
    }

    println!("\n=== Real-world use case ===");
    println!("This is useful for:");
    println!("  • Error handling chains (error.caused_by)");
    println!("  • Task dependency graphs (task.depends_on)");
    println!("  • Configuration validation (config.imports)");
    println!("  • Module dependency analysis (module.requires)");
    println!("  • Any directed graph relationship!");
}
