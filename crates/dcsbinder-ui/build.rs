fn main() {
    // Material 3 style for buttons / line edits / etc. Slint also supports
    // "fluent" (default), "cupertino", "qt" — material plays best with the
    // dark, dense layout DCSBinder uses.
    let config = slint_build::CompilerConfiguration::new().with_style("material".to_string());
    slint_build::compile_with_config("ui/main.slint", config).expect("slint compile");
}
