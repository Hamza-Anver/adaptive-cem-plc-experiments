use std::env;
mod common;

// This macro automatically generates the `mod` declarations, the 
// CLI match statement, and the dynamic help menu.
macro_rules! fuzzer_registry {
    ($($name:ident),*) => {
        // 1. Automatically declare all the modules inside the fuzzers folder
        pub mod fuzzers {
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



use crate::common::{
    get_all_var_metadata, get_key_var_metadata, get_var_vec_to_hashmap
};
// Check that the common rs functions are valid
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_all_var_metadata() {
        let metadata = get_all_var_metadata();
        assert!(!metadata.is_empty(), "Expected to retrieve some variable metadata");
        println!("Retrieved {} variables from metadata", metadata.len());   
        for var in metadata.iter() {
            let name = String::from_utf8_lossy(&var.name).trim_matches(char::from(0)).to_string();
            println!("Variable: {}, Type: {:#?}, Size: {}, Offset: {}, Is Key: {}", 
                name, var.var_type, var.size, var.offset, var.is_key);
        }
    }

    #[test]
    fn test_get_key_var_metadata() {
        let key_metadata = get_key_var_metadata();
        assert!(!key_metadata.is_empty(), "Expected to retrieve some key variable metadata");
        println!("Retrieved {} key variables from metadata", key_metadata.len());   
        for var in key_metadata.iter() {
            let name = String::from_utf8_lossy(&var.name).trim_matches(char::from(0)).to_string();
            println!("Key Variable: {}, Type: {:#?}, Size: {}, Offset: {}", 
                name, var.var_type, var.size, var.offset);
        }
    }

    #[test]
    fn test_get_var_vec_to_hashmap() {
        let metadata = get_all_var_metadata();
        let state_map = get_var_vec_to_hashmap(metadata);
        assert!(!state_map.is_empty(), "Expected to retrieve some variable states");
        println!("Retrieved state for {} variables", state_map.len());
        for (name, value) in state_map.iter() {
            // FIX: is this printing the right thing
            println!("Variable: {}, Value: {:#?}", name, value.to_ascii_lowercase());
        }
    }
}