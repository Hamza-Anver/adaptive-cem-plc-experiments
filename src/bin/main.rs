use std::env;
use libafl_sandbox::common::{input_size};

// This macro automatically generates the `mod` declarations, the 
// CLI match statement, and the dynamic help menu.
macro_rules! fuzzer_registry {
    ($($name:ident),*) => {
        // 1. Automatically declare all the modules inside the fuzzers folder
        mod fuzzers {
            $(pub mod $name;)*
        }

        // 2. Automatically generate the routing function
        fn route_fuzzer(strategy: &str) {
            match strategy {
                // If they type the name, run the module's run() function
                $(stringify!($name) => fuzzers::$name::run(),)*
                
                // If they type something wrong, generate a dynamic help menu
                _ => {
                    println!("Unknown fuzzer strategy: '{}'", strategy);
                    println!("Available strategies:");
                    $(println!("  - {}", stringify!($name));)*
                }
            }
        }
    };
}

fuzzer_registry!(
    simple_stateless, 
    simple_stateful,
    pipeline_smoke,
    pipeline_greybox
);

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        println!("========================================");
        println!("LibAFL PLC Sandbox");
        println!("========================================");
        println!("Usage: cargo run --release -- <strategy>");
        println!();
        // Trigger the dynamic help menu by passing an empty string
        route_fuzzer(""); 
        return;
    }

    let strategy = &args[1];
    
    route_fuzzer(strategy);
}

#[cfg(test)]
mod tests {
    use super::*;
    use libafl_sandbox::common::{full_state, full_state_size, get_all_var_metadata, set_full_state};

    #[test]
    fn test_get_all_var_metadata() {
        let metadata = get_all_var_metadata();
        let count = metadata.len();
        println!("Retrieved {} variables from metadata", count);
        for var in metadata.iter() {
            let name = String::from_utf8_lossy(&var.name).trim_matches(char::from(0)).to_string();
            println!("Variable: {}, Type: {:#?}, Size: {}, Offset: {}",
                name, var.var_type, var.size, var.offset);
        }
    }

    #[test]
    fn test_input_and_state_contract() {
        let size = input_size();
        assert!(size > 0, "Expected a positive input size");

        let state_size = full_state_size();
        let state = full_state();

        if state_size == 0 {
            assert!(state.is_empty(), "Expected empty state when the target does not expose introspection");
            return;
        }

        assert_eq!(state_size, state.len(), "State helper should return the advertised size");
        assert!(set_full_state(&state), "Setting the captured state back should succeed");

        let round_trip = full_state();
        assert_eq!(state.len(), round_trip.len(), "Round-tripped state should keep the same size");
    }

    #[test]
    fn test_metadata_is_self_consistent() {
        let metadata = get_all_var_metadata();
        for var in metadata.iter() {
            let name = String::from_utf8_lossy(&var.name).trim_matches(char::from(0)).to_string();
            assert!(!name.is_empty(), "Metadata entries should have a readable name");
            assert!(var.size > 0, "Metadata entries should have a non-zero size");
        }
    }
}
