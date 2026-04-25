use std::env;

fn main() {
    // 1. Read the target environment variable, default to pipeline_deep_oob
    let target = env::var("PLC_TARGET").unwrap_or_else(|_| "pipeline_deep_oob".to_string());
    let target_file = format!("mock_targets/{}.c", target);

    // 2. Get the compiler, with sensible defaults per platform
    let compiler = env::var("PLC_COMPILER").unwrap_or_else(|_| {
        if cfg!(target_os = "macos") {
            "/opt/homebrew/opt/llvm/bin/clang".to_string()
        } else if cfg!(target_os = "linux") {
            "clang".to_string()
        } else {
            "cc".to_string() // Let cc crate auto-detect on other platforms
        }
    });

    // 3. Compile the Harness + The Dynamic Target
    // Note: sancov instrumentation disabled for cdylib compatibility
    // TODO: Re-enable with proper runtime linking for full coverage support
    let mut builder = cc::Build::new();
    if !compiler.is_empty() {
        builder.compiler(&compiler);
    }
    builder
        .file("mock_targets/harness.c")
        .file(&target_file)
        .flag("-O2")
        .flag("-g")
        .compile("mock_plc");

    // 4. Cache Invalidation Rules
    println!("cargo:rerun-if-changed=mock_targets");
    println!("cargo:rerun-if-env-changed=PLC_TARGET");
    println!("cargo:rerun-if-env-changed=PLC_COMPILER");
}