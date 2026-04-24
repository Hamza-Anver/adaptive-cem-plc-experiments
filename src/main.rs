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