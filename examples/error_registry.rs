//! Example: Error handling with cycle-safe causal chains
//!
//! Demonstrates a real-world error handling system that prevents
//! circular error dependencies using the generic cycle detector.

use ascii_dag::cycles::generic::{detect_cycle_fn, has_cycle_fn};
use std::collections::HashMap;

/// A chainable error type with causal relationships
#[derive(Debug, Clone)]
pub struct ChainableError {
    pub code: &'static str,
    pub message: String,
    pub caused_by: Vec<&'static str>, // Error codes this depends on
}

impl ChainableError {
    fn new(code: &'static str, message: &str) -> Self {
        Self {
            code,
            message: message.to_string(),
            caused_by: Vec::new(),
        }
    }

    fn with_cause(mut self, cause: &'static str) -> Self {
        self.caused_by.push(cause);
        self
    }

    fn with_causes(mut self, causes: Vec<&'static str>) -> Self {
        self.caused_by.extend(causes);
        self
    }
}

/// Error registry that validates no circular dependencies
pub struct ErrorRegistry {
    errors: HashMap<&'static str, ChainableError>,
}

impl ErrorRegistry {
    pub fn new() -> Self {
        Self {
            errors: HashMap::new(),
        }
    }

    /// Register an error, checking for cycles
    pub fn register(&mut self, error: ChainableError) -> Result<(), String> {
        let code = error.code;
        
        // Temporarily add the error to check for cycles
        self.errors.insert(code, error);

        // Check for cycles
        if self.has_cycle() {
            // Remove the error that caused the cycle
            self.errors.remove(code);
            
            // Find the cycle path for error message
            let cycle = self.find_cycle();
            if let Some(path) = cycle {
                return Err(format!(
                    "Cannot register '{}': would create circular dependency: {:?}",
                    code, path
                ));
            }
            return Err(format!("Cannot register '{}': would create a cycle", code));
        }

        Ok(())
    }

    fn has_cycle(&self) -> bool {
        let all_codes: Vec<&'static str> = self.errors.keys().copied().collect();
        let errors_clone: HashMap<_, _> = self.errors.clone();
        
        let get_dependencies = move |err_code: &&'static str| -> Vec<&'static str> {
            errors_clone
                .get(err_code)
                .map(|e| e.caused_by.clone())
                .unwrap_or_default()
        };

        has_cycle_fn(&all_codes, get_dependencies)
    }

    fn find_cycle(&self) -> Option<Vec<&'static str>> {
        let all_codes: Vec<&'static str> = self.errors.keys().copied().collect();
        let errors_clone: HashMap<_, _> = self.errors.clone();
        
        let get_dependencies = move |err_code: &&'static str| -> Vec<&'static str> {
            errors_clone
                .get(err_code)
                .map(|e| e.caused_by.clone())
                .unwrap_or_default()
        };

        detect_cycle_fn(&all_codes, get_dependencies)
    }

    pub fn get(&self, code: &str) -> Option<&ChainableError> {
        self.errors.get(code)
    }

    /// Get the full error chain (root causes)
    pub fn get_chain(&self, code: &str) -> Vec<&ChainableError> {
        let mut chain = Vec::new();
        let mut visited = std::collections::HashSet::new();
        self.collect_chain(code, &mut chain, &mut visited);
        chain
    }

    fn collect_chain<'a>(
        &'a self,
        code: &str,
        chain: &mut Vec<&'a ChainableError>,
        visited: &mut std::collections::HashSet<&'static str>,
    ) {
        if let Some(error) = self.errors.get(code) {
            if visited.contains(&error.code) {
                return; // Prevent infinite loops (shouldn't happen due to validation)
            }
            visited.insert(error.code);
            
            for &cause_code in &error.caused_by {
                self.collect_chain(cause_code, chain, visited);
            }
            
            chain.push(error);
        }
    }
}

fn main() {
    println!("=== Error Registry with Cycle Detection ===\n");

    let mut registry = ErrorRegistry::new();

    // Register valid error chain
    println!("1. Registering valid error chain...\n");

    registry
        .register(ChainableError::new("ERR_PERMISSION", "Permission denied"))
        .unwrap();

    registry
        .register(
            ChainableError::new("ERR_FILE_READ", "Failed to read file")
                .with_cause("ERR_PERMISSION"),
        )
        .unwrap();

    registry
        .register(
            ChainableError::new("ERR_PARSE", "Failed to parse configuration")
                .with_cause("ERR_FILE_READ"),
        )
        .unwrap();

    println!("   ✓ Registered error chain:");
    println!("     ERR_PARSE → ERR_FILE_READ → ERR_PERMISSION\n");

    // Try to register an error that would create a cycle
    println!("2. Attempting to register circular dependency...\n");

    // Current chain: ERR_PARSE → ERR_FILE_READ → ERR_PERMISSION
    
    // Register ERR_RETRY that depends on ERR_PARSE
    registry
        .register(ChainableError::new("ERR_RETRY", "Retry failed").with_cause("ERR_PARSE"))
        .unwrap();
    
    println!("   ✓ Added ERR_RETRY → ERR_PARSE");
    println!("   Chain is now: ERR_RETRY → ERR_PARSE → ERR_FILE_READ → ERR_PERMISSION\n");

    // Now try to make ERR_PERMISSION depend on ERR_RETRY - this creates a cycle!
    // ERR_RETRY → ERR_PARSE → ERR_FILE_READ → ERR_PERMISSION → ERR_RETRY (CYCLE!)
    
    let cyclic_error = ChainableError::new("ERR_PERMISSION", "Permission error with retry")
        .with_cause("ERR_RETRY"); // This would create: ... → ERR_PERMISSION → ERR_RETRY → ERR_PARSE → ...

    match registry.register(cyclic_error) {
        Ok(_) => println!("   ❌ Should have detected cycle!"),
        Err(msg) => println!("   ✓ Cycle detected and prevented:\n     {}\n", msg),
    }

    // Show the valid error chain
    println!("3. Querying error chain:\n");
    
    if let Some(chain) = registry.errors.get("ERR_PARSE") {
        println!("   Error: {}", chain.message);
        let full_chain = registry.get_chain("ERR_PARSE");
        
        println!("   Causal chain:");
        for (i, error) in full_chain.iter().enumerate() {
            println!("     {}. [{}] {}", i + 1, error.code, error.message);
        }
    }

    println!("\n=== Benefits ===");
    println!("  ✓ Prevents infinite error loops at registration time");
    println!("  ✓ Validates error dependencies before they cause runtime issues");
    println!("  ✓ Works with any relationship (caused_by, depends_on, etc.)");
    println!("  ✓ Generic approach works with any data structure!");
}
