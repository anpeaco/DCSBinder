fn main() {
    // Fluent-light keeps Slint's std-widgets flat with subtle borders, which
    // pairs well with the GitHub-Light diff palette we set in main.slint.
    // Material was too round / elevated and fought our explicit colors.
    let config = slint_build::CompilerConfiguration::new().with_style("fluent-light".to_string());
    slint_build::compile_with_config("ui/main.slint", config).expect("slint compile");
}
