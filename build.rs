fn main() {
    // We use the `cc` crate to compile the C target.
    cc::Build::new()
        .file("mock_targets/mock_plc.c")
        // Force the use of Clang, as it has the best coverage sanitizers
        .compiler("/opt/homebrew/opt/llvm/bin/clang") 
        // Inject coverage tracking, but leave the execution engine to LibAFL
        .flag("-fsanitize-coverage=trace-pc-guard")
        .flag("-g")
        .flag("-O2")
        .compile("mock_plc"); // Outputs a static library named libmock_plc.a and links it

    // Tell Cargo to recompile the C code only if it actually changes
    println!("cargo:rerun-if-changed=mock_target/mock_plc.c");
}