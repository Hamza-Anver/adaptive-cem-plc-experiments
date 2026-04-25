use std::env;

fn main() {
    // 1. Read the environment variable, default to "boiler_plc" if none is provided.
    let target = env::var("PLC_TARGET").unwrap_or_else(|_| "pipeline_deep_oob".to_string());
    let target_file = format!("mock_targets/{}.c", target);

    // 2. Compile the Harness + The Dynamic Target
    cc::Build::new()
        .file("mock_targets/harness.c") 
        .file(&target_file)             // Inject the chosen file here
        .compiler("/opt/homebrew/opt/llvm/bin/clang") 
        .flag("-fsanitize-coverage=trace-pc-guard")
        .flag("-O2")
        .flag("-g") 
        .compile("mock_plc");

    // 3. Cache Invalidation Rules
    println!("cargo:rerun-if-changed=mock_targets");
    println!("cargo:rerun-if-env-changed=PLC_TARGET"); // MUST HAVE THIS
}